//! Register state, traps and run statistics.
//!
//! Bit numbering follows the PowerPC manuals: bit 0 is the MOST significant bit. Helpers here convert, so the
//! rest of the crate can quote the manual's numbers.
use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::fmt;

pub const XER_SO: u32 = 0x8000_0000;
pub const XER_OV: u32 = 0x4000_0000;
pub const XER_CA: u32 = 0x2000_0000;
/// XER bits a program can write: SO, OV, CA and the 7-bit string byte count (PEM 2.1.3).
pub const XER_MASK: u32 = 0xE000_007F;

/// Why the interpreter stopped (other than [`Cpu::halt`] / [`Cpu::block`] / the instruction budget). Every variant
/// leaves `pc` AT the faulting instruction, so a host can inspect, patch and resume.
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
        }
    }
}
impl core::error::Error for Trap {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    /// Instructions retired by this call (a native hook counts as one).
    pub insns: u64,
    /// True if stopped by [`Cpu::halt`].
    pub halted: bool,
    /// True if stopped by [`Cpu::block`]; the next resume continues the guest.
    pub blocked: bool,
    /// Why execution stopped on a fault; `None` for halt, block or budget exhaustion.
    pub trap: Option<Trap>,
}

/// A register, for [`Cpu::set_reg`] / [`Cpu::reg`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg { Gpr(u8), Pc, Lr, Ctr, Xer, Cr, Fpscr }

/// One PowerPC hardware thread. It is also the saved context: the interpreter has no global state, so any number of
/// guests can run in one process (unlike heirloom-cpu-68k's Musashi).
#[derive(Clone)]
pub struct Cpu {
    pub gpr: [u32; 32],
    /// FPRs as raw IEEE-754 double bit patterns.
    pub fpr: [u64; 32],
    pub cr: u32,
    pub lr: u32,
    pub ctr: u32,
    pub xer: u32,
    pub fpscr: u32,
    pub pc: u32,
    /// Time base, advanced one tick per retired instruction.
    pub tb: u64,
    pub(crate) reservation: Option<u32>,
    /// Shared copy-on-write: cloning a CPU (a saved thread context, a Mixed Mode frame) does not copy the set.
    pub(crate) natives: Arc<BTreeSet<u32>>,
    /// Version of `natives`: 0 for the empty set, else a process-unique number taken at each change, so two CPUs
    /// (clones, saved contexts) with equal versions have equal sets. A block cache re-checks its blocks when it changes.
    pub(crate) natives_gen: u64,
    pub(crate) next: u32,
    pub(crate) redirected: bool,
    halted: bool,
    blocked: bool,
    /// The last `trace_cap` program counters executed (oldest first); empty and free when `trace_cap` is 0.
    trace: alloc::collections::VecDeque<u32>,
    trace_cap: usize,
}

/// The saved state of one guest; the same type, named as in heirloom-cpu-68k.
pub type CpuContext = Cpu;

impl Cpu {
    /// A reset CPU that will execute at `entry` with r1 = `sp`; everything else zero.
    pub fn new(entry: u32, sp: u32) -> Cpu {
        let mut gpr = [0u32; 32];
        gpr[1] = sp;
        Cpu {
            gpr, fpr: [0; 32], cr: 0, lr: 0, ctr: 0, xer: 0, fpscr: 0, pc: entry, tb: 0, reservation: None,
            natives: Arc::new(BTreeSet::new()), natives_gen: 0, next: 0, redirected: false, halted: false, blocked: false, trace: alloc::collections::VecDeque::new(), trace_cap: 0,
        }
    }
    pub fn reg(&self, r: Reg) -> u32 {
        match r {
            Reg::Gpr(n) => self.gpr[(n & 31) as usize],
            Reg::Pc => self.pc, Reg::Lr => self.lr, Reg::Ctr => self.ctr,
            Reg::Xer => self.xer, Reg::Cr => self.cr, Reg::Fpscr => self.fpscr,
        }
    }
    pub fn set_reg(&mut self, r: Reg, v: u32) {
        match r {
            Reg::Gpr(n) => self.gpr[(n & 31) as usize] = v,
            Reg::Pc => self.set_pc(v), Reg::Lr => self.lr = v, Reg::Ctr => self.ctr = v,
            Reg::Xer => self.xer = v & XER_MASK, Reg::Cr => self.cr = v,
            Reg::Fpscr => { self.fpscr = v; crate::fpscr::fix(self) }
        }
    }
    pub fn fpr_f64(&self, n: usize) -> f64 { f64::from_bits(self.fpr[n & 31]) }
    pub fn set_fpr_f64(&mut self, n: usize, v: f64) { self.fpr[n & 31] = v.to_bits() }
    /// Continue at `pc`. Inside a host callback this REPLACES the default continuation (the next instruction, or
    /// LR for a native hook); outside one it just sets the program counter.
    pub fn set_pc(&mut self, pc: u32) { self.pc = pc; self.redirected = true; }
    /// Bind `addr` as native glue: when execution reaches it, [`crate::Host::native`] runs instead of an
    /// instruction. If the host handles it without redirecting, the guest returns through LR (an implicit `blr`).
    pub fn bind_native(&mut self, addr: u32) {
        let a = addr & !3;
        if !self.natives.contains(&a) { Arc::make_mut(&mut self.natives).insert(a); self.natives_changed(); }
    }
    pub fn unbind_native(&mut self, addr: u32) -> bool {
        let a = addr & !3;
        let gone = self.natives.contains(&a);
        if gone { Arc::make_mut(&mut self.natives).remove(&a); self.natives_changed(); }
        gone
    }
    fn natives_changed(&mut self) {
        static NEXT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
        self.natives_gen = NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    pub fn is_native(&self, addr: u32) -> bool { self.natives.contains(&(addr & !3)) }
    /// Stop after the current instruction; the run is over.
    pub fn halt(&mut self) { self.halted = true }
    /// Stop after the current instruction but leave the guest resumable (a host call that must wait).
    pub fn block(&mut self) { self.blocked = true }
    /// Keep the program counters of the last `n` executed instructions (0 turns it off): the post-mortem of a fault.
    pub fn trace_pcs(&mut self, n: usize) { self.trace_cap = n; self.trace.clear(); }
    /// The recorded program counters, oldest first (the faulting instruction is the last).
    pub fn recent_pcs(&self) -> impl Iterator<Item = u32> + '_ { self.trace.iter().copied() }
    #[inline]
    pub(crate) fn tracing(&self) -> bool { self.trace_cap != 0 }
    pub(crate) fn trace_snapshot(&self) -> alloc::collections::VecDeque<u32> { self.trace.clone() }
    pub(crate) fn trace_restore(&mut self, t: &alloc::collections::VecDeque<u32>) { self.trace.clone_from(t); }
    pub(crate) fn note_pc(&mut self, pc: u32) {
        if self.trace_cap == 0 { return; }
        if self.trace.len() == self.trace_cap { self.trace.pop_front(); }
        self.trace.push_back(pc);
    }
    pub(crate) fn halted(&self) -> bool { self.halted }
    pub(crate) fn blocked(&self) -> bool { self.blocked }
    pub(crate) fn clear_blocked(&mut self) { self.blocked = false }

    /// CR field `n` (0 = most significant nibble: CR0).
    pub fn cr_field(&self, n: u32) -> u32 { (self.cr >> (28 - 4 * (n & 7))) & 0xF }
    pub fn set_cr_field(&mut self, n: u32, v: u32) {
        let sh = 28 - 4 * (n & 7);
        self.cr = (self.cr & !(0xF << sh)) | ((v & 0xF) << sh);
    }
    /// CR bit `i` in manual numbering (0 = CR0[LT]).
    pub fn cr_bit(&self, i: u32) -> bool { (self.cr >> (31 - (i & 31))) & 1 != 0 }
    pub fn set_cr_bit(&mut self, i: u32, v: bool) {
        let m = 1u32 << (31 - (i & 31));
        if v { self.cr |= m } else { self.cr &= !m }
    }
    /// CR0 = LT/GT/EQ of `result` read as signed, plus XER[SO] (what every Rc=1 integer instruction does).
    pub(crate) fn set_cr0(&mut self, result: u32) {
        self.set_cr_compare(0, (result as i32).cmp(&0));
    }
    /// CR field `bf` = LT/GT/EQ for `ord` plus XER[SO] (cmp, cmpl and every Rc=1 integer instruction).
    pub(crate) fn set_cr_compare(&mut self, bf: u32, ord: core::cmp::Ordering) {
        let so = (self.xer & XER_SO != 0) as u32;
        let f = match ord {
            core::cmp::Ordering::Less => 8, core::cmp::Ordering::Greater => 4, core::cmp::Ordering::Equal => 2,
        };
        self.set_cr_field(bf, f | so);
    }
}
