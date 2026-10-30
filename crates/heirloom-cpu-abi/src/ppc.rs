//! The PowerPC face of the CPU contract (`ARCH_PPC`, cpu-ppc.library): CPU ABI v3, a FLAT face of its own (the design ruling is
//! docs/heirloom/23-ENGINE-SPLIT.md's CPU ABI v3 note in coralos). [`crate::CpuVTable`] (v2, per-access bus callbacks) is untouched;
//! a PowerPC guest runs ~200-300 MIPS at tier 1 on the host and a callback per load/store would cost most of that, and tier 1 needs
//! the code watch, which an opaque bus cannot give.
//!
//! The rules the records below encode:
//! - **RAM is the ENGINE's.** [`FlatRam`] and its [`CodeWatch`] (defined by the memory layer, heirloom-mem-abi) describe the engine's big-endian guest RAM (heirloom-mem-flat32's
//!   `GuestMem`). The engine is the only allocator: it sizes the watch to cover every page of RAM before publishing it, and grows
//!   both only between runs or inside a host callback, publishing the new pointers in the same records (and bumping `epoch`). The
//!   library borrows them: it may flip watch bits and bump generations in place, never reallocate, and it re-reads the whole
//!   descriptor at run entry and after every callback.
//! - **Registers are the ENGINE's.** [`PpcContext`] is a plain value the engine owns (one per guest thread, Mixed Mode frame,
//!   callback CPU); `run` executes ON it in place and publishes it complete at every exit and before every callback. The library
//!   keeps no register-derived state between calls.
//! - **A native hit ends the run's current block** and calls [`PpcHostCallbacks::on_native`]; its outcome code says how the guest
//!   continues. An unknown code is a host-protocol fault ([`EXIT_HOST_PROTOCOL`]), not a guess.
//! - **Why a run stopped** is a [`PpcExit`] record with ONE codec ([`PpcExit::encode`] / [`PpcExit::decode`]): a [`Trap`] names its
//!   instruction family by a closed id into [`WHAT`], never by a string pointer across the boundary.
//! - [`Trap`] lives HERE (heirloom-cpu-ppc re-exports it) so the engine side, which links no core, has the
//!   same type (ONE-PARSER-PER-FORMAT).
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_void;
use core::fmt;

/// [`PpcVTable::version`]: the first PowerPC flat face.
pub const PPC_ABI_VERSION: u32 = 1;
/// [`PpcVTable::abi_family`]: 'PPCF', so a v2 [`crate::CpuVTable`] (whose fourth word is a function pointer) is never taken for one.
pub const ABI_FAMILY_PPC_FLAT: u32 = u32::from_be_bytes(*b"PPCF");
/// `PpcCreate::sub_arch`: the 601/603/G3 user instruction set (the only one).
pub const SUB_ARCH_G3: u32 = 3;

/// One PowerPC register file (the architectural state a program sees). FPRs are raw IEEE-754 bit patterns.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpcRegs {
    pub gpr: [u32; 32],
    pub fpr: [u64; 32],
    pub cr: u32,
    pub lr: u32,
    pub ctr: u32,
    pub xer: u32,
    pub fpscr: u32,
    pub pc: u32,
    /// Time base: one tick per retired instruction.
    pub tb: u64,
}

/// The lwarx/stwcx. reservation (part of the context: a thread switch carries it).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpcReservation {
    /// 1 = held at `address`.
    pub valid: u32,
    pub address: u32,
}

/// [`PpcContext::state`].
pub const CTX_RUNNABLE: u32 = 0;
/// `halt` was asked (by a native's [`HOST_HALT`]): the guest is over; `run` returns at once.
pub const CTX_HALTED: u32 = 1;

/// One guest thread's complete execution context, owned by the engine (copy it to save a thread; nothing else is needed).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PpcContext {
    /// `size_of::<PpcContext>()`: a context of another layout is refused by `run`.
    pub size: u32,
    pub flags: u32,
    pub regs: PpcRegs,
    pub reservation: PpcReservation,
    pub state: u32,
    pub reserved: u32,
}

impl PpcContext {
    /// A reset context that will execute at `entry` with r1 = `sp`, everything else zero.
    pub fn new(entry: u32, sp: u32) -> PpcContext {
        let mut regs = PpcRegs { pc: entry, ..PpcRegs::default() };
        regs.gpr[1] = sp;
        PpcContext { size: core::mem::size_of::<PpcContext>() as u32, flags: 0, regs, reservation: PpcReservation::default(), state: CTX_RUNNABLE, reserved: 0 }
    }
}

/// The engine's guest RAM and its code watch: the memory layer's records (heirloom-mem-abi), which this face borrows.
pub use heirloom_mem_abi::{CodeWatch, FlatRam, CODE_PAGE_SHIFT};

/// [`PpcHostCallbacks::on_native`] / `on_syscall` outcomes.
/// The native ran: the guest returns through LR (as if it were a `blr`); a syscall continues after the `sc`.
pub const HOST_HANDLED: i32 = 0;
/// The native set `regs.pc` itself: the guest continues there.
pub const HOST_REDIRECTED: i32 = 1;
/// The guest is over (`run` returns [`EXIT_HALT`]).
pub const HOST_HALT: i32 = 2;
/// The guest must wait: `run` returns [`EXIT_BLOCK`] with `pc` still ON the native, so the next run calls it again.
pub const HOST_BLOCK: i32 = 3;
/// The host declined: a [`Trap::NativeUnhandled`] / [`Trap::Syscall`].
pub const HOST_UNHANDLED: i32 = 4;

/// The engine's side of a run. `ctx` is passed back unchanged; both receive the run's context, already published complete.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PpcHostCallbacks {
    /// `size_of::<PpcHostCallbacks>()`.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Execution reached `addr`, bound with `bind_native`. `regs.pc` = `addr`.
    pub on_native: extern "C" fn(ctx: *mut c_void, cpu: *mut PpcContext, addr: u32) -> i32,
    /// An `sc` at `pc` (`regs.pc` = `pc`).
    pub on_syscall: extern "C" fn(ctx: *mut c_void, cpu: *mut PpcContext, pc: u32) -> i32,
}

/// What `create` needs: the RAM and the host. Both records must outlive the instance (the library keeps the pointers).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PpcCreate {
    /// `size_of::<PpcCreate>()`.
    pub size: u32,
    /// [`SUB_ARCH_G3`].
    pub sub_arch: u32,
    pub ram: *const FlatRam,
    pub host: *const PpcHostCallbacks,
}

/// [`PpcExit::code`].
pub const EXIT_BUDGET: u32 = 0;
pub const EXIT_HALT: u32 = 1;
/// A native answered [`HOST_BLOCK`]: resumable, `pc` on the native.
pub const EXIT_BLOCK: u32 = 2;
/// A [`Trap`]: [`PpcExit::decode`] gives it back.
pub const EXIT_TRAP: u32 = 3;
/// A callback answered a code that is none of the HOST_* ones (`word` = the code): the run stopped before another instruction,
/// `pc` on the native; the callback's own side effects stand.
pub const EXIT_HOST_PROTOCOL: u32 = 4;
/// The run was refused before any instruction (a context, RAM descriptor or instance that fails validation; `word` names which:
/// [`REFUSE_CONTEXT`], [`REFUSE_RAM`], [`REFUSE_INSTANCE`]).
pub const EXIT_REFUSED: u32 = 5;

pub const REFUSE_CONTEXT: u32 = 1;
pub const REFUSE_RAM: u32 = 2;
pub const REFUSE_INSTANCE: u32 = 3;

/// [`Trap`] kinds in [`PpcExit::detail`] (low 8 bits; bits 16.. = the [`WHAT`] index for the kinds that carry one).
const T_UNIMPLEMENTED: u32 = 0;
const T_INVALID_FORM: u32 = 1;
const T_PRIVILEGED: u32 = 2;
const T_BUS_FAULT: u32 = 3;
const T_ALIGNMENT: u32 = 4;
const T_SYSCALL: u32 = 5;
const T_TRAP_WORD: u32 = 6;
const T_NATIVE_UNHANDLED: u32 = 7;
const T_REFUSED: u32 = 8;

/// Why a run stopped, as words; [`PpcExit::encode`] / [`PpcExit::decode`] are the one codec.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpcExit {
    pub size: u32,
    pub code: u32,
    pub pc: u32,
    /// Trap: the instruction word (Unimplemented / InvalidForm / Privileged / TrapWord). HostProtocol: the bad code. Refused: which.
    pub word: u32,
    /// Trap: the faulting address (BusFault / Alignment) or the native address (NativeUnhandled).
    pub addr: u32,
    /// Trap: the kind (low 8 bits) and the [`WHAT`] index (bits 16..).
    pub detail: u32,
    /// BusFault: 1 = a store.
    pub write: u32,
    pub reserved: u32,
    /// Instructions retired by this run (a native counts as one).
    pub retired: u64,
}

/// Why the interpreter stopped (other than a halt, a block or the budget). Every variant leaves `pc` AT the faulting instruction, so
/// a host can inspect, patch and resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// An instruction this interpreter does not execute. `what` names the family (never "unknown").
    Unimplemented { pc: u32, word: u32, what: &'static str },
    /// A defined opcode used in a form the architecture marks invalid (e.g. `lwzu` with RA=RT).
    InvalidForm { pc: u32, word: u32, why: &'static str },
    /// A supervisor-only instruction or SPR in user mode.
    Privileged { pc: u32, word: u32, what: &'static str },
    /// A load, store or instruction fetch outside guest RAM.
    BusFault { pc: u32, addr: u32, write: bool },
    /// An access the architecture requires to be aligned (lwarx, stwcx., lmw, stmw) was not.
    Alignment { pc: u32, addr: u32, what: &'static str },
    /// `sc` and the host did not handle it.
    Syscall { pc: u32 },
    /// A `tw`/`twi` whose condition held.
    TrapWord { pc: u32, word: u32 },
    /// Execution reached a bound native address and the host declined it.
    NativeUnhandled { addr: u32 },
    /// The CPU ran NOTHING: no table bound, RAM with no flat descriptor, an instance the library would not create, a run it
    /// refused ([`EXIT_REFUSED`]) or a host callback that broke the protocol ([`EXIT_HOST_PROTOCOL`]).
    Refused { why: &'static str },
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Trap::Unimplemented { pc, word, what } => write!(f, "unimplemented {what} ({word:#010x}) at {pc:#010x}"),
            Trap::InvalidForm { pc, word, why } => write!(f, "invalid form: {why} ({word:#010x}) at {pc:#010x}"),
            Trap::Privileged { pc, word, what } => write!(f, "privileged {what} ({word:#010x}) at {pc:#010x}"),
            Trap::BusFault { pc, addr, write } => {
                write!(f, "bus fault: {} {addr:#010x} at {pc:#010x}", if write { "store to" } else { "load from" })
            }
            Trap::Alignment { pc, addr, what } => write!(f, "alignment: {what} at {addr:#010x} (pc {pc:#010x})"),
            Trap::Syscall { pc } => write!(f, "unhandled sc at {pc:#010x}"),
            Trap::TrapWord { pc, word } => write!(f, "trap word taken ({word:#010x}) at {pc:#010x}"),
            Trap::NativeUnhandled { addr } => write!(f, "native glue at {addr:#010x} not handled by the host"),
            Trap::Refused { why } => write!(f, "the PowerPC CPU ran nothing: {why}"),
        }
    }
}
impl core::error::Error for Trap {}

/// Index 0: a reason string the table does not list (the encoder's named fallback; a test keeps the table complete).
pub const WHAT_UNLISTED: &str = "an instruction family heirloom-cpu-abi::ppc::WHAT does not list";

/// The closed vocabulary of a [`Trap`]'s `what` / `why` across the boundary (index = the id in [`PpcExit::detail`]). APPEND ONLY:
/// an id never changes meaning. heirloom-cpu-ppc's `what_table_lists_every_reason_the_core_names` test fails when the core
/// names a reason missing here.
pub const WHAT: &[&str] = &[
    WHAT_UNLISTED,
    "64-bit PowerPC / reserved opcode",
    "AltiVec load/store (lvx/stvx family)",
    "AltiVec / paired-single (opcode 4)",
    "bcctr with BO decrementing CTR",
    "dcbi",
    "D-form integer",
    "eciwx/ecowx (external control, not in user subset)",
    "FP arithmetic under a directed rounding mode (FPSCR[RN] != 0)",
    "lmw",
    "lmw with RA in the loaded range",
    "lswi/lswx with RA or RB inside the loaded register range",
    "lwarx",
    "mfmsr",
    "mfmsr (supervisor)",
    "mfspr/mtspr",
    "mfspr of a supervisor SPR",
    "mftb with a TBR other than 268/269",
    "mtmsr",
    "mtmsr (supervisor)",
    "mtspr of a supervisor SPR",
    "mulhw/mulhwu with the reserved OE bit set",
    "opcode 19 extended",
    "opcode 59 extended (reserved)",
    "opcode 63 extended (reserved)",
    "primary opcode 0 (reserved/zero word)",
    "reserved opcode",
    "reserved opcode-31 extended opcode",
    "reserved opcode (POWER2 quad / VSX, not 6xx)",
    "rfi",
    "sc with reserved bit 30 clear",
    "segment register access (supervisor)",
    "SPR 0 (601 MQ)",
    "SPR 22 (DEC)",
    "SPR 256 (VRSAVE)",
    "SPR 4-6 (601 RTC)",
    "SPR outside the user subset",
    "SPR (performance monitor)",
    "stmw",
    "stwcx.",
    "stwcx. with Rc=0",
    "TLB/segment management (supervisor)",
    "tlbsync",
    "update-form FP load/store with RA=0",
    "update-form load/store with RA=0 or RA=RT",
    REFUSED_UNBOUND,
    REFUSED_NO_FLAT_RAM,
    REFUSED_CREATE,
    REFUSED_CONTEXT,
    REFUSED_RAM,
    REFUSED_INSTANCE,
    REFUSED_HOST_PROTOCOL,
    REFUSED_EXIT_CODE,
];

/// The [`Trap::Refused`] reasons (in [`WHAT`], so a refusal crosses the table by id like any other reason).
pub const REFUSED_UNBOUND: &str = "no cpu-ppc.library table is bound";
pub const REFUSED_NO_FLAT_RAM: &str = "the guest RAM has no flat descriptor";
pub const REFUSED_CREATE: &str = "cpu-ppc.library refused to create an instance";
pub const REFUSED_CONTEXT: &str = "cpu-ppc.library refused the register context";
pub const REFUSED_RAM: &str = "cpu-ppc.library refused the RAM descriptor";
pub const REFUSED_INSTANCE: &str = "cpu-ppc.library refused the instance";
pub const REFUSED_HOST_PROTOCOL: &str = "a host callback answered a code the protocol does not define";
pub const REFUSED_EXIT_CODE: &str = "cpu-ppc.library returned an exit code the ABI does not define";

/// The [`Trap::Refused`] reason an [`EXIT_REFUSED`] / [`EXIT_HOST_PROTOCOL`] / unknown exit record names.
pub fn refusal_of(exit: &PpcExit) -> &'static str {
    match (exit.code, exit.word) {
        (EXIT_REFUSED, REFUSE_CONTEXT) => REFUSED_CONTEXT,
        (EXIT_REFUSED, REFUSE_RAM) => REFUSED_RAM,
        (EXIT_REFUSED, _) => REFUSED_INSTANCE,
        (EXIT_HOST_PROTOCOL, _) => REFUSED_HOST_PROTOCOL,
        _ => REFUSED_EXIT_CODE,
    }
}

fn what_id(s: &str) -> u32 {
    WHAT.iter().position(|w| *w == s).unwrap_or(0) as u32
}
fn what_str(id: u32) -> Option<&'static str> {
    WHAT.get(id as usize).copied()
}

impl PpcExit {
    fn new(code: u32, pc: u32, retired: u64) -> PpcExit {
        PpcExit { size: core::mem::size_of::<PpcExit>() as u32, code, pc, retired, ..PpcExit::default() }
    }
    pub fn budget(pc: u32, retired: u64) -> PpcExit { PpcExit::new(EXIT_BUDGET, pc, retired) }
    pub fn halt(pc: u32, retired: u64) -> PpcExit { PpcExit::new(EXIT_HALT, pc, retired) }
    pub fn block(pc: u32, retired: u64) -> PpcExit { PpcExit::new(EXIT_BLOCK, pc, retired) }
    pub fn host_protocol(pc: u32, bad_code: i32, retired: u64) -> PpcExit {
        PpcExit { word: bad_code as u32, ..PpcExit::new(EXIT_HOST_PROTOCOL, pc, retired) }
    }
    pub fn refused(which: u32) -> PpcExit { PpcExit { word: which, ..PpcExit::new(EXIT_REFUSED, 0, 0) } }

    /// The record for `trap` after `retired` instructions.
    pub fn trap(trap: &Trap, retired: u64) -> PpcExit {
        let (pc, word, addr, kind, what, write) = match *trap {
            Trap::Unimplemented { pc, word, what } => (pc, word, 0, T_UNIMPLEMENTED, what_id(what), 0),
            Trap::InvalidForm { pc, word, why } => (pc, word, 0, T_INVALID_FORM, what_id(why), 0),
            Trap::Privileged { pc, word, what } => (pc, word, 0, T_PRIVILEGED, what_id(what), 0),
            Trap::BusFault { pc, addr, write } => (pc, 0, addr, T_BUS_FAULT, 0, write as u32),
            Trap::Alignment { pc, addr, what } => (pc, 0, addr, T_ALIGNMENT, what_id(what), 0),
            Trap::Syscall { pc } => (pc, 0, 0, T_SYSCALL, 0, 0),
            Trap::TrapWord { pc, word } => (pc, word, 0, T_TRAP_WORD, 0, 0),
            Trap::NativeUnhandled { addr } => (addr, 0, addr, T_NATIVE_UNHANDLED, 0, 0),
            Trap::Refused { why } => (0, 0, 0, T_REFUSED, what_id(why), 0),
        };
        PpcExit { word, addr, detail: kind | what << 16, write, ..PpcExit::new(EXIT_TRAP, pc, retired) }
    }

    /// The trap an [`EXIT_TRAP`] record names; `None` for any other code and for a record no library writes (unknown kind or id,
    /// a wrong size).
    pub fn decode_trap(&self) -> Option<Trap> {
        if self.code != EXIT_TRAP || self.size != core::mem::size_of::<PpcExit>() as u32 { return None; }
        let (pc, word, addr) = (self.pc, self.word, self.addr);
        let what = what_str(self.detail >> 16)?;
        Some(match self.detail & 0xFF {
            T_UNIMPLEMENTED => Trap::Unimplemented { pc, word, what },
            T_INVALID_FORM => Trap::InvalidForm { pc, word, why: what },
            T_PRIVILEGED => Trap::Privileged { pc, word, what },
            T_BUS_FAULT => Trap::BusFault { pc, addr, write: self.write != 0 },
            T_ALIGNMENT => Trap::Alignment { pc, addr, what },
            T_SYSCALL => Trap::Syscall { pc },
            T_TRAP_WORD => Trap::TrapWord { pc, word },
            T_NATIVE_UNHANDLED => Trap::NativeUnhandled { addr },
            T_REFUSED => Trap::Refused { why: what },
            _ => return None,
        })
    }
}

/// Counters one `read_prof` snapshot holds (caller-owned).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PpcProf {
    pub size: u32,
    /// The tier now selected (`set_tier`): 0 interpreter, 1 blocks.
    pub tier: u32,
    pub insns: u64,
    pub natives: u64,
    pub blocks: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_invalidations: u64,
}

/// `PpcVTable::set_tier`.
pub const TIER_INTERP: u32 = 0;
pub const TIER_BLOCKS: u32 = 1;

/// cpu-ppc.library's table: `coral_lib_open()` returns it. Every entry returns 0 or a negative named refusal unless it says otherwise.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PpcVTable {
    /// [`PPC_ABI_VERSION`].
    pub version: u32,
    /// `size_of::<PpcVTable>()` as the library was built: entries are only ever appended.
    pub size: u32,
    /// [`crate::ARCH_PPC`].
    pub arch_id: u32,
    /// [`ABI_FAMILY_PPC_FLAT`].
    pub abi_family: u32,
    /// No feature bits are defined yet; a consumer refuses a table that sets one it does not know.
    pub features: u64,
    /// [`LAYOUT`] as the library was built: a mismatch with the consumer's is refused (the records differ).
    pub layout: u64,
    /// A new instance on `cfg` (validated first): 0 and `*out` set, or -1 (`*out` untouched).
    pub create: extern "C" fn(cfg: *const PpcCreate, out: *mut *mut crate::CpuInstance) -> i32,
    pub destroy: extern "C" fn(inst: *mut crate::CpuInstance) -> i32,
    /// Run `cpu` (ON it, in place) until a halt, a block, a trap or `budget` instructions; the record says which. Returns 0, or -1
    /// when the record could not be written (null).
    pub run: extern "C" fn(inst: *mut crate::CpuInstance, cpu: *mut PpcContext, budget: u64, exit: *mut PpcExit) -> i32,
    /// Make `addr` (word-aligned down) a native hook: reaching it calls `on_native` instead of executing an instruction.
    pub bind_native: extern "C" fn(inst: *mut crate::CpuInstance, addr: u32) -> i32,
    /// 1 if `addr` was bound (and is no longer), 0 if it was not.
    pub unbind_native: extern "C" fn(inst: *mut crate::CpuInstance, addr: u32) -> i32,
    /// [`TIER_INTERP`] or [`TIER_BLOCKS`]; anything else -1.
    pub set_tier: extern "C" fn(inst: *mut crate::CpuInstance, tier: u32) -> i32,
    pub read_prof: extern "C" fn(inst: *mut crate::CpuInstance, out: *mut PpcProf) -> i32,
    /// Keep the program counters of the last `n` executed instructions (0 = off).
    pub trace_pcs: extern "C" fn(inst: *mut crate::CpuInstance, n: u32) -> i32,
    /// The kept program counters, oldest first, into `out` (at most `cap`); `*written` = how many.
    pub recent_pcs: extern "C" fn(inst: *mut crate::CpuInstance, out: *mut u32, cap: u32, written: *mut u32) -> i32,
    /// Code decoded from `[addr, addr + len)` is stale (the guest's icbi range; a later JIT's invalidation hook).
    pub flush_icache_range: extern "C" fn(inst: *mut crate::CpuInstance, addr: u32, len: u32) -> i32,
}

/// A fingerprint of every record this face exchanges (sizes and the offsets that matter), so two sides built from different record
/// definitions refuse each other at open instead of misreading.
pub const LAYOUT: u64 = {
    use core::mem::{offset_of, size_of};
    let parts: [u64; 16] = [
        size_of::<PpcRegs>() as u64, offset_of!(PpcRegs, pc) as u64, offset_of!(PpcRegs, tb) as u64,
        size_of::<PpcContext>() as u64, offset_of!(PpcContext, regs) as u64, offset_of!(PpcContext, state) as u64,
        size_of::<CodeWatch>() as u64, offset_of!(CodeWatch, gen) as u64, offset_of!(CodeWatch, active) as u64,
        size_of::<FlatRam>() as u64, offset_of!(FlatRam, watch) as u64,
        size_of::<PpcHostCallbacks>() as u64, size_of::<PpcCreate>() as u64,
        size_of::<PpcExit>() as u64, size_of::<PpcProf>() as u64, size_of::<PpcVTable>() as u64,
    ];
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < parts.len() { h = (h ^ parts[i]).wrapping_mul(0x0100_0000_01b3); i += 1; }
    h
};

/// Why a table was refused (each by name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpcOpenRefusal {
    NullTable,
    /// Not the PowerPC flat face (a v2 CpuVTable, or anything else): `found` is the fourth word.
    WrongFamily { found: u32 },
    WrongArch { found: u32 },
    VersionTooOld { found: u32, required: u32 },
    TableTooSmall { found: u32, required: u32 },
    UnknownFeatures { found: u64 },
    LayoutMismatch { found: u64, expected: u64 },
}

/// Check a table pointer before forming a reference to it: the header words are read unaligned, one at a time, family first (a v2
/// table is refused before anything past its own header is read).
///
/// # Safety
/// `p` must be null or point at least at 16 readable bytes; a success additionally requires `size` readable bytes, which the
/// library's own `size` asserts.
pub unsafe fn check_ppc(p: *const PpcVTable, required_version: u32) -> Result<(), PpcOpenRefusal> {
    if p.is_null() { return Err(PpcOpenRefusal::NullTable); }
    let rd32 = |q: *const u32| unsafe { q.read_unaligned() };
    let family = rd32(core::ptr::addr_of!((*p).abi_family));
    if family != ABI_FAMILY_PPC_FLAT { return Err(PpcOpenRefusal::WrongFamily { found: family }); }
    let arch = rd32(core::ptr::addr_of!((*p).arch_id));
    if arch != crate::ARCH_PPC { return Err(PpcOpenRefusal::WrongArch { found: arch }); }
    let version = rd32(core::ptr::addr_of!((*p).version));
    if version < required_version { return Err(PpcOpenRefusal::VersionTooOld { found: version, required: required_version }); }
    let size = rd32(core::ptr::addr_of!((*p).size));
    let need = core::mem::size_of::<PpcVTable>() as u32;
    if size < need { return Err(PpcOpenRefusal::TableTooSmall { found: size, required: need }); }
    let features = unsafe { core::ptr::addr_of!((*p).features).read_unaligned() };
    if features != 0 { return Err(PpcOpenRefusal::UnknownFeatures { found: features }); }
    let layout = unsafe { core::ptr::addr_of!((*p).layout).read_unaligned() };
    if layout != LAYOUT { return Err(PpcOpenRefusal::LayoutMismatch { found: layout, expected: LAYOUT }); }
    Ok(())
}

// ---- The plain types the static core (heirloom-cpu-ppc) and the engine-side client (heirloom-cpu-ppc-client) both expose: ONE
// definition each.

pub const XER_SO: u32 = 0x8000_0000;
pub const XER_OV: u32 = 0x4000_0000;
pub const XER_CA: u32 = 0x2000_0000;
/// XER bits a program can write: SO, OV, CA and the 7-bit string byte count (PEM 2.1.3).
pub const XER_MASK: u32 = 0xE000_007F;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    /// Instructions retired by this call (a native hook counts as one).
    pub insns: u64,
    /// True if stopped by `Cpu::halt`.
    pub halted: bool,
    /// True if stopped by `Cpu::block`; the next resume continues the guest.
    pub blocked: bool,
    /// Why execution stopped on a fault; `None` for halt, block or budget exhaustion.
    pub trap: Option<Trap>,
}

/// A register, for `Cpu::set_reg` / `Cpu::reg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg { Gpr(u8), Pc, Lr, Ctr, Xer, Cr, Fpscr }


/// CR field `n` of `cr` (0 = the most significant nibble: CR0).
pub fn cr_field(cr: u32, n: u32) -> u32 { (cr >> (28 - 4 * (n & 7))) & 0xF }
pub fn set_cr_field(cr: &mut u32, n: u32, v: u32) {
    let sh = 28 - 4 * (n & 7);
    *cr = (*cr & !(0xF << sh)) | ((v & 0xF) << sh);
}
/// CR bit `i` in manual numbering (0 = CR0[LT]).
pub fn cr_bit(cr: u32, i: u32) -> bool { (cr >> (31 - (i & 31))) & 1 != 0 }
pub fn set_cr_bit(cr: &mut u32, i: u32, v: bool) {
    let m = 1u32 << (31 - (i & 31));
    if v { *cr |= m } else { *cr &= !m }
}

/// Which interpreter an Exec's `resume` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Tier 0: the reference, one instruction at a time.
    Interp,
    /// Tier 1: pre-decoded blocks, invalidated by code-page write generations (heirloom-cpu-ppc's t1cache).
    Blocks,
    /// Tier 1 checked against tier 0 on every block (slow; for gates and triage). Stops at the first divergence,
    /// which the static core's `Exec::divergence` (the library does not serve lockstep) then names.
    Lockstep,
}

impl Tier {
    /// The number a `[PPC-PROF ... tier=]` line prints.
    pub fn number(self) -> u32 { match self { Tier::Interp => 0, Tier::Blocks | Tier::Lockstep => 1 } }
    /// Parse `0` / `interp`, `1` / `blocks`, `lockstep`; None for anything else.
    pub fn parse(s: &str) -> Option<Tier> {
        match s { "0" | "interp" => Some(Tier::Interp), "1" | "blocks" => Some(Tier::Blocks), "lockstep" => Some(Tier::Lockstep), _ => None }
    }
}

/// Cheap counters, kept by every tier (none costs anything per instruction).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Prof {
    /// Guest instructions retired (a native hook counts as one).
    pub insns: u64,
    /// Native hooks entered (Toolbox / library callouts).
    pub natives: u64,
    /// Time spent inside native hooks, in the clock's units (only with the static core's `Exec::profile_natives`).
    pub native_time: u64,
    /// Tier 1: blocks run.
    pub blocks: u64,
}

impl Prof {
    /// The profile line: `[PPC-PROF insns=<n> native_calls=<n> elapsed_us=<n> mips=<n.nn> tier=<t>]`. Elapsed time is
    /// the caller's (this crate has no clock).
    pub fn line(&self, elapsed_us: u64, tier: Tier) -> alloc::string::String {
        let mips = if elapsed_us == 0 { 0.0 } else { self.insns as f64 / elapsed_us as f64 };
        alloc::format!("[PPC-PROF insns={} native_calls={} elapsed_us={elapsed_us} mips={mips:.2} tier={}]", self.insns, self.natives, tier.number())
    }
}

