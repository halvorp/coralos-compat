//! Carbon QuickDraw accessors and small routines (specs: macos9-specs/Quickdraw/*): the QD globals copied out, the clip region copied, the pen location, DeltaPoint, GetIndPattern, GetTime.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::toolbox::WindowSpec;
use heirloom_fmt_resfork::writer::ForkBuilder;

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3300;
const R: u32 = 0x3400;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}
fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    let spec = WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 220, right: 250 }, title: "W".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: true, ref_con: 0 };
    let win = m.mac.make_window(spec).unwrap();
    (m, win)
}

#[test]
fn the_qd_globals_are_copied_out_and_the_installers_weak_accessor_answers() {
    let (mut m, _) = machine();
    assert_eq!(call(&mut m, "GetQDGlobalsArrow", &[OUT]), OUT);
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap(), 0x0000);
    assert_eq!(m.mac.mem.read_u16(OUT + 2).unwrap(), 0x4000, "row 1 of the arrow");
    assert_eq!((m.mac.mem.read_u16(OUT + 64).unwrap(), m.mac.mem.read_u16(OUT + 66).unwrap()), (1, 1), "hot spot (1,1)");
    assert_eq!(call(&mut m, "GetQDGlobalsScreenBits", &[OUT]), OUT);
    let gd = m.mac.mm.block(m.mac.cqd.main_device).unwrap().ptr;
    assert_eq!(m.mac.mem.read_u16(OUT + 10).unwrap(), m.mac.mem.read_u16(gd + 0x22 + 4).unwrap(), "screenBits.bounds.bottom is the main device's");
    assert_eq!(call(&mut m, "GetQDGlobalsBlack", &[OUT]), OUT);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xFFFF_FFFF);
    call(&mut m, "GetQDGlobalsGray", &[OUT]);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xAA55_AA55);
    assert_eq!(call(&mut m, "GetQDGlobalsRandomSeed", &[]), 1);
    call(&mut m, "SetQDGlobalsRandomSeed", &[77]);
    assert_eq!(call(&mut m, "GetQDGlobalsRandomSeed", &[]), 77);
    // NEGATIVE control: before InitGraf there are no globals, said by name
    let mut m2 = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m2.bind("GetQDGlobalsArrow");
    let mut regs = [0u32; 32];
    regs[3] = OUT;
    assert!(matches!(m2.glue_call("GetQDGlobalsArrow", &mut regs), GlueOutcome::Refused(w) if w.contains("not initialised")));
}

#[test]
fn port_accessors_deltapoint_indpattern_gettime_and_a5() {
    let (mut m, win) = machine();
    // GetPortPenLocation(NULL = the current port, &pt): the pen is at (30, 40) after a MoveTo
    call(&mut m, "SetPort", &[win]);
    call(&mut m, "MoveTo", &[40, 30]);
    assert_eq!(call(&mut m, "GetPortPenLocation", &[0, OUT]), OUT);
    assert_eq!((m.mac.mem.read_u16(OUT).unwrap(), m.mac.mem.read_u16(OUT + 2).unwrap()), (30, 40), "{{v 30, h 40}}");
    // NEGATIVE: a NULL point sets the error latch to paramErr and returns it untouched
    assert_eq!(call(&mut m, "GetPortPenLocation", &[0, 0]), 0);
    assert_eq!(m.mac.cqd.qd_error, -50);
    // clip region: copied out into a region the caller owns, and back
    let rgn = call(&mut m, "NewRgn", &[]);
    assert_eq!(call(&mut m, "GetPortClipRegion", &[win, rgn]), rgn);
    assert_eq!(call(&mut m, "GetPortClipRegion", &[win, 0]), 0, "a NULL region is refused with the error latch");
    call(&mut m, "SetPortClipRegion", &[win, rgn]);
    // DeltaPoint(0x00140050, 0x000A0020) = 0x000A0030 (spec vector)
    assert_eq!(call(&mut m, "DeltaPoint", &[0x0014_0050, 0x000A_0020]), 0x000A_0030);
    // GetIndPattern: PAT# 0 of three patterns; index 3 gives the third; index 0 / 4 leave the destination unchanged
    let mut pats = vec![0u8, 3];
    for i in 0..3u8 { pats.extend([i + 1; 8]); }
    let fork = ForkBuilder::new().add(b"PAT#", 0, None, &pats).build();
    m.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork).unwrap());
    m.mac.mem.write_bytes(OUT, &[0xEE; 8]).unwrap();
    call(&mut m, "GetIndPattern", &[OUT, 0, 3]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 8).unwrap()).unwrap(), &[3u8; 8]);
    m.mac.mem.write_bytes(OUT, &[0xEE; 8]).unwrap();
    call(&mut m, "GetIndPattern", &[OUT, 0, 0]);
    call(&mut m, "GetIndPattern", &[OUT, 0, 4]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 8).unwrap()).unwrap(), &[0xEEu8; 8]);
    // GetTime = SecondsToDate(Time): 1904-01-01 + 86400 s is 1904-01-02 00:00:00
    m.mac.mem.write_u32(heirloom_mac68k_engine::lowmem::TIME, 86_400).unwrap();
    call(&mut m, "GetTime", &[OUT]);
    assert_eq!((m.mac.mem.read_u16(OUT).unwrap(), m.mac.mem.read_u16(OUT + 2).unwrap(), m.mac.mem.read_u16(OUT + 4).unwrap()), (1904, 1, 2));
    // SetA5 returns the old value
    assert_eq!(call(&mut m, "SetA5", &[0x1234]), 0);
    assert_eq!(call(&mut m, "SetA5", &[0x5678]), 0x1234);
    assert_eq!(call(&mut m, "RestoreDeviceClut", &[0]), 0);
}
