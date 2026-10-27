//! `heirloom-cpu-abi` entry points for the single process-wide Musashi CPU.

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use heirloom_cpu_abi::{
    CpuBusCallbacks, CpuInstance, CpuStop, CpuVTable, ARCH_M68K, CPU_ABI_VERSION,
    STOP_BUDGET, STOP_FAULT, STOP_HALT, STOP_STALL, STOP_TRAP,
};
use heirloom_mem_flat32::GuestMem;

use super::{
    m68k_context_size, m68k_get_reg, m68k_set_irq, m68k_set_reg, resume, start_model,
    Cpu, CpuContext, CpuModel, Guard, Host, REG_PC,
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
/// True exactly while `run_impl` executes the instance: then (and only then) Musashi's live registers are the instance's and a
/// trap callback may read or write them. CPU_LOCK cannot answer that question: the static API's `run` on another thread
/// holds it too, and its live registers belong to that run.
static RUNNING: AtomicBool = AtomicBool::new(false);

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
    RUNNING.store(true, Ordering::Release);
    let stats = resume(&mut host, ctx, budget);
    RUNNING.store(false, Ordering::Release);
    // Read the PC from the context `resume` just saved, not from the live core: the lock is released, so another user of
    // the process-wide core may already have replaced the live registers.
    // SAFETY: the context holds exactly m68k_context_size() bytes; m68k_get_reg reads only the context it is given.
    let pc = unsafe { m68k_get_reg(ctx.bytes_mut().as_mut_ptr(), REG_PC) };
    if !stop.is_null() {
        // SAFETY: the ABI requires `stop`, when non-null, to be writable.
        let stop = unsafe { &mut *stop };
        stop.insns = stats.insns;
        stop.bus_faults = stats.bus_faults;
        stop.detail = 0;
        stop.pc = pc;
        // Kind priority: halt, block, stall (stop.pc = where), bus fault, budget; bus_faults is reported with every kind.
        if stats.halted {
            stop.kind = STOP_HALT;
        } else if stats.blocked {
            stop.kind = STOP_TRAP;
            stop.detail = 1;
        } else if let Some(stall_pc) = stats.stalled_at {
            stop.kind = STOP_STALL;
            stop.pc = stall_pc;
        } else if stats.bus_faults > 0 {
            stop.kind = STOP_FAULT;
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
    if RUNNING.load(Ordering::Acquire) {
        // SAFETY: inside this instance's run (a trap callback) the run owns Musashi's globals and holds CPU_LOCK, so no
        // additional lock is taken.
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
    let value = if RUNNING.load(Ordering::Acquire) {
        // SAFETY: inside this instance's run (a trap callback) the live registers ARE its registers. No reference to
        // `Cpu` is formed.
        unsafe { m68k_get_reg(ptr::null_mut(), reg) }
    } else {
        // Between runs the instance's SAVED context is its registers: the live ones may belong to another user of the
        // process-wide core (the static API's run on another thread), so read the saved bytes.
        let Some(ctx) = instance.ctx.as_mut() else { return -1 };
        // SAFETY: the context holds exactly m68k_context_size() bytes; m68k_get_reg reads only the context it is given.
        unsafe { m68k_get_reg(ctx.bytes_mut().as_mut_ptr(), reg) }
    };
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
    if RUNNING.load(Ordering::Acquire) {
        // SAFETY: inside this instance's run (a trap callback) the lock is already held and the live CPU must be changed
        // directly; taking the lock again would deadlock.
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

/// Context size is a property of the CPU model, not the instance.
extern "C" fn context_size_impl(inst: *mut CpuInstance) -> u32 {
    if validate(inst).is_null() { return 0; }
    // SAFETY: m68k_context_size is a pure constant query with no preconditions.
    unsafe { m68k_context_size() }
}

/// Snapshot the saved context. Refused mid-run (stale) or before any run.
extern "C" fn save_context_impl(inst: *mut CpuInstance, dst: *mut u8, len: u32) -> i32 {
    let inst = validate(inst);
    if inst.is_null() || dst.is_null() { return -1; }
    // Refuse during a run: the saved context is stale while Musashi executes.
    if RUNNING.load(Ordering::Acquire) { return -1; }
    // SAFETY: pure constant query.
    let ctx_size = unsafe { m68k_context_size() };
    if len < ctx_size { return -1; }
    // SAFETY: validate confirmed inst is the live Instance.
    let instance = unsafe { &*inst };
    let Some(ctx) = instance.ctx.as_ref() else { return -1; };
    let src = ctx.bytes();
    // SAFETY: dst is non-null, len >= ctx_size, src.len() == ctx_size.
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst, ctx_size as usize); }
    0
}

/// Load a saved context. Refused mid-run (would corrupt Musashi globals).
extern "C" fn load_context_impl(inst: *mut CpuInstance, src: *const u8, len: u32) -> i32 {
    let inst = validate(inst);
    if inst.is_null() || src.is_null() { return -1; }
    if RUNNING.load(Ordering::Acquire) { return -1; }
    // SAFETY: pure constant query.
    let ctx_size = unsafe { m68k_context_size() };
    if len != ctx_size { return -1; }
    // SAFETY: src is non-null, len == ctx_size.
    let slice = unsafe { core::slice::from_raw_parts(src, ctx_size as usize) };
    // SAFETY: validate confirmed inst is the live Instance.
    let instance = unsafe { &mut *inst };
    instance.ctx = Some(CpuContext::from_bytes(slice));
    0
}

/// Most recent PCs, oldest first, capped to the last `cap` entries.
extern "C" fn recent_pcs_impl(inst: *mut CpuInstance, out: *mut u32, cap: u32) -> u32 {
    let inst = validate(inst);
    if inst.is_null() || out.is_null() { return 0; }
    let pcs = crate::recent_pcs();
    let len = pcs.len();
    let n = core::cmp::min(cap as usize, len);
    if n == 0 { return 0; }
    let start = len - n;
    // SAFETY: out is non-null, cap >= n.
    unsafe {
        for (i, &pc) in pcs[start..].iter().enumerate() {
            core::ptr::write(out.add(i), pc);
        }
    }
    n as u32
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
    context_size: context_size_impl,
    save_context: save_context_impl,
    load_context: load_context_impl,
    recent_pcs: recent_pcs_impl,
};

pub fn vtable() -> &'static CpuVTable {
    &CPU_68K_VTABLE
}

#[cfg(test)]
mod vtable_tests {
    use super::*;
    use core::ffi::c_void;
    use core::sync::atomic::AtomicU32;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());
    static BUS_LIMIT: AtomicU32 = AtomicU32::new(0x10000);

    // 64 KiB big-endian RAM. Reset reads [0..4)=SSP, [4..8)=PC.
    static mut RAM: [u8; 0x10000] = [0; 0x10000];

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bus_read(addr: u32, len: u32) -> Result<u32, ()> {
        let end = addr.checked_add(len).ok_or(())?;
        if addr >= BUS_LIMIT.load(Ordering::Relaxed) || end > 0x10000 {
            return Err(());
        }
        let mut value = 0;
        for i in 0..len {
            // SAFETY: the range check above covers every byte read.
            let byte = unsafe { ptr::addr_of!(RAM).cast::<u8>().add((addr + i) as usize).read() };
            value = (value << 8) | u32::from(byte);
        }
        Ok(value)
    }

    fn bus_write(addr: u32, len: u32, value: u32) -> Result<(), ()> {
        let end = addr.checked_add(len).ok_or(())?;
        if addr >= BUS_LIMIT.load(Ordering::Relaxed) || end > 0x10000 {
            return Err(());
        }
        for i in 0..len {
            let shift = 8 * (len - 1 - i);
            // SAFETY: the range check above covers every byte written.
            unsafe {
                ptr::addr_of_mut!(RAM).cast::<u8>().add((addr + i) as usize).write((value >> shift) as u8);
            }
        }
        Ok(())
    }

    extern "C" fn read8(_ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
        if out.is_null() { return 1; }
        match bus_read(addr, 1) {
            Ok(value) => {
                // SAFETY: the ABI supplies a writable result pointer.
                unsafe { *out = value };
                0
            }
            Err(()) => 1,
        }
    }

    extern "C" fn read16(_ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
        if out.is_null() { return 1; }
        match bus_read(addr, 2) {
            Ok(value) => {
                // SAFETY: the ABI supplies a writable result pointer.
                unsafe { *out = value };
                0
            }
            Err(()) => 1,
        }
    }

    extern "C" fn read32(_ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
        if out.is_null() { return 1; }
        match bus_read(addr, 4) {
            Ok(value) => {
                // SAFETY: the ABI supplies a writable result pointer.
                unsafe { *out = value };
                0
            }
            Err(()) => 1,
        }
    }

    extern "C" fn write8(_ctx: *mut c_void, addr: u32, value: u32) -> i32 {
        if bus_write(addr, 1, value).is_ok() { 0 } else { 1 }
    }

    extern "C" fn write16(_ctx: *mut c_void, addr: u32, value: u32) -> i32 {
        if bus_write(addr, 2, value).is_ok() { 0 } else { 1 }
    }

    extern "C" fn write32(_ctx: *mut c_void, addr: u32, value: u32) -> i32 {
        if bus_write(addr, 4, value).is_ok() { 0 } else { 1 }
    }

    extern "C" fn on_trap(_ctx: *mut c_void, _pc: u32, opcode: u32) -> i32 {
        // ILLEGAL halts; the A-line word $A000 blocks (the guest stops mid-call and the next run continues after it), which
        // is how these tests stop at an exact instruction: a run budget is not instruction-exact (Musashi runs in slices).
        match opcode { 0x4AFC => 2, 0xA000 => 3, _ => 1 }
    }

    fn bus() -> CpuBusCallbacks {
        CpuBusCallbacks {
            ctx: ptr::null_mut(),
            read8,
            read16,
            read32,
            write8,
            write16,
            write32,
            on_trap,
        }
    }

    struct Rig(*mut CpuInstance);

    impl Drop for Rig {
        fn drop(&mut self) {
            destroy_impl(self.0);
        }
    }

    fn rig(program: &[u8]) -> Rig {
        BUS_LIMIT.store(0x10000, Ordering::Relaxed);
        // SAFETY: LOCK serialises every use of RAM in this test module.
        unsafe { ptr::write_bytes(ptr::addr_of_mut!(RAM).cast::<u8>(), 0, 0x10000) };
        bus_write(0, 4, 0x8000).unwrap();
        bus_write(4, 4, 0x1000).unwrap();
        for (i, &byte) in program.iter().enumerate() {
            bus_write(0x1000 + i as u32, 1, u32::from(byte)).unwrap();
        }
        let inst = create_impl(1, &bus());
        assert!(!inst.is_null());
        reset_impl(inst);
        Rig(inst)
    }

    fn reg(inst: *mut CpuInstance, reg: u32) -> u32 {
        let mut value = 0;
        assert_eq!(get_reg_impl(inst, reg, &mut value), 0);
        value as u32
    }

    fn stopped(inst: *mut CpuInstance, budget: u64) -> CpuStop {
        let mut stop = CpuStop { kind: 0, pc: 0, detail: 0, bus_faults: 0, insns: 0 };
        assert_eq!(run_impl(inst, budget, &mut stop), 0);
        stop
    }

    #[test]
    fn v2_table_exposes_context_and_diagnostics() {
        let _guard = lock();
        let rig = rig(&[0x4A, 0xFC]);
        assert_eq!(vtable().version, 2);
        assert_eq!(vtable().size, 112);
        assert!(context_size_impl(rig.0) > 0);
        assert_eq!(vtable().context_size as *const () as usize, context_size_impl as *const () as usize);
        assert_eq!(vtable().save_context as *const () as usize, save_context_impl as *const () as usize);
        assert_eq!(vtable().load_context as *const () as usize, load_context_impl as *const () as usize);
        assert_eq!(vtable().recent_pcs as *const () as usize, recent_pcs_impl as *const () as usize);
    }

    #[test]
    fn saved_context_resumes_the_same_execution() {
        let _guard = lock();
        let rig = rig(&[0x70, 0x01, 0xA0, 0x00, 0x70, 0x02, 0x4A, 0xFC]);
        let blocked = stopped(rig.0, 100);
        assert_eq!((blocked.kind, blocked.detail), (STOP_TRAP, 1));
        assert_eq!(reg(rig.0, 0), 1);
        let size = context_size_impl(rig.0);
        let mut saved = vec![0u8; size as usize];
        assert_eq!(save_context_impl(rig.0, saved.as_mut_ptr(), size), 0);
        let first = stopped(rig.0, 100);
        assert_eq!(reg(rig.0, 0), 2);
        assert_eq!(first.kind, STOP_HALT);
        assert_eq!(load_context_impl(rig.0, saved.as_ptr(), size), 0);
        let second = stopped(rig.0, 100);
        assert_eq!(reg(rig.0, 0), 2);
        assert_eq!(load_context_impl(rig.0, saved.as_ptr(), size), 0);
        let third = stopped(rig.0, 100);
        assert!(second.insns > 0);
        assert_eq!(second.insns, third.insns);
    }

    #[test]
    fn contexts_preserve_independent_register_values() {
        let _guard = lock();
        let rig = rig(&[0x70, 0x01, 0xA0, 0x00, 0x70, 0x02, 0x4A, 0xFC]);
        let blocked = stopped(rig.0, 100);
        assert_eq!((blocked.kind, blocked.detail), (STOP_TRAP, 1));
        assert_eq!(reg(rig.0, 0), 1);
        let size = context_size_impl(rig.0);
        let mut a = vec![0u8; size as usize];
        let mut b = vec![0u8; size as usize];
        assert_eq!(save_context_impl(rig.0, a.as_mut_ptr(), size), 0);
        assert_eq!(set_reg_impl(rig.0, 0, 99), 0);
        assert_eq!(save_context_impl(rig.0, b.as_mut_ptr(), size), 0);
        assert_eq!(load_context_impl(rig.0, a.as_ptr(), size), 0);
        assert_eq!(reg(rig.0, 0), 1);
        assert_eq!(load_context_impl(rig.0, b.as_ptr(), size), 0);
        assert_eq!(reg(rig.0, 0), 99);
    }

    #[test]
    fn context_calls_reject_bad_arguments() {
        let _guard = lock();
        let rig = rig(&[0x4A, 0xFC]);
        let size = context_size_impl(rig.0);
        let mut bytes = vec![0u8; size as usize + 1];
        assert_eq!(save_context_impl(rig.0, bytes.as_mut_ptr(), size - 1), -1);
        assert_eq!(save_context_impl(rig.0, ptr::null_mut(), size), -1);
        assert_eq!(load_context_impl(rig.0, bytes.as_ptr(), size + 1), -1);
        assert_eq!(save_context_impl(ptr::null_mut(), bytes.as_mut_ptr(), size), -1);
        assert_eq!(load_context_impl(ptr::null_mut(), bytes.as_ptr(), size), -1);
        assert_eq!(context_size_impl(ptr::null_mut()), 0);
    }

    #[test]
    fn stop_instruction_is_a_stall() {
        let _guard = lock();
        let rig = rig(&[0x4E, 0x72, 0x27, 0x00]);
        let stop = stopped(rig.0, 1000);
        // A STOP instruction stalls the core: STOP_STALL, never a fault.
        assert_eq!(stop.kind, STOP_STALL);
        assert!((0x1000..=0x1004).contains(&stop.pc));
        assert_eq!(stop.bus_faults, 0);
    }

    #[test]
    fn halt_keeps_the_bus_fault_count() {
        let _guard = lock();
        let rig = rig(&[0x23, 0xC0, 0x00, 0x00, 0x90, 0x00, 0x4A, 0xFC]);
        BUS_LIMIT.store(0x8000, Ordering::Relaxed);
        let stop = stopped(rig.0, 1000);
        assert_eq!(stop.kind, STOP_HALT);
        assert!(stop.bus_faults >= 1);
    }

    #[test]
    fn recent_pcs_returns_the_newest_tail() {
        let _guard = lock();
        let rig = rig(&[0x70, 0x01, 0x70, 0x02, 0x4A, 0xFC]);
        stopped(rig.0, 1000);
        let mut pcs = [0u32; 2];
        assert_eq!(recent_pcs_impl(rig.0, pcs.as_mut_ptr(), 2), 2);
        assert!(pcs == [0x1002, 0x1004]
            || (pcs[0] < pcs[1] && pcs.iter().all(|pc| (0x1000..=0x1004).contains(pc))));
    }
}