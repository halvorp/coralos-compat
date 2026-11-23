//! Unit tests + negative controls for InterfaceLib sound volume, resource size, window box/grow/zoom,
//! Icon Utilities (GetIconSuite / PlotIconSuite), and Standard File Package routines.
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_ui_abi::RecordingGuestUi;

fn call(m: &mut MacPpc<RecordingGuestUi>, name: &str, args: &[u32]) -> (GlueOutcome, u32) {
    m.bind(name);
    let mut regs = [0u32; 32];
    for (i, &v) in args.iter().enumerate() {
        regs[3 + i] = v;
    }
    let out = m.glue_call(name, &mut regs);
    (out, regs[3])
}

#[test]
fn interfacelib_bindings_cover_sound_icons_stdfile_and_window_controls() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).expect("machine");
    for name in [
        "GetDefaultOutputVolume",
        "SetDefaultOutputVolume",
        "GetSoundVol",
        "SetSoundVol",
        "GetMaxResourceSize",
        "GrowWindow",
        "TrackBox",
        "ZoomWindow",
        "GetIconSuite",
        "PlotIconSuite",
        "StandardGetFile",
        "StandardPutFile",
    ] {
        let b = m.bind_from("InterfaceLib", name);
        assert!(b.is_bound(), "expected InterfaceLib/{name} to bind, got {b:?}");
    }
    // Negative control: fabricated symbol stays Unbound
    assert!(matches!(m.bind_from("InterfaceLib", "NoSuchToolboxCall"), Binding::Unbound(_)));
}

#[test]
fn sound_volume_icons_stdfile_and_window_controls_with_negative_controls() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).expect("machine");
    let scratch = m.mac.mm.new_ptr(&mut m.mac.mem, &mut m.mac.heap, 256, true).unwrap();

    // 1) SetDefaultOutputVolume / GetDefaultOutputVolume round-trip
    assert_eq!(call(&mut m, "SetDefaultOutputVolume", &[0x0080_0080]), (GlueOutcome::Return, 0));
    assert_eq!(call(&mut m, "GetDefaultOutputVolume", &[scratch]), (GlueOutcome::Return, 0));
    assert_eq!(m.mac.mem.read_u32(scratch).unwrap(), 0x0080_0080);

    // 2) SetSoundVol / GetSoundVol round-trip (0..7)
    assert_eq!(call(&mut m, "SetSoundVol", &[5]), (GlueOutcome::Return, 0));
    assert_eq!(call(&mut m, "GetSoundVol", &[scratch + 4]), (GlueOutcome::Return, 0));
    assert_eq!(m.mac.mem.read_u16(scratch + 4).unwrap(), 5);

    // 3) NEGATIVE: GetIconSuite on missing resource returns noSuchIconErr (-2580) and writes NIL
    m.mac.mem.write_u32(scratch + 8, 0xDEAD_BEEF).unwrap();
    let (out, r) = call(&mut m, "GetIconSuite", &[scratch + 8, 9999, 0xFF]);
    assert_eq!(out, GlueOutcome::Return);
    assert_eq!(r as i32, -2580);
    assert_eq!(m.mac.mem.read_u32(scratch + 8).unwrap(), 0);

    // 4) NEGATIVE: PlotIconSuite with NULL rect/suite returns paramErr (-50)
    let (out, r) = call(&mut m, "PlotIconSuite", &[0, 0, 0, 0]);
    assert_eq!(out, GlueOutcome::Return);
    assert_eq!(r as i32, -50);

    // 5) StandardGetFile & StandardPutFile zero sfGood (cancelled)
    m.mac.mem.write_u8(scratch + 16, 1).unwrap();
    assert_eq!(call(&mut m, "StandardGetFile", &[0, 0, 0, scratch + 16]).0, GlueOutcome::Return);
    assert_eq!(m.mac.mem.read_u8(scratch + 16).unwrap(), 0);

    m.mac.mem.write_u8(scratch + 128, 1).unwrap();
    assert_eq!(call(&mut m, "StandardPutFile", &[0, 0, scratch + 128]).0, GlueOutcome::Return);
    assert_eq!(m.mac.mem.read_u8(scratch + 128).unwrap(), 0);

    // 6) NEGATIVE: GrowWindow / TrackBox / ZoomWindow on non-open window refuse cleanly
    for (name, args) in [
        ("GrowWindow", &[0x1234_5678u32, 0, 0][..]),
        ("TrackBox", &[0x1234_5678u32, 0, 7][..]),
        ("ZoomWindow", &[0x1234_5678u32, 7, 1][..]),
    ] {
        m.mac.refused.clear();
        let _ = call(&mut m, name, args);
        assert!(m.mac.refused.iter().any(|s| s.contains("not an open window")), "{name}: {:?}", m.mac.refused);
    }
}
