//! `heirloom-cpu-abi` entry points for the single process-wide Musashi CPU.

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use heirloom_cpu_abi::{
    CpuBusCallbacks, CpuInstance, CpuStop, CpuVTable, ARCH_M68K, CPU_ABI_VERSION,
    STOP_BUDGET, STOP_FAULT, STOP_HALT, STOP_TRAP,
};
use heirloom_mem_flat32::GuestMem;

use super::{
    m68k_get_reg, m68k_set_irq, m68k_set_reg, resume, start_model, Cpu, CpuContext,
    CpuModel, Guard, Host, CPU_LOCK, REG_PC,
};

/// A field-by-field copy of the caller's callback table (plain function pointers and an opaque context pointer; the contract
/// crate does not derive Copy, and its layout must not change for that).
fn copy_bus(b: &CpuBusCallbacks) -> CpuBusCallbacks {
    CpuBusCallbacks { ctx: b.ctx, read8: b.read8, read16: b.read16, read32: b.read32, write8: b.write8, write16: b.write16, write32: b.write32, on_trap: b.on_trap }
}

struct Instance {
    bus: CpuBusCallbacks,
    model: CpuModel,
    ctx: Option<CpuContext>,
}

static INSTANCE: AtomicPtr<Instance> = AtomicPtr::new(ptr::null_mut());

fn validate(inst: *mut CpuInstance) -> *mut Instance {
    if inst.is_null() {
        return ptr::null_mut();
    }
    let current = INSTANCE.load(Ordering::Acquire);
    if current.cast::<CpuInstance>() != inst {
        ptr::null_mut()
    } else {
        current
    }
}

struct BusHost {
    bus: CpuBusCallbacks,
    dummy_mem: GuestMem,
}

impl BusHost {
    fn new(bus: CpuBusCallbacks) -> Self {
        Self { bus, dummy_mem: GuestMem::new(0) }
    }

    fn trap(&mut self, cpu: &mut Cpu, pc: u32, opcode: u32) -> bool {
        match (self.bus.on_trap)(self.bus.ctx, pc, opcode) {
            0 => true,
            2 => {
                cpu.halt();
                true
            }
            3 => {
                cpu.block();
                true
            }
            _ => false,
        }
    }
}

impl Host for BusHost {
    fn mem(&mut self) -> &mut GuestMem {
        // `read` and `write` below are the real memory path. This empty object only
        // satisfies the trait method used by `start_model` to prepare reset vectors.
        &mut self.dummy_mem
    }

    fn read(&mut self, addr: u32, len: u32) -> Result<u32, ()> {
        let mut out = 0;
        let rc = match len {
            1 => (self.bus.read8)(self.bus.ctx, addr, &mut out),
            2 => (self.bus.read16)(self.bus.ctx, addr, &mut out),
            4 => (self.bus.read32)(self.bus.ctx, addr, &mut out),
            _ => return Err(()),
        };
        if rc == 0 { Ok(out) } else { Err(()) }
    }

    fn write(&mut self, addr: u32, len: u32, value: u32) -> Result<(), ()> {
        let rc = match len {
            1 => (self.bus.write8)(self.bus.ctx, addr, value),
            2 => (self.bus.write16)(self.bus.ctx, addr, value),
            4 => (self.bus.write32)(self.bus.ctx, addr, value),
            _ => return Err(()),
        };
        if rc == 0 { Ok(()) } else { Err(()) }
    }

    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool {
        // The Rust hook only routes the bridge ILLEGAL opcode to this method.
        self.trap(cpu, pc, 0x4AFC)
    }

    fn aline(&mut self, cpu: &mut Cpu, opcode: u16, pc: u32) -> bool {
        self.trap(cpu, pc, u32::from(opcode))
    }
}

extern "C" fn create_impl(sub_arch: u32, bus: *const CpuBusCallbacks) -> *mut CpuInstance {
    let model = match sub_arch {
        1 => CpuModel::M68000,
        2 => CpuModel::M68010,
        4 => CpuModel::M68020,
        _ => return ptr::null_mut(),
    };
    if bus.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: the ABI requires a non-null pointer to a readable callback table for
    // the duration of this call; the instance owns this copy afterwards.
    let bus = unsafe { ptr::read(bus) };
    let raw = Box::into_raw(Box::new(Instance { bus, model, ctx: None }));
    if INSTANCE
        .compare_exchange(ptr::null_mut(), raw, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        raw.cast::<CpuInstance>()
    } else {
        // SAFETY: the slot was occupied, so `raw` was never published or shared.
        unsafe { drop(Box::from_raw(raw)) };
        ptr::null_mut()
    }
}

extern "C" fn destroy_impl(inst: *mut CpuInstance) {
    let raw = validate(inst);
    if raw.is_null() {
        return;
    }
    if INSTANCE
        .compare_exchange(raw, ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        // SAFETY: successful removal from the slot transfers the unique ownership
        // originally produced by `Box::into_raw` back to this function.
        unsafe { drop(Box::from_raw(raw)) };
    }
}

extern "C" fn reset_impl(inst: *mut CpuInstance) {
    let raw = validate(inst);
    if raw.is_null() {
        return;
    }
    // SAFETY: `validate` proves this is the live instance in the one-instance slot.
    let instance = unsafe { &mut *raw };
    let mut ssp = 0;
    let mut pc = 0;
    let ssp_rc = (instance.bus.read32)(instance.bus.ctx, 0, &mut ssp);
    let pc_rc = (instance.bus.read32)(instance.bus.ctx, 4, &mut pc);
    let mut host = BusHost::new(copy_bus(&instance.bus));
    instance.ctx = Some(start_model(
        &mut host,
        if pc_rc == 0 { pc } else { 0 },
        if ssp_rc == 0 { ssp } else { 0 },
        &[],
        instance.model,
    ));
}

extern "C" fn run_impl(inst: *mut CpuInstance, budget: u64, stop: *mut CpuStop) -> i32 {
    let raw = validate(inst);
    if raw.is_null() {
        return -1;
    }
    // SAFETY: `validate` proves this is the live instance in the one-instance slot.
    let instance = unsafe { &mut *raw };
    let ctx = match instance.ctx.as_mut() {
        Some(ctx) => ctx,
        None => return -1,
    };
    let mut host = BusHost::new(copy_bus(&instance.bus));
    let stats = resume(&mut host, ctx, budget);
    // SAFETY: `resume` leaves Musashi's live registers matching the saved context.
    // Reading them here also avoids taking CPU_LOCK a second time.
    let pc = unsafe { m68k_get_reg(ptr::null_mut(), REG_PC) };
    if !stop.is_null() {
        // SAFETY: the ABI requires `stop`, when non-null, to be writable.
        let stop = unsafe { &mut *stop };
        stop.insns = stats.insns;
        stop.pc = pc;
        stop.detail = 0;
        if stats.halted {
            stop.kind = STOP_HALT;
        } else if stats.blocked {
            stop.kind = STOP_TRAP;
            stop.detail = 1;
        } else if let Some(stalled_at) = stats.stalled_at {
            stop.kind = STOP_FAULT;
            stop.detail = stalled_at.into();
        } else {
            stop.kind = STOP_BUDGET;
        }
    }
    0
}

extern "C" fn interrupt_impl(inst: *mut CpuInstance, level: u32) -> i32 {
    if validate(inst).is_null() || level > 7 {
        return -1;
    }
    if CPU_LOCK.load(Ordering::Acquire) {
        // SAFETY: a trap callback may request an interrupt while the current run
        // owns Musashi's globals, so no additional lock is taken.
        unsafe { m68k_set_irq(level) };
    } else {
        let _guard = Guard::take();
        // SAFETY: CPU_LOCK is held while changing Musashi's global IRQ line.
        unsafe { m68k_set_irq(level) };
    }
    0
}

extern "C" fn get_reg_impl(inst: *mut CpuInstance, reg: u32, out: *mut u64) -> i32 {
    if reg > 17 || out.is_null() {
        return -1;
    }
    let raw = validate(inst);
    if raw.is_null() {
        return -1;
    }
    // SAFETY: `validate` proves this is the live instance in the one-instance slot.
    let instance = unsafe { &mut *raw };
    if instance.ctx.is_none() {
        return -1;
    }
    // SAFETY: the live registers mirror the saved context between runs and are the
    // current registers during a trap callback. No reference to `Cpu` is formed.
    let value = unsafe { m68k_get_reg(ptr::null_mut(), reg) };
    // SAFETY: non-null `out` is an ABI-provided writable result pointer.
    unsafe { *out = u64::from(value) };
    0
}

extern "C" fn set_reg_impl(inst: *mut CpuInstance, reg: u32, value: u64) -> i32 {
    if reg > 17 {
        return -1;
    }
    let raw = validate(inst);
    if raw.is_null() {
        return -1;
    }
    // SAFETY: `validate` proves this is the live instance in the one-instance slot.
    let instance = unsafe { &mut *raw };
    if instance.ctx.is_none() {
        return -1;
    }
    if CPU_LOCK.load(Ordering::Acquire) {
        // SAFETY: inside a run or trap, the lock is already held and the live CPU
        // must be changed directly; taking the lock again would deadlock.
        unsafe { m68k_set_reg(reg, value as u32) };
    } else if let Some(ctx) = instance.ctx.as_mut() {
        ctx.set_reg(reg, value as u32);
    }
    0
}

extern "C" fn invalidate_code_impl(_inst: *mut CpuInstance, _addr: u32, _len: u32) {
    // Musashi fetches and interprets instructions from memory; it has no translated
    // code cache to invalidate.
}

pub static CPU_68K_VTABLE: CpuVTable = CpuVTable {
    version: CPU_ABI_VERSION,
    size: core::mem::size_of::<CpuVTable>() as u32,
    arch_id: ARCH_M68K,
    create: create_impl,
    destroy: destroy_impl,
    reset: reset_impl,
    run: run_impl,
    interrupt: interrupt_impl,
    get_reg: get_reg_impl,
    set_reg: set_reg_impl,
    invalidate_code: invalidate_code_impl,
};

pub fn vtable() -> &'static CpuVTable {
    &CPU_68K_VTABLE
}