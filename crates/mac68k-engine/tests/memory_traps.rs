//! The Memory Manager as a program sees it: OS traps, register convention, handles as pointers to pointers.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::fixture::MEMTEST;
use mac68k_engine::Mac;

fn run() -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let r = m.run_program(MEMTEST, 0, 50_000).unwrap();
    assert!(r.stats.halted, "ExitToShell ends the run: {:#?}", m.log);
    assert_eq!(r.stats.bus_faults, 0);
    m
}
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }
fn w(m: &Mac<RecordingGuestUi>, a: u32) -> i16 { m.mem.read_u16(a).unwrap() as i16 }

#[test]
fn handles_pointers_state_resize_and_dispose_through_registers() {
    let m = run();
    let h = l(&m, 0x3000);
    assert_ne!(h, 0);
    assert_eq!(w(&m, 0x3004), 0, "NewHandle -> noErr");
    assert_eq!(l(&m, 0x3008), 0x80, "HGetState of a locked handle");
    assert_eq!(w(&m, 0x300C), -117, "SetHandleSize on a locked handle: memLockedErr");
    assert_eq!(w(&m, 0x300E), 0, "after HUnlock it resizes");
    assert_eq!(l(&m, 0x3010), 0xCAFE_BABE, "the block moved, the contents and the handle's meaning did not");
    assert_eq!(l(&m, 0x3014), 4096, "GetHandleSize");
    assert_eq!(w(&m, 0x3018), 0, "DisposeHandle");
    assert_eq!(w(&m, 0x301A), -111, "GetHandleSize of a disposed handle: memWZErr");
    assert_eq!(w(&m, 0x301C), -111, "MemError (the MemErr global) agrees");
    assert!(m.refused.iter().any(|r| r.starts_with("GetHandleSize:") && r.contains("memWZErr")), "a stale handle is also a NAMED refusal: {:?}", m.refused);
    // NewPtrClear + BlockMove + GetPtrSize + DisposePtr
    assert_ne!(l(&m, 0x3020), 0);
    assert_eq!((l(&m, 0x3028), l(&m, 0x302C)), (0x4142_4344, 0x4546_4748), "BlockMove copied ABCDEFGH");
    assert_eq!(l(&m, 0x3030), 32);
    assert_eq!(w(&m, 0x3034), 0);
    assert_eq!(m.mm.live_ptrs(), 0);
}

#[test]
fn trap_sets_the_condition_codes_from_d0() {
    let m = run();
    assert_eq!(w(&m, 0x3042), 1, "BEQ taken after a successful HLock (D0 = 0)");
    assert_eq!(w(&m, 0x3040), 1, "BNE taken after memFullErr");
    assert_eq!(l(&m, 0x3050), 0, "a failed NewHandle returns NIL in A0");
    assert_eq!(m.mm.live_handles(), 1, "only the last (cleared) handle is left alive");
}

#[test]
fn clear_variant_is_the_same_trap_with_bit_9() {
    let m = run();
    assert_eq!(l(&m, 0x3048), 0xEEEE_EEEE, "CONTROL: plain NewHandle gives back the dirty block");
    assert_eq!(l(&m, 0x3044), 0, "NewHandleClear ($A322) zeroed it");
}

#[test]
fn a_wild_blockmove_is_refused_by_name_and_the_run_goes_on() {
    // lea 0xFFFF0000,a0 ; move.l a0,a1 ; moveq #4,d0 ; _BlockMove ; _ExitToShell
    let prog = [0x41, 0xF9, 0xFF, 0xFF, 0x00, 0x00, 0x22, 0x48, 0x70, 0x04, 0xA0, 0x2E, 0xA9, 0xF4];
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let r = m.run_program(&prog, 0, 100).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0);
    assert_eq!(m.refused.len(), 1, "{:?}", m.refused);
    assert!(m.refused[0].starts_with("BlockMove: 4 bytes 0xffff0000 -> 0xffff0000 leave guest RAM"), "{:?}", m.refused);
    assert_eq!(m.mem.read_u16(0x220).unwrap() as i16, -111);
}
