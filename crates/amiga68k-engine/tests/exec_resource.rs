//! exec OpenResource, driven by tests/guest/c/resource.c.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/resource.bin");

#[test]
fn opening_a_resource_nobody_added_returns_null_and_is_not_a_refusal() {
    let mut a = machine();
    let r = run(&mut a, PROG); // `run` fails on any refusal: before this handler existed the call was UNIMPLEMENTED
    assert_eq!((r[0], r[1]), (0, 1), "NULL even though d0 held garbage on entry; the program ran on");
    assert!(a.log.iter().any(|l| l == "[exec] OpenResource \"FileSystem.resource\" -> NULL (no such resource)"), "{:#?}", a.log);
}

#[test]
fn execbase_reports_its_own_library_version_like_a_real_kickstart() {
    let mut a = machine();
    run(&mut a, PROG);
    let base = amiga68k_engine::machine::EXEC_BASE;
    let (ty, ver, rev, soft) = (a.mem.read_u8(base + 8).unwrap(), a.mem.read_u16(base + 20).unwrap(), a.mem.read_u16(base + 22).unwrap(), a.mem.read_u16(base + 34).unwrap());
    assert_eq!((ty, ver, rev, soft), (9, 40, 68, 40), "NT_LIBRARY, AmigaOS 3.1 (40.68); all four were 0 before, so a `SysBase->LibNode.lib_Version >= 36` gate failed");
}

#[test]
fn task_resource_allocates_independent_slots_reuses_freed_ones_and_refuses_what_it_does_not_model_by_name() {
    let mut a = machine();
    let r = run_refusing(&mut a, include_bytes!("guest/taskres.bin"));
    assert_eq!(&r[..11], &[1, 1, 2, 0, 1, 0xCAFE, 0xBEEF, 1, 0, 1, 1], "{r:x?}");
    assert_eq!((r[11], r[12], r[13], r[14]), (0, 0, 0, 1), "a never-allocated slot reads 0; RunTaskHooks over a registered hook is refused (d0 = 0); the main task's parent slot is 0; the program ran to its end");
    let refused: Vec<&str> = a.refused.iter().map(String::as_str).collect();
    assert_eq!(refused.len(), 3, "{refused:?}");
    assert!(refused[0].contains("SetTaskStorageSlot 99: not an allocated slot"), "{refused:?}");
    assert!(refused[1].contains("FreeTaskStorageSlot 99: not an allocated slot"), "{refused:?}");
    assert!(refused[2].contains("RunTaskHooks type 1: 1 registered hook"), "{refused:?}");
}

#[test]
fn timer_device_get_sys_time_reads_the_virtual_clock_the_same_way_as_tr_getsystime() {
    let mut a = machine();
    let r = run(&mut a, include_bytes!("guest/timerdev.bin"));
    assert_eq!((r[0], r[1]), (0, 1), "OpenDevice succeeded and filled io_Device");
    assert_eq!((r[2], r[3]), (0, 0), "the clock starts at 0");
    assert_eq!((r[4], r[5]), (2, 500_000), "after a 2.5 s TR_ADDREQUEST");
    assert_eq!((r[6], r[7], r[8]), (2, 500_000, 1), "GetUpTime reports the same clock");
}

#[test]
fn dos_set_vbuf_accepts_the_console_and_fails_like_aros_on_null_or_a_misaligned_buffer() {
    let mut a = machine();
    let r = run_refusing(&mut a, include_bytes!("guest/setvbuf.bin"));
    assert_eq!((r[0], r[1], r[2], r[3], r[4]), (0, u32::MAX, u32::MAX, 0, 1), "{r:x?}");
    let refused: Vec<&str> = a.refused.iter().map(String::as_str).collect();
    assert_eq!(refused.len(), 1, "only the never-opened handle is refused: {refused:?}");
    assert!(refused[0].contains("SetVBuf on 0x48d0: not an open file"), "{refused:?}");
}

#[test]
fn get_parent_task_storage_slot_reads_the_task_that_added_the_caller() {
    let mut a = machine();
    let r = run_refusing(&mut a, include_bytes!("guest/taskparent.bin"));
    assert_eq!(r[0], 0, "the main program's parent is the launcher, which owns no storage: 0, not a refusal");
    assert_eq!(r[1], 0xCAFE, "the child added by main sees MAIN's value of the slot (rom/task/GetParentTaskStorageSlot.c)");
    assert_eq!(r[2], 0, "NEGATIVE CONTROL: the child's own value of the slot is not inherited");
    assert_eq!(r[4], 0xF00D, "a grandchild sees the CHILD's value, not main's: the parent is who was running at AddTask");
    assert_eq!(r[3], 1, "the child ran to its end");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}
