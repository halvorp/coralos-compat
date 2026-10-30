//! The core behind heirloom-cpu-abi's PowerPC flat face ([`PpcVTable`], CPU ABI v3): what cpu-ppc.library's `coral_lib_open`
//! returns. The records and their rules are heirloom-cpu-abi::ppc's; in short: the ENGINE owns the guest RAM (a [`FlatRam`]
//! descriptor + its code watch) and every register context, and `run` executes ON the context it is handed, calling the engine
//! back only at a native hook or an `sc`.
//!
//! An instance keeps what is not architectural: the bound natives, the trace ring and the [`Exec`] (tier, counters and tier 1's
//! decoded-block cache, which is the point of keeping it). Its core [`Cpu`]'s registers are overwritten from the context at every
//! run entry and published back at every exit and before every callback, so the instance caches no register state.
//!
//! Re-entrancy: the engine may call `bind_native` / `unbind_native` from inside a callback. While a run is in progress the core
//! Cpu and the Exec are MOVED OUT of the instance into the run's locals, so no `&mut Instance` is live across a callback; a bind
//! made then is queued in `pending` and applied to the run's Cpu as soon as the callback returns.

use alloc::boxed::Box;
use alloc::vec::Vec;
use heirloom_cpu_abi::ppc::{
    FlatRam, PpcContext, PpcCreate, PpcExit, PpcHostCallbacks, PpcProf, PpcVTable, ABI_FAMILY_PPC_FLAT, CTX_HALTED, HOST_BLOCK,
    HOST_HALT, HOST_HANDLED, HOST_REDIRECTED, HOST_UNHANDLED, LAYOUT, PPC_ABI_VERSION, REFUSE_CONTEXT, REFUSE_INSTANCE, REFUSE_RAM,
    SUB_ARCH_G3, TIER_BLOCKS, TIER_INTERP,
};
use heirloom_cpu_abi::{CpuInstance, ARCH_PPC};
use heirloom_mem_flat32::GuestMem;

use crate::{Cpu, Exec, Host, Tier};

struct Instance {
    ram: *const FlatRam,
    host: *const PpcHostCallbacks,
    /// `None` while a run has moved it out.
    cpu: Option<Cpu>,
    exec: Option<Exec>,
    /// Binds (true) / unbinds (false) asked for during a run, applied after the callback that asked.
    pending: Vec<(u32, bool)>,
    running: bool,
}

fn instance<'a>(inst: *mut CpuInstance) -> Option<&'a mut Instance> {
    // SAFETY: a non-null handle is a Box<Instance> leaked by `create` and not yet destroyed (the table's contract). During a run the
    // run holds no reference into it (its Cpu and Exec are moved out), so this is the only live reference.
    unsafe { inst.cast::<Instance>().as_mut() }
}

/// The context's registers into the core's register file.
fn load(cpu: &mut Cpu, ctx: &PpcContext) {
    let r = &ctx.regs;
    (cpu.gpr, cpu.fpr, cpu.cr, cpu.lr, cpu.ctr, cpu.xer, cpu.fpscr, cpu.pc, cpu.tb) = (r.gpr, r.fpr, r.cr, r.lr, r.ctr, r.xer, r.fpscr, r.pc, r.tb);
    cpu.reservation = (ctx.reservation.valid != 0).then_some(ctx.reservation.address);
}

/// The core's register file published into the context, complete.
fn store(cpu: &Cpu, ctx: &mut PpcContext) {
    let r = &mut ctx.regs;
    (r.gpr, r.fpr, r.cr, r.lr, r.ctr, r.xer, r.fpscr, r.pc, r.tb) = (cpu.gpr, cpu.fpr, cpu.cr, cpu.lr, cpu.ctr, cpu.xer, cpu.fpscr, cpu.pc, cpu.tb);
    ctx.reservation.valid = cpu.reservation.is_some() as u32;
    ctx.reservation.address = cpu.reservation.unwrap_or(0);
}

/// The run's Host: a view of the engine's RAM (remade after every callback, which may have grown it) and the engine's callbacks.
struct Adapter {
    inst: *mut CpuInstance,
    ram: *const FlatRam,
    host: *const PpcHostCallbacks,
    ctx: *mut PpcContext,
    view: GuestMem,
    /// A callback answered a code that is none of HOST_*: (native address, code).
    protocol: Option<(u32, i32)>,
}

impl Adapter {
    fn view(ram: *const FlatRam) -> GuestMem {
        // SAFETY: `ram` was validated at create/run and the engine keeps it live and unmoved while no callback runs; the view is
        // remade after every callback, the only time the engine may move it.
        unsafe { GuestMem::view(ram) }
    }

    /// Publish `cpu`, call the engine, take its registers back, apply the binds it asked for, remake the RAM view.
    fn callback(&mut self, cpu: &mut Cpu, call: impl FnOnce(&PpcHostCallbacks, *mut PpcContext) -> i32) -> i32 {
        // SAFETY: `ctx` is the caller's context, valid for the whole run (checked at entry); no other reference to it is live here.
        store(cpu, unsafe { &mut *self.ctx });
        // SAFETY: `host` was validated at create and outlives the instance (PpcCreate's contract).
        let code = call(unsafe { &*self.host }, self.ctx);
        // SAFETY: as above; the callback has returned, so the engine no longer writes the context.
        load(cpu, unsafe { &*self.ctx });
        if let Some(i) = instance(self.inst) {
            for (addr, bind) in core::mem::take(&mut i.pending) {
                if bind { cpu.bind_native(addr) } else { cpu.unbind_native(addr); }
            }
        }
        self.view = Self::view(self.ram);
        code
    }

    /// Map a callback's code onto the core; `at` is where execution reached the native (or the `sc`).
    fn outcome(&mut self, cpu: &mut Cpu, code: i32, at: u32) -> bool {
        match code {
            HOST_HANDLED => true,
            HOST_REDIRECTED => { let pc = cpu.pc; cpu.set_pc(pc); true }
            HOST_HALT => { cpu.halt(); true }
            // pc stays where the engine left it (the engine parks ON the call: pc = the native), so the next run calls it again
            HOST_BLOCK => { let pc = cpu.pc; cpu.set_pc(pc); cpu.block(); true }
            HOST_UNHANDLED => false,
            other => { self.protocol = Some((at, other)); cpu.set_pc(at); cpu.halt(); true }
        }
    }
}

impl Host for Adapter {
    fn mem(&mut self) -> &mut GuestMem { &mut self.view }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        cpu.pc = addr;
        let code = self.callback(cpu, |h, ctx| (h.on_native)(h.ctx, ctx, addr));
        self.outcome(cpu, code, addr)
    }
    fn syscall(&mut self, cpu: &mut Cpu) -> bool {
        let pc = cpu.pc;
        let code = self.callback(cpu, |h, ctx| (h.on_syscall)(h.ctx, ctx, pc));
        // HANDLED continues after the `sc` (the core's own continuation); the others as for a native
        if code == HOST_HANDLED { cpu.pc = pc; return true; }
        self.outcome(cpu, code, pc)
    }
}

fn ram_ok(ram: &FlatRam) -> bool {
    ram.size == core::mem::size_of::<FlatRam>() as u32 && !ram.watch.is_null() && (ram.len == 0 || !ram.base.is_null())
}

extern "C" fn create(cfg: *const PpcCreate, out: *mut *mut CpuInstance) -> i32 {
    // SAFETY: a non-null cfg is a PpcCreate the caller owns for this call (the table's contract).
    let Some(c) = (unsafe { cfg.as_ref() }) else { return -1 };
    if out.is_null() || c.size != core::mem::size_of::<PpcCreate>() as u32 || c.sub_arch != SUB_ARCH_G3 { return -1; }
    // SAFETY: non-null record pointers the caller promises are live for the instance's lifetime (PpcCreate's contract).
    let (Some(ram), Some(host)) = (unsafe { c.ram.as_ref() }, unsafe { c.host.as_ref() }) else { return -1 };
    if !ram_ok(ram) || host.size != core::mem::size_of::<PpcHostCallbacks>() as u32 { return -1; }
    let i = Box::new(Instance { ram: c.ram, host: c.host, cpu: Some(Cpu::new(0, 0)), exec: Some(Exec::default()), pending: Vec::new(), running: false });
    // SAFETY: `out` is non-null and writable (the table's contract).
    unsafe { *out = Box::into_raw(i).cast() };
    0
}

extern "C" fn destroy(inst: *mut CpuInstance) -> i32 {
    if inst.is_null() { return -1; }
    // SAFETY: a non-null handle is the Box `create` leaked; the caller destroys it once and not during a run.
    drop(unsafe { Box::from_raw(inst.cast::<Instance>()) });
    0
}

extern "C" fn run(inst: *mut CpuInstance, cpu_ctx: *mut PpcContext, budget: u64, exit: *mut PpcExit) -> i32 {
    // SAFETY: a non-null exit is the caller's record for this call.
    let Some(exit) = (unsafe { exit.as_mut() }) else { return -1 };
    let (ram, host, mut cpu, mut exec) = match instance(inst) {
        Some(i) if i.running => { *exit = PpcExit::refused(REFUSE_INSTANCE); return 0; } // a run from inside a callback
        Some(i) => match (i.cpu.take(), i.exec.take()) {
            (Some(c), Some(e)) => { i.running = true; (i.ram, i.host, c, e) }
            _ => { *exit = PpcExit::refused(REFUSE_INSTANCE); return 0; }
        },
        None => { *exit = PpcExit::refused(REFUSE_INSTANCE); return 0; }
    };
    let finish = |cpu: Cpu, exec: Exec| if let Some(i) = instance(inst) { (i.cpu, i.exec, i.running) = (Some(cpu), Some(exec), false); };
    // SAFETY: a non-null context is the caller's for the whole call.
    let ctx_ok = unsafe { cpu_ctx.as_ref() }.is_some_and(|c| c.size == core::mem::size_of::<PpcContext>() as u32);
    // SAFETY: `ram` was validated at create; the engine keeps it live (it may have grown since: re-validated here).
    let ram_now = unsafe { ram.as_ref() }.is_some_and(ram_ok);
    if !ctx_ok || !ram_now {
        *exit = PpcExit::refused(if ctx_ok { REFUSE_RAM } else { REFUSE_CONTEXT });
        finish(cpu, exec);
        return 0;
    }
    // SAFETY: checked non-null and of the right size above.
    let ctx = unsafe { &mut *cpu_ctx };
    if ctx.state == CTX_HALTED {
        *exit = PpcExit::halt(ctx.regs.pc, 0);
        finish(cpu, exec);
        return 0;
    }
    load(&mut cpu, ctx);
    cpu.clear_halted();
    let mut a = Adapter { inst, ram, host, ctx: cpu_ctx, view: Adapter::view(ram), protocol: None };
    let st = exec.resume(&mut a, &mut cpu, budget);
    // SAFETY: as above; no callback runs any more.
    let ctx = unsafe { &mut *cpu_ctx };
    store(&cpu, ctx);
    *exit = match (a.protocol, st.trap) {
        (Some((at, code)), _) => PpcExit::host_protocol(at, code, st.insns),
        (None, Some(t)) => PpcExit::trap(&t, st.insns),
        (None, None) if st.halted => { ctx.state = CTX_HALTED; PpcExit::halt(cpu.pc, st.insns) }
        (None, None) if st.blocked => PpcExit::block(cpu.pc, st.insns),
        (None, None) => PpcExit::budget(cpu.pc, st.insns),
    };
    finish(cpu, exec);
    0
}

extern "C" fn bind_native(inst: *mut CpuInstance, addr: u32) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    match i.cpu.as_mut() { Some(c) => c.bind_native(addr), None => i.pending.push((addr, true)) }
    0
}

extern "C" fn unbind_native(inst: *mut CpuInstance, addr: u32) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    match i.cpu.as_mut() { Some(c) => c.unbind_native(addr) as i32, None => { i.pending.push((addr, false)); 0 } }
}

extern "C" fn set_tier(inst: *mut CpuInstance, tier: u32) -> i32 {
    let Some(e) = instance(inst).and_then(|i| i.exec.as_mut()) else { return -1 };
    e.tier = match tier { TIER_INTERP => Tier::Interp, TIER_BLOCKS => Tier::Blocks, _ => return -1 };
    0
}

extern "C" fn read_prof(inst: *mut CpuInstance, out: *mut PpcProf) -> i32 {
    let (Some(e), false) = (instance(inst).and_then(|i| i.exec.as_ref()), out.is_null()) else { return -1 };
    let c = e.cache_stats();
    let p = PpcProf {
        size: core::mem::size_of::<PpcProf>() as u32, tier: e.tier.number(), insns: e.prof.insns, natives: e.prof.natives,
        blocks: e.prof.blocks, cache_hits: e.prof.blocks.saturating_sub(c.decodes), cache_misses: c.decodes,
        cache_invalidations: c.stale + c.flushes,
    };
    // SAFETY: `out` is non-null and the caller's record.
    unsafe { *out = p };
    0
}

extern "C" fn trace_pcs(inst: *mut CpuInstance, n: u32) -> i32 {
    let Some(c) = instance(inst).and_then(|i| i.cpu.as_mut()) else { return -1 };
    c.trace_pcs(n as usize);
    0
}

extern "C" fn recent_pcs(inst: *mut CpuInstance, out: *mut u32, cap: u32, written: *mut u32) -> i32 {
    let Some(c) = instance(inst).and_then(|i| i.cpu.as_ref()) else { return -1 };
    if written.is_null() || (out.is_null() && cap > 0) { return -1; }
    let mut n = 0u32;
    for pc in c.recent_pcs().take(cap as usize) {
        // SAFETY: `out` holds `cap` writable u32s (the caller's buffer) and n < cap.
        unsafe { *out.add(n as usize) = pc };
        n += 1;
    }
    // SAFETY: checked non-null.
    unsafe { *written = n };
    0
}

extern "C" fn flush_icache_range(inst: *mut CpuInstance, addr: u32, len: u32) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    if len == 0 { return 0; }
    let mut view = Adapter::view(i.ram);
    let (first, last) = (addr >> heirloom_mem_flat32::CODE_PAGE_SHIFT, addr.saturating_add(len - 1) >> heirloom_mem_flat32::CODE_PAGE_SHIFT);
    for p in first..=last { view.invalidate_code(p << heirloom_mem_flat32::CODE_PAGE_SHIFT); }
    0
}

static PPC_VTABLE: PpcVTable = PpcVTable {
    version: PPC_ABI_VERSION,
    size: core::mem::size_of::<PpcVTable>() as u32,
    arch_id: ARCH_PPC,
    abi_family: ABI_FAMILY_PPC_FLAT,
    features: 0,
    layout: LAYOUT,
    create,
    destroy,
    run,
    bind_native,
    unbind_native,
    set_tier,
    read_prof,
    trace_pcs,
    recent_pcs,
    flush_icache_range,
};

/// cpu-ppc.library's table.
pub fn vtable() -> &'static PpcVTable { &PPC_VTABLE }

/// NEGATIVE CONTROL ONLY: plant tier 1's stale-code mutant on an instance (tests prove the shared watch is what keeps it honest).
#[doc(hidden)]
pub fn plant_stale_code_mutant(inst: *mut CpuInstance) {
    if let Some(e) = instance(inst).and_then(|i| i.exec.as_mut()) { e.plant_stale_code_mutant(); }
}
