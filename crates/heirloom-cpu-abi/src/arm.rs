//! The ARM face of the CPU contract (`ARCH_ARM`, cpu-arm.library): the records the table speaks for an ARMv6 user-mode core,
//! and the Rust types both sides decode them to. ONE definition each (ONE-PARSER-PER-FORMAT): heirloom-cpu-arm (the core,
//! behind the table) re-exports these types as its own API, and heirloom-cpu-arm-client (the engine side, which links no core)
//! uses the same ones; [`ArmExit::encode`] / [`ArmExit::decode`] here are the only codec of the exit record. So a guest program sees one `Exit`, one `Fault` and one register file whichever side runs it.
//!
//! The Rust types moved here unchanged from heirloom-cpu-arm (their shapes adapted in part from HexRaysSA/rax 526acfa, MIT).
//!
//! What ARM adds to the 68k/PPC shape, and why:
//! - **Fetches are their own bus calls** ([`ArmBusCallbacks`]): an engine applies execute permission on a fetch, never on a
//!   read, so the core cannot fold fetches into `read16`/`read32`. `create` for `ARCH_ARM` takes a pointer to this record,
//!   whose first field IS the common [`CpuBusCallbacks`].
//! - **The saved context is a published layout** ([`ArmContext`]), not opaque bytes: the engine keeps one register file per
//!   guest thread and switches threads by `load_context`, so it must read and write registers between runs at no cost.
//! - **Why a run stopped** is a record in the context ([`ArmExit`]), because an [`Exit::Trap`] names an instruction word, its
//!   state and a kind with operands that [`CpuStop`](crate::CpuStop) cannot carry.
//! - An SVC does not call `on_trap`: the run stops (`STOP_TRAP`) with the immediate in the exit record and the PC past the SVC,
//!   and the engine dispatches the EKA2 exec call between runs, exactly as with the core linked in.

use crate::CpuBusCallbacks;

/// `create`'s `sub_arch` for ARMv6 (ARM1136/ARM1176). Anything else is refused with a null instance.
pub const SUB_ARCH_ARMV6: u32 = 6;

/// The bus a cpu-arm.library instance runs on: the common callbacks (`base.read*`/`write*` for data, `base.on_trap` unused)
/// plus the two instruction fetches. 0 = success, nonzero = the access faulted.
#[repr(C)]
pub struct ArmBusCallbacks {
    pub base: CpuBusCallbacks,
    /// Thumb fetch (`addr` even).
    pub fetch16: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, out: *mut u32) -> i32,
    /// A32 fetch (`addr` 4-aligned).
    pub fetch32: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, out: *mut u32) -> i32,
}

/// `ArmExit::code`.
pub const EXIT_BUDGET: u32 = 0;
pub const EXIT_SVC: u32 = 1;
pub const EXIT_BKPT: u32 = 2;
pub const EXIT_WAIT: u32 = 3;
pub const EXIT_FAULT: u32 = 4;
pub const EXIT_TRAP: u32 = 5;

/// Why the last run stopped, as words. [`ArmExit::encode`] / [`ArmExit::decode`] are the one codec.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmExit {
    pub code: u32,
    /// Fault/Trap: the instruction's address.
    pub pc: u32,
    /// Svc/Bkpt: the immediate. Fault: the address. Trap: the instruction word.
    pub a: u32,
    /// Fault: the access (0 read, 1 write, 2 fetch). Trap: bit 0 = Thumb, bits 8.. = the trap kind (TRAP_*).
    pub b: u32,
    /// Trap: the kind's operands (see [`ArmExit::encode`]).
    pub c: u32,
    pub d: u32,
}

const TRAP_UNDEFINED: u32 = 0;
const TRAP_UNPREDICTABLE: u32 = 1;
const TRAP_PRIVILEGED: u32 = 2;
const TRAP_UNALIGNED: u32 = 3;
const TRAP_COPROCESSOR: u32 = 4;
const TRAP_CP15: u32 = 5;
const TRAP_VFP_SYSREG: u32 = 6;
const TRAP_VFP_EXCEPTION: u32 = 7;

impl ArmExit {
    pub fn encode(exit: &Exit) -> ArmExit {
        let mut e = ArmExit::default();
        match *exit {
            Exit::BudgetSpent => e.code = EXIT_BUDGET,
            Exit::Svc(imm) => { e.code = EXIT_SVC; e.a = imm; }
            Exit::Bkpt(imm) => { e.code = EXIT_BKPT; e.a = imm as u32; }
            Exit::Wait => e.code = EXIT_WAIT,
            Exit::Fault { pc, fault } => {
                e.code = EXIT_FAULT;
                e.pc = pc;
                e.a = fault.addr;
                e.b = match fault.access { Access::Read => 0, Access::Write => 1, Access::Fetch => 2 };
            }
            Exit::Trap(t) => {
                e.code = EXIT_TRAP;
                e.pc = t.pc;
                e.a = t.insn;
                let (kind, c, d) = match t.kind {
                    TrapKind::Undefined => (TRAP_UNDEFINED, 0, 0),
                    TrapKind::Unpredictable => (TRAP_UNPREDICTABLE, 0, 0),
                    TrapKind::Privileged => (TRAP_PRIVILEGED, 0, 0),
                    TrapKind::Unaligned { addr, write } => (TRAP_UNALIGNED, addr, write as u32),
                    TrapKind::Coprocessor { cp } => (TRAP_COPROCESSOR, cp as u32, 0),
                    TrapKind::Cp15 { read, opc1, crn, crm, opc2 } => (
                        TRAP_CP15,
                        u32::from_le_bytes([opc1, crn, crm, opc2]),
                        read as u32,
                    ),
                    TrapKind::VfpSystemRegister { reg } => (TRAP_VFP_SYSREG, reg as u32, 0),
                    TrapKind::VfpException { flags } => (TRAP_VFP_EXCEPTION, flags as u32, 0),
                };
                e.b = t.thumb as u32 | kind << 8;
                e.c = c;
                e.d = d;
            }
        }
        e
    }

    /// The exit this record names; `None` for a record no core writes (an unknown code, access or trap kind).
    pub fn decode(&self) -> Option<Exit> {
        Some(match self.code {
            EXIT_BUDGET => Exit::BudgetSpent,
            EXIT_SVC => Exit::Svc(self.a),
            EXIT_BKPT => Exit::Bkpt(u16::try_from(self.a).ok()?),
            EXIT_WAIT => Exit::Wait,
            EXIT_FAULT => {
                let access = match self.b { 0 => Access::Read, 1 => Access::Write, 2 => Access::Fetch, _ => return None };
                Exit::Fault { pc: self.pc, fault: Fault { addr: self.a, access } }
            }
            EXIT_TRAP => {
                let [opc1, crn, crm, opc2] = self.c.to_le_bytes();
                let byte = |v: u32| u8::try_from(v).ok();
                let kind = match self.b >> 8 {
                    TRAP_UNDEFINED => TrapKind::Undefined,
                    TRAP_UNPREDICTABLE => TrapKind::Unpredictable,
                    TRAP_PRIVILEGED => TrapKind::Privileged,
                    TRAP_UNALIGNED => TrapKind::Unaligned { addr: self.c, write: self.d != 0 },
                    TRAP_COPROCESSOR => TrapKind::Coprocessor { cp: byte(self.c)? },
                    TRAP_CP15 => TrapKind::Cp15 { read: self.d != 0, opc1, crn, crm, opc2 },
                    TRAP_VFP_SYSREG => TrapKind::VfpSystemRegister { reg: byte(self.c)? },
                    TRAP_VFP_EXCEPTION => TrapKind::VfpException { flags: byte(self.c)? },
                    _ => return None,
                };
                Exit::Trap(Trap { pc: self.pc, insn: self.a, thumb: self.b & 1 != 0, kind })
            }
            _ => return None,
        })
    }
}

/// `ArmContext::unaligned`.
pub const UNALIGNED_ARMV6: u32 = 0;
pub const UNALIGNED_LEGACY: u32 = 1;
pub const UNALIGNED_STRICT: u32 = 2;

/// One ARMv6 user-mode thread's whole state, as `save_context` writes it and `load_context` reads it (`context_size` returns
/// its size). The configuration rides with it, so one library instance runs every guest thread of an engine.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmContext {
    /// R0-R15; R15 is the address of the next instruction.
    pub regs: [u32; 16],
    /// NZCVQ, GE, E and T as MRS reads them (the mode field reads User and is ignored on load).
    pub cpsr: u32,
    pub vfp_s: [u32; 32],
    pub fpscr: u32,
    pub tpidrurw: u32,
    pub tpidruro: u32,
    /// UNALIGNED_*.
    pub unaligned: u32,
    /// 1 = VFPv2 present and enabled.
    pub vfp: u32,
    /// The local exclusive monitor: the tagged address, and its size in bytes (0 = no reservation).
    pub exclusive_addr: u32,
    pub exclusive_size: u32,
    /// Why the last run stopped (written by `save_context`; ignored by `load_context`).
    pub exit: ArmExit,
}

/// The CPSR's user-visible mode field: always User.
pub const MODE_USR: u32 = 0x10;

/// The CPSR fields a user-mode thread has, and their one packing (MRS's view: mode = User, I/F/A clear).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Psr {
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// The sticky saturation flag.
    pub q: bool,
    /// The SIMD GE[3:0] flags.
    pub ge: u8,
    /// CPSR.E (SETEND BE).
    pub big_endian: bool,
    /// CPSR.T.
    pub thumb: bool,
}

impl Psr {
    pub fn pack(&self) -> u32 {
        (self.n as u32) << 31
            | (self.z as u32) << 30
            | (self.c as u32) << 29
            | (self.v as u32) << 28
            | (self.q as u32) << 27
            | (self.ge as u32 & 0xF) << 16
            | (self.big_endian as u32) << 9
            | (self.thumb as u32) << 5
            | MODE_USR
    }

    /// Flags, GE, E and T of `value` (the mode field is ignored: a user-mode thread).
    pub fn unpack(value: u32) -> Psr {
        Psr {
            n: value & (1 << 31) != 0,
            z: value & (1 << 30) != 0,
            c: value & (1 << 29) != 0,
            v: value & (1 << 28) != 0,
            q: value & (1 << 27) != 0,
            ge: ((value >> 16) & 0xF) as u8,
            big_endian: value & (1 << 9) != 0,
            thumb: value & (1 << 5) != 0,
        }
    }
}

// ── The Rust types both sides share ─────────────────────────────────────────────────────────────────────────────────────────

/// How unaligned data accesses behave (ARMv6 SCTLR.U and SCTLR.A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unaligned {
    /// U=1, A=0 (EKA2's setting): LDR/STR/LDRH/STRH/LDRSH of any alignment are performed as unaligned
    /// accesses; LDM/STM, LDRD/STRD, SWP, VFP loads/stores and the exclusives must be word-aligned (the
    /// exclusives naturally aligned) or they trap.
    Armv6,
    /// U=0, A=0 (the ARMv5 model): an unaligned LDR loads the aligned word rotated, STR and the multiples
    /// ignore address bits [1:0], a halfword access ignores bit 0.
    Legacy,
    /// A=1: every access must be naturally aligned (LDRD/STRD and the multiples word-aligned).
    Strict,
}

impl Unaligned {
    pub fn to_raw(self) -> u32 {
        match self { Unaligned::Armv6 => UNALIGNED_ARMV6, Unaligned::Legacy => UNALIGNED_LEGACY, Unaligned::Strict => UNALIGNED_STRICT }
    }
    pub fn from_raw(raw: u32) -> Option<Unaligned> {
        match raw { UNALIGNED_ARMV6 => Some(Unaligned::Armv6), UNALIGNED_LEGACY => Some(Unaligned::Legacy), UNALIGNED_STRICT => Some(Unaligned::Strict), _ => None }
    }
}

/// Static CPU configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub unaligned: Unaligned,
    /// VFPv2 present and enabled (CPACR + FPEXC.EN set by the OS). When false, cp10/cp11 trap.
    pub vfp: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { unaligned: Unaligned::Armv6, vfp: true }
    }
}

/// The VFPv2 register file: S0-S31, aliased as D0-D15 (D[i] = S[2i+1]:S[2i]), and FPSCR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vfp {
    pub s: [u32; 32],
    pub fpscr: u32,
}

impl Vfp {
    pub fn d(&self, i: usize) -> u64 {
        self.s[2 * i] as u64 | (self.s[2 * i + 1] as u64) << 32
    }

    pub fn set_d(&mut self, i: usize, v: u64) {
        self.s[2 * i] = v as u32;
        self.s[2 * i + 1] = (v >> 32) as u32;
    }
}

/// The kind of access that faulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Fetch,
}

/// A memory access the host refused (unmapped, protection, ...). The host returns it from [`Memory`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub addr: u32,
    pub access: Access,
}

/// Guest memory, little-endian, implemented by the host.
///
/// The CPU only calls the 16-bit methods with even addresses and the 32-bit methods with 4-aligned addresses;
/// an unaligned access the architecture allows is composed from byte accesses by the CPU itself.
pub trait Memory {
    fn read_u8(&mut self, addr: u32) -> Result<u8, Fault>;
    fn read_u16(&mut self, addr: u32) -> Result<u16, Fault>;
    fn read_u32(&mut self, addr: u32) -> Result<u32, Fault>;
    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault>;
    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault>;
    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault>;
    /// Instruction fetch (Thumb); the host may apply execute permission here.
    fn fetch_u16(&mut self, addr: u32) -> Result<u16, Fault>;
    /// Instruction fetch (A32).
    fn fetch_u32(&mut self, addr: u32) -> Result<u32, Fault>;
}

/// Why `run`/`step` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The instruction budget is used up; the CPU is between instructions.
    BudgetSpent,
    /// SVC/SWI with its immediate (24 bits in A32, 8 in Thumb). PC is already past the SVC.
    Svc(u32),
    /// BKPT with its immediate. PC is left on the BKPT.
    Bkpt(u16),
    /// WFI or WFE. PC is past the hint; the host may block the thread.
    Wait,
    /// The host's memory refused an access. PC is left on the instruction and no register was written.
    Fault { pc: u32, fault: Fault },
    /// An instruction this core does not execute. PC is left on it and no state was changed.
    Trap(Trap),
}

/// A named refusal to execute one instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    pub pc: u32,
    /// The instruction word (a Thumb halfword is zero-extended).
    pub insn: u32,
    pub thumb: bool,
    pub kind: TrapKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    /// UNDEFINED on ARMv6 (this includes ARMv6T2/ARMv7-only encodings, Thumb-2, and NEON).
    Undefined,
    /// UNPREDICTABLE on ARMv6: refused rather than guessed.
    Unpredictable,
    /// A privileged instruction or register access from user mode (SRS, RFE, SMC, SPSR, exception return, ...).
    Privileged,
    /// A data access whose alignment the configured [`Unaligned`] mode refuses.
    Unaligned { addr: u32, write: bool },
    /// An access to a coprocessor this core does not have.
    Coprocessor { cp: u8 },
    /// A CP15 register user mode cannot reach.
    Cp15 { read: bool, opc1: u8, crn: u8, crm: u8, opc2: u8 },
    /// A VFP system register user mode cannot reach (FPEXC, ...), or VFP is configured off.
    VfpSystemRegister { reg: u8 },
    /// A VFP operation raised a floating-point exception whose FPSCR trap-enable bit the guest set (bits are
    /// FPSCR's cumulative-flag layout: IOC=0, DZC=1, OFC=2, UFC=3, IXC=4, IDC=7). Untrapped exceptions only
    /// set the cumulative flags, as on hardware.
    VfpException { flags: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exit_round_trips_through_the_record() {
        let f = |addr, access| Exit::Fault { pc: 0x8000, fault: Fault { addr, access } };
        let t = |kind, thumb| Exit::Trap(Trap { pc: 0x8004, insn: 0xE7F0_00F0, thumb, kind });
        let all = [
            Exit::BudgetSpent,
            Exit::Svc(0x00FF_FFFF),
            Exit::Bkpt(0xBEEF),
            Exit::Wait,
            f(0x1234_5678, Access::Read),
            f(4, Access::Write),
            f(0xFFFF_FFFC, Access::Fetch),
            t(TrapKind::Undefined, false),
            t(TrapKind::Unpredictable, true),
            t(TrapKind::Privileged, false),
            t(TrapKind::Unaligned { addr: 0x1001, write: true }, false),
            t(TrapKind::Coprocessor { cp: 14 }, false),
            t(TrapKind::Cp15 { read: true, opc1: 0, crn: 13, crm: 0, opc2: 3 }, false),
            t(TrapKind::VfpSystemRegister { reg: 8 }, true),
            t(TrapKind::VfpException { flags: 0x9F }, false),
        ];
        for exit in all {
            assert_eq!(ArmExit::encode(&exit).decode(), Some(exit), "{exit:?}");
        }
    }

    #[test]
    fn a_record_no_core_writes_is_refused() {
        assert_eq!(ArmExit { code: 99, ..Default::default() }.decode(), None);
        assert_eq!(ArmExit { code: EXIT_FAULT, b: 3, ..Default::default() }.decode(), None);
        assert_eq!(ArmExit { code: EXIT_TRAP, b: 42 << 8, ..Default::default() }.decode(), None);
        assert_eq!(ArmExit { code: EXIT_BKPT, a: 0x1_0000, ..Default::default() }.decode(), None);
    }
}
