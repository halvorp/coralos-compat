//! The core behind heirloom-cpu-abi's flat [`CpuVTable`] (`ARCH_ARM`): what cpu-arm.library's `coral_lib_open` returns. The
//! ARM records (bus, context, exit) are heirloom-cpu-abi::arm's; see that module for why ARM differs from the 68k shape.
//!
//! Pure Rust, no globals: each `create` is an independent instance (one per engine is enough, since the context carries the
//! configuration and the engine switches guest threads with `load_context`).
//!
//! Register numbers for `get_reg` / `set_reg`: 0-15 = R0-R15 (R15 = the next instruction), [`REG_CPSR`], [`REG_FPSCR`],
//! [`REG_TPIDRURW`], [`REG_TPIDRURO`]. Anything else is refused (-1).

extern crate alloc;

use alloc::boxed::Box;
use core::ffi::c_void;
use heirloom_cpu_abi::arm::{ArmBusCallbacks, ArmContext, SUB_ARCH_ARMV6};
use heirloom_cpu_abi::{
    CpuInstance, CpuStop, CpuVTable, ARCH_ARM, CPU_ABI_VERSION, STOP_BUDGET, STOP_FAULT, STOP_HALT, STOP_TRAP,
};

use crate::{Access, Config, Cpu, Exit, Fault, Memory};

pub const REG_CPSR: u32 = heirloom_cpu_abi::regs::arm::CPSR;
pub const REG_FPSCR: u32 = 17;
pub const REG_TPIDRURW: u32 = 18;
pub const REG_TPIDRURO: u32 = 19;

struct Instance {
    bus: ArmBusCallbacks,
    cpu: Cpu,
    exit: Exit,
}

/// Field-by-field copy (the contract records derive nothing, and their layout must not change for that).
fn copy_bus(b: &ArmBusCallbacks) -> ArmBusCallbacks {
    let c = &b.base;
    ArmBusCallbacks {
        base: heirloom_cpu_abi::CpuBusCallbacks {
            ctx: c.ctx,
            read8: c.read8,
            read16: c.read16,
            read32: c.read32,
            write8: c.write8,
            write16: c.write16,
            write32: c.write32,
            on_trap: c.on_trap,
        },
        fetch16: b.fetch16,
        fetch32: b.fetch32,
    }
}

/// The instance behind a handle, or `None` for a null one. A handle is only ever one `create` returned.
fn instance<'a>(inst: *mut CpuInstance) -> Option<&'a mut Instance> {
    // SAFETY: a non-null handle is a Box<Instance> leaked by create_impl and not yet destroyed (the table's contract).
    unsafe { inst.cast::<Instance>().as_mut() }
}

/// The instance's bus as the core's [`Memory`]. A refused access is reported with the address the core asked for.
struct Bus<'a>(&'a ArmBusCallbacks);

impl Bus<'_> {
    fn read(&self, f: extern "C" fn(*mut c_void, u32, *mut u32) -> i32, addr: u32, access: Access) -> Result<u32, Fault> {
        let mut v = 0u32;
        match f(self.0.base.ctx, addr, &mut v) {
            0 => Ok(v),
            _ => Err(Fault { addr, access }),
        }
    }
    fn write(&self, f: extern "C" fn(*mut c_void, u32, u32) -> i32, addr: u32, value: u32) -> Result<(), Fault> {
        match f(self.0.base.ctx, addr, value) {
            0 => Ok(()),
            _ => Err(Fault { addr, access: Access::Write }),
        }
    }
}

impl Memory for Bus<'_> {
    fn read_u8(&mut self, addr: u32) -> Result<u8, Fault> {
        self.read(self.0.base.read8, addr, Access::Read).map(|v| v as u8)
    }
    fn read_u16(&mut self, addr: u32) -> Result<u16, Fault> {
        self.read(self.0.base.read16, addr, Access::Read).map(|v| v as u16)
    }
    fn read_u32(&mut self, addr: u32) -> Result<u32, Fault> {
        self.read(self.0.base.read32, addr, Access::Read)
    }
    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault> {
        self.write(self.0.base.write8, addr, value as u32)
    }
    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault> {
        self.write(self.0.base.write16, addr, value as u32)
    }
    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault> {
        self.write(self.0.base.write32, addr, value)
    }
    fn fetch_u16(&mut self, addr: u32) -> Result<u16, Fault> {
        self.read(self.0.fetch16, addr, Access::Fetch).map(|v| v as u16)
    }
    fn fetch_u32(&mut self, addr: u32) -> Result<u32, Fault> {
        self.read(self.0.fetch32, addr, Access::Fetch)
    }
}

/// `sub_arch` must be [`SUB_ARCH_ARMV6`]; `bus` must point to an [`ArmBusCallbacks`]. The CPU starts in the default
/// configuration (EKA2's: unaligned ARMv6, VFP on); `load_context` sets another.
extern "C" fn create_impl(sub_arch: u32, bus: *const heirloom_cpu_abi::CpuBusCallbacks) -> *mut CpuInstance {
    if sub_arch != SUB_ARCH_ARMV6 || bus.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: for ARCH_ARM the contract says `bus` points to an ArmBusCallbacks (whose first field is the common record).
    let bus = copy_bus(unsafe { &*bus.cast::<ArmBusCallbacks>() });
    Box::into_raw(Box::new(Instance { bus, cpu: Cpu::new(Config::default()), exit: Exit::BudgetSpent })).cast()
}

extern "C" fn destroy_impl(inst: *mut CpuInstance) {
    if !inst.is_null() {
        // SAFETY: see `instance`; destroy ends the handle.
        drop(unsafe { Box::from_raw(inst.cast::<Instance>()) });
    }
}

/// Every register cleared, the configuration kept.
extern "C" fn reset_impl(inst: *mut CpuInstance) {
    if let Some(i) = instance(inst) {
        i.cpu = Cpu::new(i.cpu.config);
        i.exit = Exit::BudgetSpent;
    }
}

extern "C" fn run_impl(inst: *mut CpuInstance, budget: u64, stop: *mut CpuStop) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    if stop.is_null() {
        return -1;
    }
    let (exit, insns) = i.cpu.run_counted(&mut Bus(&i.bus), budget);
    i.exit = exit;
    let (kind, pc, faults) = match exit {
        Exit::BudgetSpent => (STOP_BUDGET, i.cpu.pc(), 0),
        Exit::Svc(_) | Exit::Bkpt(_) => (STOP_TRAP, i.cpu.pc(), 0),
        Exit::Wait => (STOP_HALT, i.cpu.pc(), 0),
        Exit::Fault { pc, .. } => (STOP_FAULT, pc, 1),
        Exit::Trap(t) => (STOP_FAULT, t.pc, 0),
    };
    // SAFETY: non-null, and the caller's record (the table's contract).
    unsafe { *stop = CpuStop { kind, pc, detail: 0, bus_faults: faults, insns } };
    0
}

/// A user-mode core takes no interrupts: refused.
extern "C" fn interrupt_impl(_inst: *mut CpuInstance, _level: u32) -> i32 {
    -1
}

extern "C" fn get_reg_impl(inst: *mut CpuInstance, reg: u32, out: *mut u64) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    let v = match reg {
        0..=15 => i.cpu.regs[reg as usize],
        REG_CPSR => i.cpu.cpsr(),
        REG_FPSCR => i.cpu.vfp.fpscr,
        REG_TPIDRURW => i.cpu.tpidrurw,
        REG_TPIDRURO => i.cpu.tpidruro,
        _ => return -1,
    };
    if out.is_null() {
        return -1;
    }
    // SAFETY: non-null caller record.
    unsafe { *out = v as u64 };
    0
}

extern "C" fn set_reg_impl(inst: *mut CpuInstance, reg: u32, value: u64) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    let v = value as u32;
    match reg {
        0..=15 => i.cpu.regs[reg as usize] = v,
        REG_CPSR => i.cpu.set_cpsr(v),
        REG_FPSCR => i.cpu.vfp.fpscr = v,
        REG_TPIDRURW => i.cpu.tpidrurw = v,
        REG_TPIDRURO => i.cpu.tpidruro = v,
        _ => return -1,
    }
    0
}

/// An interpreter caches no decoded code: nothing to invalidate.
extern "C" fn invalidate_code_impl(_inst: *mut CpuInstance, _addr: u32, _len: u32) {}

extern "C" fn context_size_impl(_inst: *mut CpuInstance) -> u32 {
    core::mem::size_of::<ArmContext>() as u32
}

extern "C" fn save_context_impl(inst: *mut CpuInstance, dst: *mut u8, len: u32) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    if dst.is_null() || (len as usize) < core::mem::size_of::<ArmContext>() {
        return -1;
    }
    let ctx = i.cpu.to_context(&i.exit);
    // SAFETY: dst holds at least size_of::<ArmContext>() writable bytes (checked); unaligned write, no reference formed.
    unsafe { dst.cast::<ArmContext>().write_unaligned(ctx) };
    0
}

extern "C" fn load_context_impl(inst: *mut CpuInstance, src: *const u8, len: u32) -> i32 {
    let Some(i) = instance(inst) else { return -1 };
    if src.is_null() || (len as usize) < core::mem::size_of::<ArmContext>() {
        return -1;
    }
    // SAFETY: src holds at least size_of::<ArmContext>() readable bytes (checked); every bit pattern is a valid ArmContext.
    let ctx = unsafe { src.cast::<ArmContext>().read_unaligned() };
    match Cpu::from_context(&ctx) {
        Some(cpu) => {
            i.cpu = cpu;
            i.exit = Exit::BudgetSpent;
            0
        }
        None => -1,
    }
}

/// No PC ring is kept.
extern "C" fn recent_pcs_impl(_inst: *mut CpuInstance, _out: *mut u32, _cap: u32) -> u32 {
    0
}

static CPU_ARM_VTABLE: CpuVTable = CpuVTable {
    version: CPU_ABI_VERSION,
    size: core::mem::size_of::<CpuVTable>() as u32,
    arch_id: ARCH_ARM,
    create: create_impl,
    destroy: destroy_impl,
    reset: reset_impl,
    run: run_impl,
    interrupt: interrupt_impl,
    get_reg: get_reg_impl,
    set_reg: set_reg_impl,
    invalidate_code: invalidate_code_impl,
    context_size: context_size_impl,
    save_context: save_context_impl,
    load_context: load_context_impl,
    recent_pcs: recent_pcs_impl,
};

/// The table cpu-arm.library's `coral_lib_open` returns.
pub fn vtable() -> &'static CpuVTable {
    &CPU_ARM_VTABLE
}
