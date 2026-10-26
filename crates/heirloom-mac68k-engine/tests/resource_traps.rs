//! The Resource Manager as a program sees it: resources are handles into guest memory.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::fixture::{restest_fork, RESTEST};
use heirloom_mac68k_engine::resmgr::APP_REF_NUM;
use heirloom_mac68k_engine::Mac;
use heirloom_fmt_resfork::ResFork;

fn run() -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.rm.open_fork(APP_REF_NUM, ResFork::parse(restest_fork()).unwrap());
    let r = m.run_program(RESTEST, 0, 50_000).unwrap();
    assert!(r.stats.halted, "{:#?}", m.log);
    assert_eq!(r.stats.bus_faults, 0);
    m
}
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }
fn w(m: &Mac<RecordingGuestUi>, a: u32) -> i16 { m.mem.read_u16(a).unwrap() as i16 }

#[test]
fn get_count_named_ind_size_and_release() {
    let m = run();
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let h = l(&m, 0x3000);
    assert_ne!(h, 0);
    assert_eq!(w(&m, 0x3004), 2, "CountResources('TEXT')");
    let second = l(&m, 0x3008);
    assert!(second != 0 && second != h, "GetNamedResource folds case and finds TEXT 129");
    assert_eq!(l(&m, 0x3030), second, "GetIndResource #2 is the same handle as the named one");
    assert_eq!(l(&m, 0x300C), 0, "no such id -> NIL");
    assert_eq!(w(&m, 0x3010), -192, "ResError = resNotFound");
    assert_eq!(l(&m, 0x3014), 5, "SizeRsrc");
    assert_eq!(l(&m, 0x3018), 0x6865_6C6C, "'hell' is behind **h in guest memory");
    assert_eq!(l(&m, 0x301C), h, "GetResource on a loaded resource returns the same handle");
    assert_eq!(w(&m, 0x3020), 0, "ReleaseResource ok");
    assert_eq!(w(&m, 0x3022), -192, "a released handle is no longer a resource: resNotFound");
    assert!(m.mm.block(h).is_none(), "ReleaseResource disposed the handle");
    assert!(m.mm.block(second).unwrap().resource, "the other one is still a resource handle");
}

#[test]
fn the_resource_bit_survives_and_dispose_handle_forgets_the_resource() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.rm.open_fork(APP_REF_NUM, ResFork::parse(restest_fork()).unwrap());
    let h = m.get_resource(*b"TEXT", 128, false).unwrap();
    assert_eq!(m.mm.state(h), Ok(0x20), "resource bit in HGetState");
    m.dispose_handle(h).unwrap();
    let again = m.get_resource(*b"TEXT", 128, false).unwrap();
    assert_eq!(m.handle_bytes(again).unwrap(), b"hello", "a disposed resource handle reloads from the fork, not from freed memory");
    // Get1Resource only searches the current file; a bogus current file finds nothing (NEGATIVE)
    m.rm.cur = 99;
    assert!(m.get_resource(*b"TEXT", 129, true).is_err());
}

#[test]
fn max_size_rsrc_is_the_resource_size_and_resnotfound_for_a_stranger() {
    use heirloom_mac68k_engine::mixed::trap_opcode;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.rm.open_fork(APP_REF_NUM, ResFork::parse(restest_fork()).unwrap());
    let h = m.get_resource(*b"TEXT", 129, false).unwrap(); // "world!"
    let op = trap_opcode("MaxSizeRsrc").unwrap();
    assert_eq!(m.pascal_call(op, &[h]).unwrap().value, 6, "MaxSizeRsrc of 'world!'");
    let stranger = m.mm.new_handle(&mut m.mem, &mut m.heap, 9, false).unwrap();
    assert_eq!(m.pascal_call(op, &[stranger]).unwrap().value, 0, "a handle that is not a resource answers 0 ...");
    assert_eq!(m.mem.read_u16(heirloom_mac68k_engine::lowmem::RES_ERR).unwrap() as i16, -192, "... with ResError resNotFound (negative control)");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn the_standard_cursors_come_from_the_system_file_and_the_programs_own_wins() {
    use heirloom_mac68k_engine::mixed::trap_opcode;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let op = trap_opcode("GetCursor").unwrap();
    for id in 1..=4u32 {
        let h = m.pascal_call(op, &[id]).unwrap().value;
        assert_ne!(h, 0, "standard cursor {id} exists (iBeam, cross, plus, watch)");
        assert_eq!(m.mm.block(h).unwrap().size, 68, "image, mask, hot spot");
    }
    let ibeam = m.pascal_call(op, &[1]).unwrap().value;
    let hot = m.handle_bytes(ibeam).unwrap();
    assert_eq!((hot[64..66].to_vec(), hot[66..68].to_vec()), (vec![0, 4], vec![0, 7]), "the I-beam's hot spot is (4, 7)");
    assert_eq!(m.pascal_call(op, &[5]).unwrap().value, 0, "there is no cursor 5 ...");
    assert_eq!(m.mem.read_u16(heirloom_mac68k_engine::lowmem::RES_ERR).unwrap() as i16, -192, "... resNotFound (negative control)");
    // a program's own CURS 1 hides the System's
    let mut app = Mac::new(RecordingGuestUi::new()).unwrap();
    let fork = heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"CURS", 1, None, &[0xAA; 68]).build();
    app.rm.open_fork(APP_REF_NUM, ResFork::parse(fork).unwrap());
    let h = app.pascal_call(op, &[1]).unwrap().value;
    assert_eq!(app.handle_bytes(h).unwrap(), vec![0xAA; 68]);
    let two = app.pascal_call(op, &[2]).unwrap().value;
    assert_ne!(two, 0, "the others still come from the System file");
}
