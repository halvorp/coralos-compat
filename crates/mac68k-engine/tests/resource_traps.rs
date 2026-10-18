//! The Resource Manager as a program sees it: resources are handles into guest memory.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::fixture::{restest_fork, RESTEST};
use mac68k_engine::resmgr::APP_REF_NUM;
use mac68k_engine::Mac;
use retro_resfork::ResFork;

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
