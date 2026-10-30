//! heirloom-cpu-ppc's engine-facing API - [`Cpu`], [`Host`], [`Exec`], [`RunStats`], [`Trap`], [`Tier`] - driven through
//! cpu-ppc.library's [`PpcVTable`] (CPU ABI v3, heirloom-cpu-abi::ppc) instead of a linked core, so the macppc engine names its
//! CPU through one module and switches backends by a feature (doctrine ONE-NAME-ONE-CONCERN-LOADED; docs/heirloom/plan.tsv row
//! cpu-ppc). The ~300 `cpu.gpr[..]` / `cpu.lr` / `cpu.halt()` uses in the engine compile unchanged against either.
//!
//! How a run crosses the table: the [`Cpu`] here is a guest thread's register file and bound natives (cloned freely, as with the
//! core). [`Exec`] owns ONE library instance (its tier-1 block cache persists across runs: that is where the speed is) created on
//! the engine's RAM descriptor ([`GuestMem::flat_ram`]). A run publishes the Cpu as a [`PpcContext`], the library executes ON it,
//! and a native hook comes back through [`Host::native`] with the same `&mut Cpu` the core would pass; binds the engine makes in
//! a native are synced to the instance before it returns (the library queues them), so a glue cell bound mid-run is honoured.
#![cfg_attr(not(test), no_std)]
// `bind-static` exists for HOST tests and host tools only: it links the static core so a test can bind its table. A CoralOS build
// (target_os = "none") opens cpu-ppc.library instead, and linking the core there would be the CPU twice (doctrine
// LIBRARY-NOT-LINKED-TWICE) - refused at compile time, not by convention.
#[cfg(all(feature = "bind-static", target_os = "none"))]
compile_error!("heirloom-cpu-ppc-client: `bind-static` is the host-test binding; a CoralOS engine binds the PpcVTable it opened from cpu-ppc.library");

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

use heirloom_cpu_abi::ppc::{
    check_ppc, PpcContext, PpcCreate, PpcExit, PpcHostCallbacks, PpcOpenRefusal, PpcProf, CTX_HALTED, CTX_RUNNABLE,
    EXIT_BLOCK, EXIT_BUDGET, EXIT_HALT, EXIT_TRAP, HOST_BLOCK, HOST_HALT, HOST_HANDLED, HOST_REDIRECTED, HOST_UNHANDLED,
    REFUSED_CREATE, REFUSED_NO_FLAT_RAM, REFUSED_UNBOUND, SUB_ARCH_G3, TIER_BLOCKS, TIER_INTERP, WHAT_UNLISTED, refusal_of,
};
use heirloom_cpu_abi::CpuInstance;
pub use heirloom_cpu_abi::ppc::{Prof, PpcVTable, Reg, RunStats, Tier, Trap, PPC_ABI_VERSION, XER_CA, XER_MASK, XER_OV, XER_SO};
use heirloom_mem_flat32::GuestMem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindRefusal {
    Open(PpcOpenRefusal),
    AlreadyBound,
}

static VTABLE: AtomicPtr<PpcVTable> = AtomicPtr::new(ptr::null_mut());

/// Bind cpu-ppc.library's table (the engine does this once, with the table `coral_lib_open` returned, before any guest runs).
/// Binding the same table again is fine; a different one is refused.
pub fn bind(table: &'static PpcVTable) -> Result<(), BindRefusal> {
    // SAFETY: a &'static table is a full readable PpcVTable.
    unsafe { check_ppc(table, PPC_ABI_VERSION) }.map_err(BindRefusal::Open)?;
    let p = table as *const PpcVTable as *mut PpcVTable;
    match VTABLE.compare_exchange(ptr::null_mut(), p, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(()),
        Err(cur) if cur == p => Ok(()),
        Err(_) => Err(BindRefusal::AlreadyBound),
    }
}

/// Whether a table is bound (with `bind-static`, always: the static core's is bound on first use). os-mac.library's PowerPC family
/// refuses to make a machine before this is true, so no guest reaches an unbound CPU.
pub fn is_bound() -> bool {
    table().is_some()
}

/// The bound table (with `bind-static`, the static core's when nothing was bound).
fn table() -> Option<&'static PpcVTable> {
    #[cfg(feature = "bind-static")]
    if VTABLE.load(Ordering::Acquire).is_null() {
        let _ = bind(heirloom_cpu_ppc::table::vtable());
    }
    // SAFETY: only `bind` stores, and only a checked &'static table.
    unsafe { VTABLE.load(Ordering::Acquire).as_ref() }
}

/// One PowerPC hardware thread as the engine sees it: its register file (public fields, as the core's) and its bound natives.
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
    reservation: Option<u32>,
    natives: Arc<BTreeSet<u32>>,
    natives_gen: u64,
    redirected: bool,
    halted: bool,
    blocked: bool,
    trace_cap: usize,
    /// The instance that ran this Cpu last (its trace ring answers `recent_pcs`).
    last: *mut CpuInstance,
}

// SAFETY: `last` is only a handle handed back to the table's recent_pcs (it is never dereferenced here); the natives set is an Arc.
// Moving a Cpu to another thread moves a register file, as with the core's Cpu.
unsafe impl Send for Cpu {}

/// The saved state of one guest; the same type, named as in heirloom-cpu-ppc.
pub type CpuContext = Cpu;

impl Cpu {
    /// A reset CPU that will execute at `entry` with r1 = `sp`; everything else zero.
    pub fn new(entry: u32, sp: u32) -> Cpu {
        let mut gpr = [0u32; 32];
        gpr[1] = sp;
        Cpu {
            gpr, fpr: [0; 32], cr: 0, lr: 0, ctr: 0, xer: 0, fpscr: 0, pc: entry, tb: 0, reservation: None,
            natives: Arc::new(BTreeSet::new()), natives_gen: 0, redirected: false, halted: false, blocked: false, trace_cap: 0,
            last: ptr::null_mut(),
        }
    }
    pub fn reg(&self, r: Reg) -> u32 {
        match r {
            Reg::Gpr(n) => self.gpr[(n & 31) as usize],
            Reg::Pc => self.pc, Reg::Lr => self.lr, Reg::Ctr => self.ctr,
            Reg::Xer => self.xer, Reg::Cr => self.cr, Reg::Fpscr => self.fpscr,
        }
    }
    /// As the core's; FPSCR is stored as given (the core normalises it at the next run, as its own `set_reg` would).
    pub fn set_reg(&mut self, r: Reg, v: u32) {
        match r {
            Reg::Gpr(n) => self.gpr[(n & 31) as usize] = v,
            Reg::Pc => self.set_pc(v), Reg::Lr => self.lr = v, Reg::Ctr => self.ctr = v,
            Reg::Xer => self.xer = v & XER_MASK, Reg::Cr => self.cr = v, Reg::Fpscr => self.fpscr = v,
        }
    }
    pub fn fpr_f64(&self, n: usize) -> f64 { f64::from_bits(self.fpr[n & 31]) }
    pub fn set_fpr_f64(&mut self, n: usize, v: f64) { self.fpr[n & 31] = v.to_bits() }
    /// Continue at `pc`. Inside a host callback this REPLACES the default continuation (LR for a native hook).
    pub fn set_pc(&mut self, pc: u32) { self.pc = pc; self.redirected = true; }
    /// Bind `addr` as native glue: reaching it calls [`Host::native`] instead of executing an instruction.
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
        static NEXT: AtomicU64 = AtomicU64::new(1);
        self.natives_gen = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    pub fn is_native(&self, addr: u32) -> bool { self.natives.contains(&(addr & !3)) }
    /// Stop after the current instruction; the run is over.
    pub fn halt(&mut self) { self.halted = true }
    /// Stop after the current instruction but leave the guest resumable (a host call that must wait).
    pub fn block(&mut self) { self.blocked = true }
    /// Keep the program counters of the last `n` executed instructions (0 turns it off); the library keeps the ring.
    pub fn trace_pcs(&mut self, n: usize) { self.trace_cap = n; }
    /// The recorded program counters, oldest first (the faulting instruction is the last).
    pub fn recent_pcs(&self) -> impl Iterator<Item = u32> + '_ {
        let mut out = alloc::vec![0u32; self.trace_cap];
        let mut n = 0u32;
        if let (Some(t), false) = (table(), self.last.is_null()) {
            (t.recent_pcs)(self.last, out.as_mut_ptr(), out.len() as u32, &mut n);
        }
        out.truncate(n as usize);
        out.into_iter()
    }
    pub fn cr_field(&self, n: u32) -> u32 { heirloom_cpu_abi::ppc::cr_field(self.cr, n) }
    pub fn set_cr_field(&mut self, n: u32, v: u32) { heirloom_cpu_abi::ppc::set_cr_field(&mut self.cr, n, v) }
    pub fn cr_bit(&self, i: u32) -> bool { heirloom_cpu_abi::ppc::cr_bit(self.cr, i) }
    pub fn set_cr_bit(&mut self, i: u32, v: bool) { heirloom_cpu_abi::ppc::set_cr_bit(&mut self.cr, i, v) }

    fn publish(&self, ctx: &mut PpcContext) {
        let r = &mut ctx.regs;
        (r.gpr, r.fpr, r.cr, r.lr, r.ctr, r.xer, r.fpscr, r.pc, r.tb) = (self.gpr, self.fpr, self.cr, self.lr, self.ctr, self.xer, self.fpscr, self.pc, self.tb);
        ctx.reservation.valid = self.reservation.is_some() as u32;
        ctx.reservation.address = self.reservation.unwrap_or(0);
    }
    fn take_back(&mut self, ctx: &PpcContext) {
        let r = &ctx.regs;
        (self.gpr, self.fpr, self.cr, self.lr, self.ctr, self.xer, self.fpscr, self.pc, self.tb) = (r.gpr, r.fpr, r.cr, r.lr, r.ctr, r.xer, r.fpscr, r.pc, r.tb);
        self.reservation = (ctx.reservation.valid != 0).then_some(ctx.reservation.address);
    }
}

/// The guest side the CPU runs against (the core's trait, with this crate's [`Cpu`]).
pub trait Host {
    fn mem(&mut self) -> &mut GuestMem;
    /// Execution reached an address bound with [`Cpu::bind_native`]. Return true if handled: the guest then returns through LR
    /// unless the handler called [`Cpu::set_pc`]. False stops the run with [`Trap::NativeUnhandled`].
    fn native(&mut self, _cpu: &mut Cpu, _addr: u32) -> bool { false }
    /// An `sc` instruction. Return true if handled: execution continues after it. False stops the run with [`Trap::Syscall`].
    fn syscall(&mut self, _cpu: &mut Cpu) -> bool { false }
}

/// What a callback needs during one run: the host (type-erased) and the run's Cpu.
struct Slot<'a> {
    host: &'a mut dyn Host,
    cpu: *mut Cpu,
    inst: *mut CpuInstance,
    bound: *mut BoundSet,
}

/// The natives the instance has bound, and the Cpu generation they were synced from.
#[derive(Default)]
struct BoundSet { set: BTreeSet<u32>, gen: Option<u64> }

impl BoundSet {
    fn sync(&mut self, t: &PpcVTable, inst: *mut CpuInstance, cpu: &Cpu) {
        if self.gen == Some(cpu.natives_gen) { return; }
        let gone: Vec<u32> = self.set.difference(&cpu.natives).copied().collect();
        for a in gone { (t.unbind_native)(inst, a); }
        for &a in cpu.natives.difference(&self.set) { (t.bind_native)(inst, a); }
        self.set = (*cpu.natives).clone();
        self.gen = Some(cpu.natives_gen);
    }
}

fn callback(ctx: *mut c_void, cpu_ctx: *mut PpcContext, native: Option<u32>) -> i32 {
    // SAFETY: `ctx` is the Slot on resume's stack, live for the whole run; `cpu_ctx` the run's context. The library holds no
    // reference to either while the callback runs.
    let (s, c) = unsafe { (&mut *(ctx as *mut Slot), &mut *cpu_ctx) };
    // SAFETY: `s.cpu` / `s.bound` point at resume's `&mut Cpu` / Exec field, not otherwise borrowed during the run.
    let (cpu, bound) = unsafe { (&mut *s.cpu, &mut *s.bound) };
    cpu.take_back(c);
    cpu.redirected = false;
    cpu.blocked = false;
    let handled = match native { Some(a) => s.host.native(cpu, a), None => s.host.syscall(cpu) };
    if let Some(t) = table() { bound.sync(t, s.inst, cpu); }
    cpu.publish(c);
    if !handled { HOST_UNHANDLED } else if cpu.halted { HOST_HALT } else if cpu.blocked { HOST_BLOCK } else if cpu.redirected { HOST_REDIRECTED } else { HOST_HANDLED }
}
extern "C" fn on_native(ctx: *mut c_void, cpu: *mut PpcContext, addr: u32) -> i32 { callback(ctx, cpu, Some(addr)) }
extern "C" fn on_syscall(ctx: *mut c_void, cpu: *mut PpcContext, _pc: u32) -> i32 { callback(ctx, cpu, None) }

/// A runner: the selected tier, its counters, and the ONE library instance it drives. One per guest machine; swapping [`Cpu`]
/// values under it (threads, Mixed Mode frames) is fine.
pub struct Exec {
    pub tier: Tier,
    pub prof: Prof,
    inst: *mut CpuInstance,
    host: Box<PpcHostCallbacks>,
    bound: BoundSet,
    tier_set: Option<u32>,
    lockstep_refused: bool,
}

// SAFETY: the instance and the callbacks box are owned by this Exec alone and used only through `&mut self`; moving the Exec moves
// that sole use to the new thread (the library has no thread affinity and no globals).
unsafe impl Send for Exec {}

impl Default for Exec { fn default() -> Self { Exec::new(Tier::Blocks) } }

impl Drop for Exec {
    fn drop(&mut self) {
        if let (Some(t), false) = (table(), self.inst.is_null()) { (t.destroy)(self.inst); }
    }
}

/// The run that ran nothing: a [`Trap::Refused`] naming why.
fn refused(why: &'static str) -> RunStats { RunStats { insns: 0, halted: false, blocked: false, trap: Some(Trap::Refused { why }) } }

impl Exec {
    pub fn new(tier: Tier) -> Exec {
        let host = Box::new(PpcHostCallbacks { size: core::mem::size_of::<PpcHostCallbacks>() as u32, flags: 0, ctx: ptr::null_mut(), on_native, on_syscall });
        Exec { tier, prof: Prof::default(), inst: ptr::null_mut(), host, bound: BoundSet::default(), tier_set: None, lockstep_refused: false }
    }
    /// True once a resume was asked for [`Tier::Lockstep`]: cpu-ppc.library does not serve it (the static core does), so the run
    /// used tier 1 - named here, never silent.
    pub fn lockstep_refused(&self) -> bool { self.lockstep_refused }

    /// Execute `cpu` until [`Cpu::halt`], [`Cpu::block`], a trap, or `budget` instructions.
    pub fn resume<H: Host>(&mut self, host: &mut H, cpu: &mut Cpu, budget: u64) -> RunStats {
        let Some(t) = table() else { return refused(REFUSED_UNBOUND) };
        if self.inst.is_null() {
            let Some(ram) = host.mem().flat_ram() else { return refused(REFUSED_NO_FLAT_RAM) };
            let cfg = PpcCreate { size: core::mem::size_of::<PpcCreate>() as u32, sub_arch: SUB_ARCH_G3, ram, host: &*self.host };
            if (t.create)(&cfg, &mut self.inst) != 0 { self.inst = ptr::null_mut(); return refused(REFUSED_CREATE); }
        }
        let tier = match self.tier { Tier::Interp => TIER_INTERP, Tier::Blocks => TIER_BLOCKS, Tier::Lockstep => { self.lockstep_refused = true; TIER_BLOCKS } };
        if self.tier_set != Some(tier) { (t.set_tier)(self.inst, tier); self.tier_set = Some(tier); }
        self.bound.sync(t, self.inst, cpu);
        if cpu.trace_cap != 0 || !cpu.last.is_null() { (t.trace_pcs)(self.inst, cpu.trace_cap as u32); }
        cpu.last = self.inst;
        let mut ctx = PpcContext::new(cpu.pc, 0);
        cpu.publish(&mut ctx);
        ctx.state = if cpu.halted { CTX_HALTED } else { CTX_RUNNABLE };
        cpu.blocked = false;
        let mut slot = Slot { host, cpu: cpu as *mut Cpu, inst: self.inst, bound: &mut self.bound };
        self.host.ctx = (&mut slot as *mut Slot).cast();
        let mut exit = PpcExit::default();
        (t.run)(self.inst, &mut ctx, budget, &mut exit);
        self.host.ctx = ptr::null_mut();
        cpu.take_back(&ctx);
        let mut st = RunStats { insns: exit.retired, halted: false, blocked: false, trap: None };
        match exit.code {
            EXIT_BUDGET => {}
            EXIT_HALT => { cpu.halted = true; st.halted = true; }
            EXIT_BLOCK => { cpu.blocked = true; st.blocked = true; }
            EXIT_TRAP => st.trap = Some(exit.decode_trap().unwrap_or(Trap::Unimplemented { pc: exit.pc, word: exit.word, what: WHAT_UNLISTED })),
            _ => st.trap = Some(Trap::Refused { why: refusal_of(&exit) }),
        }
        let mut p = PpcProf::default();
        if (t.read_prof)(self.inst, &mut p) == 0 { (self.prof.natives, self.prof.blocks) = (p.natives, p.blocks); }
        self.prof.insns += st.insns;
        st
    }
}

/// Execute a fresh tier-0 run of `cpu` (the core's free `resume`).
pub fn resume<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64) -> RunStats { Exec::new(Tier::Interp).resume(host, cpu, budget) }

#[cfg(test)]
mod tests;
