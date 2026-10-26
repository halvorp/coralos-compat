use core::mem::{align_of, offset_of, size_of};
use heirloom_cpu_abi::*;

extern "C" fn dummy_create(
    _sub_arch: u32,
    _bus: *const CpuBusCallbacks,
) -> *mut CpuInstance {
    core::ptr::null_mut()
}

extern "C" fn dummy_destroy(_instance: *mut CpuInstance) {}

extern "C" fn dummy_reset(_instance: *mut CpuInstance) {}

extern "C" fn dummy_run(
    _instance: *mut CpuInstance,
    _budget: u64,
    _stop: *mut CpuStop,
) -> i32 {
    0
}

extern "C" fn dummy_interrupt(_instance: *mut CpuInstance, _level: u32) -> i32 {
    0
}

extern "C" fn dummy_get_reg(_instance: *mut CpuInstance, _reg: u32, _out: *mut u64) -> i32 {
    0
}

extern "C" fn dummy_set_reg(_instance: *mut CpuInstance, _reg: u32, _value: u64) -> i32 {
    0
}

extern "C" fn dummy_invalidate_code(_instance: *mut CpuInstance, _addr: u32, _len: u32) {}

fn vtable(version: u32, size: u32, arch_id: u32) -> CpuVTable {
    CpuVTable {
        version,
        size,
        arch_id,
        create: dummy_create,
        destroy: dummy_destroy,
        reset: dummy_reset,
        run: dummy_run,
        interrupt: dummy_interrupt,
        get_reg: dummy_get_reg,
        set_reg: dummy_set_reg,
        invalidate_code: dummy_invalidate_code,
    }
}

fn valid_table() -> CpuVTable {
    vtable(
        CPU_ABI_VERSION,
        size_of::<CpuVTable>() as u32,
        ARCH_PPC,
    )
}

#[cfg(target_pointer_width = "64")]
#[test]
fn bus_callbacks_layout() {
    assert_eq!(size_of::<CpuBusCallbacks>(), 64);
    assert_eq!(align_of::<CpuBusCallbacks>(), 8);
    assert_eq!(offset_of!(CpuBusCallbacks, ctx), 0);
    assert_eq!(offset_of!(CpuBusCallbacks, read8), 8);
    assert_eq!(offset_of!(CpuBusCallbacks, read16), 16);
    assert_eq!(offset_of!(CpuBusCallbacks, read32), 24);
    assert_eq!(offset_of!(CpuBusCallbacks, write8), 32);
    assert_eq!(offset_of!(CpuBusCallbacks, write16), 40);
    assert_eq!(offset_of!(CpuBusCallbacks, write32), 48);
    assert_eq!(offset_of!(CpuBusCallbacks, on_trap), 56);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn vtable_layout() {
    assert_eq!(size_of::<CpuVTable>(), 80);
    assert_eq!(align_of::<CpuVTable>(), 8);
    assert_eq!(offset_of!(CpuVTable, version), 0);
    assert_eq!(offset_of!(CpuVTable, size), 4);
    assert_eq!(offset_of!(CpuVTable, arch_id), 8);
    assert_eq!(offset_of!(CpuVTable, create), 16);
    assert_eq!(offset_of!(CpuVTable, destroy), 24);
    assert_eq!(offset_of!(CpuVTable, reset), 32);
    assert_eq!(offset_of!(CpuVTable, run), 40);
    assert_eq!(offset_of!(CpuVTable, interrupt), 48);
    assert_eq!(offset_of!(CpuVTable, get_reg), 56);
    assert_eq!(offset_of!(CpuVTable, set_reg), 64);
    assert_eq!(offset_of!(CpuVTable, invalidate_code), 72);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn stop_layout() {
    assert_eq!(size_of::<CpuStop>(), 24);
    assert_eq!(align_of::<CpuStop>(), 8);
    assert_eq!(offset_of!(CpuStop, kind), 0);
    assert_eq!(offset_of!(CpuStop, pc), 4);
    assert_eq!(offset_of!(CpuStop, detail), 8);
    assert_eq!(offset_of!(CpuStop, insns), 16);
}

#[test]
fn check_rejects_an_old_version() {
    let table = vtable(1, size_of::<CpuVTable>() as u32, ARCH_PPC);
    assert!(matches!(
        check(&table, 2, ARCH_PPC),
        Err(OpenRefusal::VersionTooOld {
            found: 1,
            required: 2
        })
    ));
}

#[test]
fn check_rejects_a_short_table() {
    let table = vtable(
        CPU_ABI_VERSION,
        size_of::<CpuVTable>() as u32 - 1,
        ARCH_PPC,
    );
    assert!(matches!(
        check(&table, CPU_ABI_VERSION, ARCH_PPC),
        Err(OpenRefusal::TableTooSmall { .. })
    ));
}

#[test]
fn check_rejects_the_wrong_architecture() {
    let table = valid_table();
    assert!(matches!(
        check(&table, CPU_ABI_VERSION, ARCH_M68K),
        Err(OpenRefusal::WrongArch {
            found: ARCH_PPC,
            required: ARCH_M68K
        })
    ));
}

#[test]
fn check_accepts_newer_exact_and_larger_tables() {
    let newer = vtable(2, size_of::<CpuVTable>() as u32, ARCH_PPC);
    assert!(matches!(check(&newer, 1, ARCH_PPC), Ok(())));

    let exact = valid_table();
    assert!(matches!(
        check(&exact, CPU_ABI_VERSION, ARCH_PPC),
        Ok(())
    ));

    let larger = vtable(
        CPU_ABI_VERSION,
        size_of::<CpuVTable>() as u32 + 1,
        ARCH_PPC,
    );
    assert!(matches!(
        check(&larger, CPU_ABI_VERSION, ARCH_PPC),
        Ok(())
    ));
}

#[test]
fn version_refusal_precedes_size_refusal() {
    let table = vtable(1, size_of::<CpuVTable>() as u32 - 1, ARCH_M68K);
    assert!(matches!(
        check(&table, 2, ARCH_PPC),
        Err(OpenRefusal::VersionTooOld {
            found: 1,
            required: 2
        })
    ));
}

#[test]
fn size_refusal_precedes_architecture_refusal() {
    let table = vtable(
        CPU_ABI_VERSION,
        size_of::<CpuVTable>() as u32 - 1,
        ARCH_PPC,
    );
    assert!(matches!(
        check(&table, CPU_ABI_VERSION, ARCH_M68K),
        Err(OpenRefusal::TableTooSmall { .. })
    ));
}

#[test]
fn check_ptr_handles_null_and_valid_tables() {
    assert!(matches!(
        unsafe { check_ptr(core::ptr::null(), CPU_ABI_VERSION, ARCH_PPC) },
        Err(OpenRefusal::NullTable)
    ));

    let table = valid_table();
    assert!(matches!(
        unsafe { check_ptr(&table, CPU_ABI_VERSION, ARCH_PPC) },
        Ok(())
    ));
}

#[test]
fn stop_kind_mappings() {
    assert!(matches!(
        StopKind::from_raw(STOP_BUDGET),
        Some(StopKind::Budget)
    ));
    assert!(matches!(
        StopKind::from_raw(STOP_TRAP),
        Some(StopKind::Trap)
    ));
    assert!(matches!(
        StopKind::from_raw(STOP_FAULT),
        Some(StopKind::Fault)
    ));
    assert!(matches!(
        StopKind::from_raw(STOP_HALT),
        Some(StopKind::Halt)
    ));
    assert!(matches!(StopKind::from_raw(0), None));
    assert!(matches!(StopKind::from_raw(99), None));
}

#[test]
fn an_indexed_register_outside_its_file_is_none_not_a_guess() {
    use heirloom_cpu_abi::regs::{arm, ppc};
    assert_eq!((ppc::gpr(0), ppc::gpr(31), ppc::gpr(32)), (Some(0), Some(31), None));
    assert_eq!((arm::r(15), arm::r(16)), (Some(15), None));
}
