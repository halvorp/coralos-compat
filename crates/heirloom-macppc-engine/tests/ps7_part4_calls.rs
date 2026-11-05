//! The calls Photoshop 7.0 reached after its Text Encoding Converter start-up (part 4), each against its contract with a negative control:
//! FSGetVolumeInfo, MPPOpen on a machine with no AppleTalk, SetPreserveGlyph, IsMetric (one body with IUMetric), NavLoad / NavUnload,
//! ShowHide, SetWinColor / GetAuxWin, GetWMgrPort / GetCWMgrPort (InstallWindowContentPaintProc: wclass_c's unit test).
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3000;
const INFO: u32 = 0x3100;
const NAME: u32 = 0x3300;
const ROOT: u32 = 0x3600;
const R: u32 = 0x3800;
const T: u32 = 0x3900;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[1] = 0x7000;
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
/// A Toolbox trap refused by name: the call returns to the program and the refusal is recorded.
fn refused(m: &mut M, name: &str, args: &[u32]) -> String {
    let before = m.mac.refused.len();
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    m.bind(name);
    let _ = m.glue_call(name, &mut regs);
    m.mac.refused.get(before..).map(|r| r.join("; ")).unwrap_or_default()
}
fn word(m: &M, at: u32) -> u32 { m.mac.mem.read_u32(at).unwrap() }

/// InitGraf + InitWindows, and an invisible colour window 0..100 x 0..80 at (40, 40).
fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    call(&mut m, "InitWindows", &[]);
    for (i, v) in [40i16, 40, 120, 140].iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, *v as u16).unwrap(); }
    m.mac.mem.write_u8(T, 0).unwrap();
    let w = call(&mut m, "NewCWindow", &[0, R, T, 0, 0, u32::MAX, 0, 0]) as u32;
    assert!(m.mac.windows.contains_key(&w));
    (m, w)
}

#[test]
fn fs_get_volume_info_by_index_and_by_ref() {
    let (mut m, _) = machine();
    m.mac.mem.zero(INFO, 126).unwrap();
    // FSGetVolumeInfo(volume, index, &actual, whichInfo, &info, &name, &rootRef): the first volume by index, sizes + blocks + FS info
    assert_eq!(call(&mut m, "FSGetVolumeInfo", &[0, 1, OUT, 0x40 | 0x80 | 0x4000, INFO, NAME, ROOT]), 0);
    let vref = m.mac.mem.read_u16(OUT).unwrap() as i16;
    assert_eq!(vref, -1);
    let block = word(&m, INFO + 56);
    let total = (u64::from(word(&m, INFO + 40)) << 32) | u64::from(word(&m, INFO + 44));
    assert!(block >= 512 && total == u64::from(word(&m, INFO + 60)) * u64::from(block), "totalBytes = totalBlocks x blockSize: {total} {block}");
    assert_eq!(m.mac.mem.read_u16(INFO + 120).unwrap(), 0x4244, "signature 'BD'");
    // the name: an HFSUniStr255 of the volume's Mac Roman name
    let n = m.mac.mem.read_u16(NAME).unwrap();
    let name: String = (0..n).map(|i| char::from_u32(u32::from(m.mac.mem.read_u16(NAME + 2 + 2 * u32::from(i)).unwrap())).unwrap()).collect();
    assert_eq!(name, heirloom_mac68k_engine::macroman::decode(&m.mac.fm.vol_info(m.mac.fm.def_vol).name));
    // fields not asked for are left alone; by vRefNum, the same volume
    m.mac.mem.write_u32(INFO + 32, 0xDEAD).unwrap();
    assert_eq!(call(&mut m, "FSGetVolumeInfo", &[vref as u16 as u32, 0, 0, 0x40, INFO, 0, 0]), 0);
    assert_eq!(word(&m, INFO + 32), 0xDEAD);
    // NEGATIVES: an index past the last volume and an unknown vRefNum are nsvErr; volume 0 with index 0 names nothing
    assert_eq!(call(&mut m, "FSGetVolumeInfo", &[0, 99, OUT, 0, 0, 0, 0]), -35);
    assert_eq!(call(&mut m, "FSGetVolumeInfo", &[(-77i16) as u16 as u32, 0, OUT, 0, 0, 0, 0]), -35);
    assert_eq!(call(&mut m, "FSGetVolumeInfo", &[0, 0, OUT, 0, 0, 0, 0]), -35);
}

#[test]
fn small_toolbox_answers() {
    let (mut m, _) = machine();
    // AppleTalk: inactive on this machine, portNotCf, named in the log
    assert_eq!(call(&mut m, "MPPOpen", &[]), -98);
    assert!(m.mac.log.iter().any(|l| l.contains("[appletalk] MPPOpen -> portNotCf")));
    // SetPreserveGlyph / GetPreserveGlyph: recorded and answered
    assert_eq!(call(&mut m, "GetPreserveGlyph", &[]) & 0xFF, 0);
    call(&mut m, "SetPreserveGlyph", &[1]);
    assert_eq!(call(&mut m, "GetPreserveGlyph", &[]) & 0xFF, 1);
    // IsMetric answers from the itl0 GetIntlResource(0) gives (metricSys at 29): one source with IUMetric
    let itl0 = call(&mut m, "GetIntlResource", &[0]) as u32;
    let p = word(&m, itl0);
    let expect = m.mac.mem.read_u8(p + 29).unwrap() == 0xFF;
    assert_eq!(call(&mut m, "IsMetric", &[]) & 0xFF != 0, expect);
    m.mac.mem.write_u8(p + 29, 0xFF).unwrap();
    assert_eq!(call(&mut m, "IsMetric", &[]) & 0xFF, 1, "the record decides, not a constant");
    // NavLoad / NavUnload: noErr (the Nav calls are natives)
    assert_eq!(call(&mut m, "NavLoad", &[]), 0);
    assert_eq!(call(&mut m, "NavUnload", &[]), 0);
}

#[test]
fn window_calls_show_hide_colours_ports_and_paint_procs() {
    let (mut m, w) = machine();
    // ShowHide: visibility only
    call(&mut m, "ShowHide", &[w, 1]);
    assert!(m.mac.windows[&w].visible && m.mac.windows[&w].native.is_some());
    call(&mut m, "ShowHide", &[w, 0]);
    assert!(!m.mac.windows[&w].visible);
    // NEGATIVE: GetAuxWin of a window with no colour table of its own needs a default 'wctb' this machine lacks: refused by name
    assert!(refused(&mut m, "GetAuxWin", &[w, OUT]).contains("default 'wctb'"));
    // SetWinColor keeps the program's handle; GetAuxWin answers TRUE with a record naming it
    let wctab = call(&mut m, "NewHandleClear", &[48]) as u32;
    call(&mut m, "SetWinColor", &[w, wctab]);
    assert_eq!(call(&mut m, "GetAuxWin", &[w, OUT]) & 0xFF, 1);
    let rec = word(&m, word(&m, OUT));
    assert_eq!((word(&m, rec + 4), word(&m, rec + 8)), (w, wctab), "awOwner, awCTable");
    // GetWMgrPort is InitWindows' port; GetCWMgrPort a CGrafPort made once, thePort unchanged
    call(&mut m, "SetPort", &[w]);
    call(&mut m, "GetWMgrPort", &[OUT]);
    assert_eq!(word(&m, OUT), m.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::WMGR_PORT).unwrap());
    call(&mut m, "GetCWMgrPort", &[OUT]);
    let cport = word(&m, OUT);
    assert!(cport != 0 && m.mac.mem.read_u16(cport + 6).unwrap() & 0xC000 == 0xC000, "a CGrafPort (portVersion)");
    call(&mut m, "GetCWMgrPort", &[OUT + 4]);
    assert_eq!(word(&m, OUT + 4), cport, "made once");
    call(&mut m, "GetPort", &[OUT]);
    assert_eq!(word(&m, OUT), w, "thePort is still the program's");
    // (InstallWindowContentPaintProc is served by the run loop: its test is wclass_c's unit test)
}
