//! `InitWindows`: the Window Manager port (`WMgrPort`, `DeskPort`), the desktop region (`GrayRgn`) and the port it leaves current.
mod common;
use common::{Prog, GLOBALS};
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::lowmem::{DESK_PORT, GRAY_RGN, WMGR_PORT};
use heirloom_mac68k_engine::{Mac, Step};

const SAVED: u32 = 0x3000;

fn run(program: impl FnOnce(&mut Prog)) -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    heirloom_mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    let mut p = Prog::new();
    program(&mut p);
    m.start(&p.exit().0, 0).unwrap();
    assert_eq!(m.step(100_000).0, Step::Exited, "{:#?}", m.log);
    m
}

#[test]
fn init_windows_makes_the_window_manager_port_the_desktop_region_and_the_current_port() {
    let m = run(|p| { p.trap(0xA912).l(SAVED).trap(0xA874); }); // InitWindows; GetPort(@saved)
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let port = m.mem.read_u32(WMGR_PORT).unwrap();
    assert_ne!(port, 0, "WMgrPort is a real port");
    assert_eq!(m.mem.read_u32(DESK_PORT).unwrap(), port, "DeskPort is the same port");
    assert_eq!(m.mem.read_u32(SAVED).unwrap(), port, "GetPort after InitWindows answers the Window Manager port (InitGraf left thePort NIL)");
    let (right, bottom) = (m.mem.read_i16(port + 2 + 6 + 6).unwrap(), m.mem.read_i16(port + 2 + 6 + 4).unwrap()); // portBits.bounds.right/bottom
    assert!(right > 0 && bottom > 0);
    let gray = m.read_region(m.mem.read_u32(GRAY_RGN).unwrap()).unwrap();
    let mbar = i32::from(m.mem.read_i16(0xBAA).unwrap());
    let (w, h) = (i32::from(right), i32::from(bottom));
    assert!(gray.contains(w / 2, h / 2), "the middle of the desktop is gray");
    assert!(!gray.contains(w / 2, mbar - 1) && gray.contains(w / 2, mbar), "the menu bar is not part of the desktop; the row below it is");
    assert!(!gray.contains(0, h - 1) && !gray.contains(w - 1, h - 1), "the bottom corners are rounded off");
    // NEGATIVE CONTROL: a second InitWindows keeps the one port and the one region (it re-initialises them in place)
    let m2 = run(|p| { p.trap(0xA912).trap(0xA912); });
    assert_eq!((m2.mem.read_u32(WMGR_PORT).unwrap(), m2.mem.read_u32(GRAY_RGN).unwrap()), (port, m.mem.read_u32(GRAY_RGN).unwrap()));
}

#[test]
fn init_windows_before_init_graf_is_refused_by_name() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.start(&Prog::new().trap(0xA912).exit().0, 0).unwrap();
    m.step(100_000);
    assert!(m.refused.iter().any(|r| r.contains("InitGraf has not run")), "{:?}", m.refused);
}
