//! graphics.library's display database over CoralOS's ONE mode (0x29000): DTAG_DIMS and DTAG_NAME records say what the screen really is
//! (SCREEN_WIDTH x SCREEN_HEIGHT x SCREEN_DEPTH, no overscan), and BestModeIDA applies bestmodeida.c's filters to it. Each answer has its
//! negative control: a request the one screen cannot meet gets INVALID_ID, an unknown mode and an unmodelled record kind are refused by name.
mod gfxcommon;
use gfxcommon::*;

const GET_DISPLAY_INFO_DATA: i32 = -756;
const BEST_MODE_IDA: i32 = -1050;
const MODE: u32 = 0x29000;
const BID_MUST_HAVE: u32 = 0x8000_0001;
const BID_MUST_NOT_HAVE: u32 = 0x8000_0002;
const BID_NOMINAL_W: u32 = 0x8000_0004;
const BID_NOMINAL_H: u32 = 0x8000_0005;
const BID_DESIRED_W: u32 = 0x8000_0006;
const BID_DESIRED_H: u32 = 0x8000_0007;
const BID_DEPTH: u32 = 0x8000_0008;
const BID_MONITOR: u32 = 0x8000_0009;

fn best(g: &mut G, tags: &[(u32, u32)]) -> u32 {
    let t = g.tags(tags);
    g.run(&[gfx(BEST_MODE_IDA, &[a(0, t)])]);
    g.result(0)
}

#[test]
fn dimension_and_name_records_describe_the_one_screen() {
    let mut g = G::new();
    let (dims, name, disp) = (g.zeroed(88), g.zeroed(56), g.zeroed(56));
    g.run(&[gfx(GET_DISPLAY_INFO_DATA, &[a(0, 0), a(1, dims), d(0, 88), d(1, 0x8000_1000), d(2, MODE)]),
            gfx(GET_DISPLAY_INFO_DATA, &[a(0, 0), a(1, name), d(0, 56), d(1, 0x8000_3000), d(2, MODE)]),
            gfx(GET_DISPLAY_INFO_DATA, &[a(0, 0), a(1, disp), d(0, 56), d(1, 0x8000_0000), d(2, MODE)]),
            gfx(GET_DISPLAY_INFO_DATA, &[a(0, 0), a(1, dims), d(0, 88), d(1, 0x8000_1000), d(2, 0x1234_0000)])]);
    assert_eq!((g.result(0), g.result(1), g.result(2)), (88, 56, 56), "the byte counts of DimensionInfo, NameInfo, DisplayInfo");
    assert_eq!(g.result(3), 0, "NEGATIVE CONTROL: a mode the database does not have answers 0");
    assert_eq!(g.r32(dims), 0x8000_1000, "QueryHeader.StructID");
    assert_eq!(g.r32(dims + 12), 22, "QueryHeader.Length in longwords");
    assert_eq!((g.r16(dims + 16), g.r16(dims + 18), g.r16(dims + 20), g.r16(dims + 22), g.r16(dims + 24)), (8, 640, 256, 640, 256), "MaxDepth, Min/Max raster");
    for off in [26, 34, 42, 50, 58] { assert_eq!((g.r16(dims + off), g.r16(dims + off + 2), g.r16(dims + off + 4), g.r16(dims + off + 6)), (0, 0, 639, 255), "rectangle at {off}: no overscan"); }
    assert_eq!(g.r8(name + 16), b'C', "the mode's name");
    assert!(g.a.mem.read_cstr(name + 16).unwrap().contains("640x256"));
    assert_eq!((g.r8(disp + 40), g.r8(disp + 41), g.r8(disp + 42)), (8, 8, 8), "8 bits per gun: the host palette is 8-bit RGB");
    assert_eq!(g.r32(disp + 18) & (1 << 8), 1 << 8, "DIPF_IS_WB");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

#[test]
fn an_unmodelled_record_kind_is_refused_by_name() {
    let mut g = G::new_plane();
    let buf = g.zeroed(64);
    g.a.start_hunk(&script(&[gfx(GET_DISPLAY_INFO_DATA, &[a(0, 0), a(1, buf), d(0, 64), d(1, 0x8000_2000), d(2, MODE)])])).unwrap();
    g.a.step(1_000_000);
    assert!(g.a.refused.iter().any(|r| r.contains("record kind 0x80002000 is not modelled")), "{:?}", g.a.refused);
}

#[test]
fn best_mode_id_applies_the_database_filters_to_the_one_mode() {
    let mut g = G::new();
    assert_eq!(best(&mut g, &[]), MODE, "no requirements: the screen's own mode");
    assert_eq!(best(&mut g, &[(BID_DESIRED_W, 640), (BID_DESIRED_H, 200), (BID_DEPTH, 4)]), MODE, "a smaller, shallower request fits");
    assert_eq!(best(&mut g, &[(BID_MONITOR, 0x29000)]), MODE, "its own monitor");
    assert_eq!(best(&mut g, &[(BID_MONITOR, 0x21000)]), MODE, "PAL_MONITOR_ID: the mask passes accept the same monitor class");
    // NEGATIVE CONTROLS: what the one screen cannot meet.
    assert_eq!(best(&mut g, &[(BID_NOMINAL_W, 640), (BID_NOMINAL_H, 256), (BID_DEPTH, 24)]), u32::MAX, "deeper than the screen");
    // (bestmodeida.c: with no NOMINAL size the depth is taken from the default monitor's nominal dimensions, so a bare depth is not a request.)
    assert_eq!(best(&mut g, &[(BID_DEPTH, 24)]), MODE, "a bare depth falls back to the monitor's nominal depth, as AROS does");
    assert_eq!(best(&mut g, &[(BID_DESIRED_W, 1280), (BID_DESIRED_H, 256)]), u32::MAX, "wider than the screen");
    assert_eq!(best(&mut g, &[(BID_DESIRED_W, 640), (BID_DESIRED_H, 1024)]), u32::MAX, "taller than the screen");
    assert_eq!(best(&mut g, &[(BID_MONITOR, 0x00041000)]), u32::MAX, "another monitor (NTSC_MONITOR_ID has other bits)");
    assert_eq!(best(&mut g, &[(BID_MUST_HAVE, 1 << 3)]), u32::MAX, "HAM is not a property of this mode");
    assert_eq!(best(&mut g, &[(BID_MUST_NOT_HAVE, 1 << 8)]), u32::MAX, "the mode IS the Workbench screen's");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

mod aros;
use heirloom_ami68k_engine::Step;

/// The REAL ScreenMode prefs editor: it lists the database (NextDisplayInfo), reads NameInfo/DisplayInfo/DimensionInfo, asks BestModeID when its
/// saved mode is not in the database, and sets its own widgets with `nnset` (MUIA_NoNotify): before NoNotify silenced the whole call this
/// recursed through its own notifications until its stack ran into its data.
#[test]
fn the_screen_mode_prefs_editor_opens_over_the_one_mode() {
    let Some((a, s)) = aros::start("Prefs/ScreenMode", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(a.ui.controls.len() > 20, "its window has its controls: {}", a.ui.controls.len());
    let has = |p: &str| a.log.iter().any(|l| l.starts_with(p));
    assert!(has("[graphics] BestModeIDA") && has("[graphics] GetDisplayInfoData 0x80001000") && has("[graphics] GetDisplayInfoData 0x80003000"));
    // NEGATIVE CONTROL: the stack never reached the program's image (the recursion this guards against).
    assert!(a.log.len() < 20_000, "{} log lines: a notification loop?", a.log.len());
}

const QUERY_OVERSCAN: i32 = -474;

/// DOpus's ViewFont asks QueryOverscan before it opens its screen (rom/intuition/queryoverscan.c): the answer is the mode's DimensionInfo
/// rectangle for the type, TRUE; the one screen has no overscan, so every type is the whole raster.
#[test]
fn query_overscan_answers_the_mode_rectangle_and_refuses_what_it_does_not_have() {
    let mut g = G::new();
    let rects: Vec<u32> = (0..6).map(|_| g.bytes(&[0xAA; 8])).collect();
    g.run(&[intu(QUERY_OVERSCAN, &[a(0, MODE), a(1, rects[0]), d(0, 1)]),
            intu(QUERY_OVERSCAN, &[a(0, MODE), a(1, rects[1]), d(0, 2)]),
            intu(QUERY_OVERSCAN, &[a(0, MODE), a(1, rects[2]), d(0, 3)]),
            intu(QUERY_OVERSCAN, &[a(0, MODE), a(1, rects[3]), d(0, 4)]),
            intu(QUERY_OVERSCAN, &[a(0, 0x1234_0000), a(1, rects[4]), d(0, 1)]),
            intu(QUERY_OVERSCAN, &[a(0, MODE), a(1, rects[5]), d(0, 9)])]);
    for i in 0..4 {
        assert_eq!(g.result(i), 1, "type {} is TRUE", i + 1);
        let r = rects[i as usize];
        assert_eq!((g.r16(r), g.r16(r + 2), g.r16(r + 4), g.r16(r + 6)), (0, 0, 639, 255), "type {} rectangle", i + 1);
    }
    // NEGATIVE CONTROLS: an unknown mode and an unknown type answer FALSE and leave the rectangle alone.
    assert_eq!((g.result(4), g.result(5)), (0, 0));
    assert_eq!((g.r32(rects[4]), g.r32(rects[5])), (0xAAAA_AAAA, 0xAAAA_AAAA));
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}
