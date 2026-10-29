//! heirloom-cpu-arm's API - [`Cpu`] with its public register file, `run` / `step` / `run_counted`, and the shared types
//! ([`Exit`], [`Memory`], [`Fault`], ...) - driven through cpu-arm.library's [`CpuVTable`] instead of a linked core. An engine
//! names its CPU through one module and gets the same program behaviour from either backend (doctrine ONE-NAME-ONE-CONCERN-LOADED;
//! docs/heirloom/plan.tsv row cpu-arm).
//!
//! How a run crosses the table: the [`Cpu`] here is the thread's register file (one per guest thread, cloned freely, as with
//! the core). [`Cpu::run_counted`] makes a library instance whose bus is the caller's [`Memory`] (through a record on the
//! caller's stack), loads the thread into it (`load_context`, an [`ArmContext`](heirloom_cpu_abi::arm::ArmContext)), runs,
//! reads the state and the exit record back (`save_context`) and destroys it. No state is shared between runs, so nothing is
//! locked; the cost is one small allocation and one context copy each way per run, and the engine runs a slice per call, so
//! that is per SVC, not per instruction.
//!
//! A fault is reported with the [`Fault`] the caller's memory returned (its address may differ from the one the core asked
//! for, e.g. a page boundary inside a word), not the core's reconstruction of it.
#![cfg_attr(not(test), no_std)]
// `bind-static` exists for HOST tests and host tools only: it links the static core so a test can bind its table. A CoralOS
// build (target_os = "none") opens cpu-arm.library instead, and linking the core there would be the CPU twice (doctrine
// LIBRARY-NOT-LINKED-TWICE) - refused at compile time, not by convention.
#[cfg(all(feature = "bind-static", target_os = "none"))]
compile_error!("heirloom-cpu-arm-client: `bind-static` is the host-test binding; a CoralOS engine binds the CpuVTable it opened from cpu-arm.library");

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use heirloom_cpu_abi::arm::{ArmBusCallbacks, ArmContext, ArmExit, Psr, SUB_ARCH_ARMV6};
use heirloom_cpu_abi::{check, CpuBusCallbacks, CpuInstance, CpuStop, CpuVTable, OpenRefusal, ARCH_ARM, CPU_ABI_VERSION};

pub use heirloom_cpu_abi::arm::{Access, Config, Exit, Fault, Memory, Trap, TrapKind, Unaligned, Vfp};

pub enum BindRefusal {
    Open(OpenRefusal),
    AlreadyBound,
}

impl core::fmt::Debug for BindRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BindRefusal::Open(OpenRefusal::NullTable) => write!(f, "Open(NullTable)"),
            BindRefusal::Open(OpenRefusal::VersionTooOld { found, required }) => write!(f, "Open(VersionTooOld {{ found: {found}, required: {required} }})"),
            BindRefusal::Open(OpenRefusal::TableTooSmall { found, required }) => write!(f, "Open(TableTooSmall {{ found: {found}, required: {required} }})"),
            BindRefusal::Open(OpenRefusal::WrongArch { found, required }) => write!(f, "Open(WrongArch {{ found: {found}, required: {required} }})"),
            BindRefusal::AlreadyBound => write!(f, "AlreadyBound"),
        }
    }
}

static VTABLE: AtomicPtr<CpuVTable> = AtomicPtr::new(ptr::null_mut());

/// Where one run's bus callbacks find the caller's memory: on the caller's stack, for the length of that run. Each run makes
/// its own library instance over it (the context carries the whole thread), so there is no shared state to lock.
struct BusSlot<'a> {
    mem: &'a mut dyn Memory,
    fault: Option<Fault>,
}

/// Bind cpu-arm.library's table (the engine does this once, with the table `coral_lib_open` returned, before any guest runs).
/// Binding the same table again is fine; a different one is refused.
pub fn bind(table: &'static CpuVTable) -> Result<(), BindRefusal> {
    check(table, CPU_ABI_VERSION, ARCH_ARM).map_err(BindRefusal::Open)?;
    let new_ptr = table as *const CpuVTable as *mut CpuVTable;
    match VTABLE.compare_exchange(ptr::null_mut(), new_ptr, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(()),
        Err(current) if current == new_ptr => Ok(()),
        Err(_) => Err(BindRefusal::AlreadyBound),
    }
}

pub fn is_bound() -> bool {
    !VTABLE.load(Ordering::Acquire).is_null()
}

fn table() -> &'static CpuVTable {
    let p = VTABLE.load(Ordering::Acquire);
    #[cfg(feature = "bind-static")]
    if p.is_null() {
        let _ = bind(heirloom_cpu_arm::vtable());
        return table();
    }
    // SAFETY: only `bind` stores, and only a checked &'static table.
    match unsafe { p.as_ref() } {
        Some(t) => t,
        None => panic!("heirloom-cpu-arm-client: cpu-arm.library is not bound (call bind with the library's CpuVTable first)"),
    }
}

fn with_mem<T>(ctx: *mut c_void, f: impl FnOnce(&mut dyn Memory) -> Result<T, Fault>) -> Result<T, ()> {
    // SAFETY: ctx is the BusSlot run_counted put in this run's bus record; it outlives the instance that calls back with it.
    let s = unsafe { &mut *(ctx as *mut BusSlot<'_>) };
    f(&mut *s.mem).map_err(|fault| s.fault = Some(fault))
}

fn out(r: Result<u32, ()>, dst: *mut u32) -> i32 {
    match r {
        Ok(v) => {
            // SAFETY: the core passes its own local.
            unsafe { *dst = v };
            0
        }
        Err(()) => 1,
    }
}

extern "C" fn r8(ctx: *mut c_void, addr: u32, dst: *mut u32) -> i32 {
    out(with_mem(ctx, |m| m.read_u8(addr).map(u32::from)), dst)
}
extern "C" fn r16(ctx: *mut c_void, addr: u32, dst: *mut u32) -> i32 {
    out(with_mem(ctx, |m| m.read_u16(addr).map(u32::from)), dst)
}
extern "C" fn r32(ctx: *mut c_void, addr: u32, dst: *mut u32) -> i32 {
    out(with_mem(ctx, |m| m.read_u32(addr)), dst)
}
extern "C" fn f16(ctx: *mut c_void, addr: u32, dst: *mut u32) -> i32 {
    out(with_mem(ctx, |m| m.fetch_u16(addr).map(u32::from)), dst)
}
extern "C" fn f32_(ctx: *mut c_void, addr: u32, dst: *mut u32) -> i32 {
    out(with_mem(ctx, |m| m.fetch_u32(addr)), dst)
}
extern "C" fn w8(ctx: *mut c_void, addr: u32, v: u32) -> i32 {
    with_mem(ctx, |m| m.write_u8(addr, v as u8)).map_or(1, |_| 0)
}
extern "C" fn w16(ctx: *mut c_void, addr: u32, v: u32) -> i32 {
    with_mem(ctx, |m| m.write_u16(addr, v as u16)).map_or(1, |_| 0)
}
extern "C" fn w32(ctx: *mut c_void, addr: u32, v: u32) -> i32 {
    with_mem(ctx, |m| m.write_u32(addr, v)).map_or(1, |_| 0)
}
/// Never called for ARCH_ARM (an SVC stops the run); refuses if a library did.
extern "C" fn no_trap(_ctx: *mut c_void, _pc: u32, _op: u32) -> i32 {
    1
}

fn bus(slot: &mut BusSlot<'_>) -> ArmBusCallbacks {
    ArmBusCallbacks {
        base: CpuBusCallbacks {
            ctx: slot as *mut BusSlot<'_> as *mut c_void,
            read8: r8,
            read16: r16,
            read32: r32,
            write8: w8,
            write16: w16,
            write32: w32,
            on_trap: no_trap,
        },
        fetch16: f16,
        fetch32: f32_,
    }
}

/// One run's library instance, destroyed when the run ends however it ends.
struct Instance<'t> {
    table: &'t CpuVTable,
    inst: *mut CpuInstance,
}

impl Drop for Instance<'_> {
    fn drop(&mut self) {
        (self.table.destroy)(self.inst);
    }
}

/// One ARMv6 hardware thread in user mode: the register file, as with the linked core. Its fields are the core's.
#[derive(Clone, Debug)]
pub struct Cpu {
    /// R0-R15. `regs[15]` is the address of the next instruction to execute (not PC+8).
    pub regs: [u32; 16],
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub q: bool,
    pub ge: u8,
    pub thumb: bool,
    pub big_endian: bool,
    pub vfp: Vfp,
    pub tpidrurw: u32,
    pub tpidruro: u32,
    pub config: Config,
    /// The local exclusive monitor (address, size), kept with the thread like the core keeps it.
    exclusive: Option<(u32, u8)>,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new(Config::default())
    }
}

impl Cpu {
    pub fn new(config: Config) -> Self {
        Cpu {
            regs: [0; 16],
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            ge: 0,
            thumb: false,
            big_endian: false,
            vfp: Vfp::default(),
            tpidrurw: 0,
            tpidruro: 0,
            config,
            exclusive: None,
        }
    }

    pub fn pc(&self) -> u32 {
        self.regs[15]
    }

    /// Branch the CPU to `addr`; bit 0 selects Thumb, as BX does.
    pub fn set_pc_interworking(&mut self, addr: u32) {
        self.thumb = addr & 1 != 0;
        self.regs[15] = if self.thumb { addr & !1 } else { addr & !3 };
    }

    fn psr(&self) -> Psr {
        Psr { n: self.n, z: self.z, c: self.c, v: self.v, q: self.q, ge: self.ge, big_endian: self.big_endian, thumb: self.thumb }
    }

    pub fn cpsr(&self) -> u32 {
        self.psr().pack()
    }

    pub fn set_cpsr(&mut self, value: u32) {
        let p = Psr::unpack(value);
        (self.n, self.z, self.c, self.v, self.q, self.ge, self.big_endian, self.thumb) = (p.n, p.z, p.c, p.v, p.q, p.ge, p.big_endian, p.thumb);
    }

    pub fn clear_exclusive(&mut self) {
        self.exclusive = None;
    }

    fn context(&self) -> ArmContext {
        let (exclusive_addr, exclusive_size) = match self.exclusive { Some((a, s)) => (a, s as u32), None => (0, 0) };
        ArmContext {
            regs: self.regs,
            cpsr: self.cpsr(),
            vfp_s: self.vfp.s,
            fpscr: self.vfp.fpscr,
            tpidrurw: self.tpidrurw,
            tpidruro: self.tpidruro,
            unaligned: self.config.unaligned.to_raw(),
            vfp: self.config.vfp as u32,
            exclusive_addr,
            exclusive_size,
            exit: ArmExit::default(),
        }
    }

    fn take_context(&mut self, ctx: &ArmContext) {
        self.regs = ctx.regs;
        self.set_cpsr(ctx.cpsr);
        self.vfp.s = ctx.vfp_s;
        self.vfp.fpscr = ctx.fpscr;
        self.tpidrurw = ctx.tpidrurw;
        self.tpidruro = ctx.tpidruro;
        self.exclusive = match ctx.exclusive_size { 0 => None, s => Some((ctx.exclusive_addr, s as u8)) };
    }

    /// Execute up to `budget` instructions, saying how many retired (see heirloom-cpu-arm's `Cpu::run_counted`).
    pub fn run_counted<M: Memory>(&mut self, mem: &mut M, budget: u64) -> (Exit, u64) {
        let t = table();
        let mut slot = BusSlot { mem, fault: None };
        let record = bus(&mut slot);
        let created = (t.create)(SUB_ARCH_ARMV6, &record as *const ArmBusCallbacks as *const CpuBusCallbacks);
        if created.is_null() {
            panic!("cpu-arm.library refused create (sub_arch {SUB_ARCH_ARMV6})");
        }
        let inst = Instance { table: t, inst: created };
        let size = core::mem::size_of::<ArmContext>() as u32;
        if (t.context_size)(inst.inst) < size {
            panic!("cpu-arm.library's context is smaller than heirloom-cpu-abi::arm::ArmContext");
        }
        let mut ctx = self.context();
        if (t.load_context)(inst.inst, (&ctx as *const ArmContext).cast(), size) != 0 {
            panic!("cpu-arm.library refused load_context (a configuration or reservation no core has)");
        }
        let mut stop = CpuStop { kind: 0, pc: 0, detail: 0, bus_faults: 0, insns: 0 };
        if (t.run)(inst.inst, budget, &mut stop) != 0 {
            panic!("cpu-arm.library refused run");
        }
        if (t.save_context)(inst.inst, (&mut ctx as *mut ArmContext).cast(), size) != 0 {
            panic!("cpu-arm.library refused save_context");
        }
        drop(inst);
        self.take_context(&ctx);
        let exit = match ctx.exit.decode() {
            Some(Exit::Fault { pc, fault }) => Exit::Fault { pc, fault: slot.fault.unwrap_or(fault) },
            Some(e) => e,
            None => panic!("cpu-arm.library wrote an exit record no core writes: {:?}", ctx.exit),
        };
        (exit, stop.insns)
    }

    pub fn run<M: Memory>(&mut self, mem: &mut M, budget: u64) -> Exit {
        self.run_counted(mem, budget).0
    }

    pub fn step<M: Memory>(&mut self, mem: &mut M) -> Exit {
        self.run(mem, 1)
    }
}

#[cfg(test)]
mod tests;
