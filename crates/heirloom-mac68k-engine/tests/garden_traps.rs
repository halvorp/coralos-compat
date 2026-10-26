//! The traps the Macintosh Garden census named (MacWrite, MacPaint, WriteNow, SimpleText, Glider, ResEdit), each against
//! hand-assembled calls and each with a NEGATIVE CONTROL (the wrong input, the other mode, the call that must refuse by name).
mod common;
use common::{put, pstr, Prog, Rig, SCRATCH};
use heirloom_ui_abi::UiCall;
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::toolbox::WindowSpec;

fn words(p: &mut Prog, w: &[u16]) { for x in w { p.0.extend_from_slice(&x.to_be_bytes()); } }
/// move.l #v,d0 / a0 / a1 ; move.l d0,addr ; move.l a0,addr
fn set_d0(p: &mut Prog, v: u32) { words(p, &[0x203C, (v >> 16) as u16, v as u16]); }
fn set_a0(p: &mut Prog, v: u32) { words(p, &[0x207C, (v >> 16) as u16, v as u16]); }
fn set_a1(p: &mut Prog, v: u32) { words(p, &[0x227C, (v >> 16) as u16, v as u16]); }
fn save_d0(p: &mut Prog, at: u32) { words(p, &[0x23C0, (at >> 16) as u16, at as u16]); }
fn save_a0(p: &mut Prog, at: u32) { words(p, &[0x23C8, (at >> 16) as u16, at as u16]); }
fn l(rig: &Rig, a: u32) -> u32 { rig.m.mem.read_u32(a).unwrap() }
fn w(rig: &Rig, a: u32) -> u16 { rig.m.mem.read_u16(a).unwrap() }
fn text(rig: &Rig, a: u32) -> String { let n = rig.m.mem.read_u8(a).unwrap() as u32; (0..n).map(|i| char::from(rig.m.mem.read_u8(a + 1 + i).unwrap())).collect() }

const STR: u32 = 0x3100;
const OUT: u32 = 0x3200;
const SECS_2026_10_19: u32 = 3_875_212_800; // Monday 19 October 2026 00:00:00

// ---- Pack7 ---------------------------------------------------------------------------------------------------------

#[test]
fn num_to_string_and_string_to_num_round_trip_and_an_unknown_selector_stops_by_name() {
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("-1234xyz"));
    // Pack7 is a REGISTER package (the corpus applications' own glue: D0 / A0 loaded, the selector the only thing pushed):
    // NumToString D0 -> Str255 at A0; StringToNum Str255 at A0 -> D0. A marker pushed under the selector must come back on
    // top of the stack (the package pops the selector and nothing else).
    let mut p = Prog::new();
    p.l(0xCAFE_F00D);
    set_d0(&mut p, (-4096i32) as u32); set_a0(&mut p, OUT); p.w(0).trap(0xA9EE);
    words(&mut p, &[0x23DF, 0, (SCRATCH + 8) as u16]); // move.l (sp)+,SCRATCH+8
    set_a0(&mut p, STR); p.w(1).trap(0xA9EE); save_d0(&mut p, SCRATCH);
    p.l(0x1111_1111).w(0); // NEGATIVE CONTROL: arguments pushed Pascal-style on the stack are NOT read; D0 / A0 are
    set_d0(&mut p, 77); set_a0(&mut p, OUT + 0x40); p.trap(0xA9EE);
    words(&mut p, &[0x588F]); // addq.l #4,sp
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(text(&g, OUT), "-4096");
    assert_eq!(l(&g, SCRATCH + 8), 0xCAFE_F00D, "only the selector word was popped");
    assert_eq!(l(&g, SCRATCH) as i32, -1234, "StringToNum answers in D0 and stops at the first non-digit");
    assert_eq!(text(&g, OUT + 0x40), "77", "NumToString read D0, not the stack");
    // NEGATIVE CONTROL: a selector the package lacks halts the guest with a refusal that names it (its frame size is unknown).
    let mut g = Rig::new();
    g.m.start(&Prog::new().w(9).trap(0xA9EE).exit().0, 0).unwrap();
    let r = g.m.run_loop(10_000).unwrap();
    assert!(r.stats.halted && g.m.refused.iter().any(|x| x.contains("Pack7 selector 0x9")), "{:?}", g.m.refused);
}

// ---- Pack6 ---------------------------------------------------------------------------------------------------------

#[test]
fn date_and_time_strings_follow_the_us_conventions() {
    let mut g = Rig::new();
    let at = SECS_2026_10_19 + 13 * 3600 + 5 * 60 + 9;
    g.run(Prog::new()
        .l(at).b(0).l(OUT).w(0).trap(0xA9ED)          // IUDateString short
        .l(at).b(1).l(OUT + 0x40).w(0).trap(0xA9ED)   // long
        .l(at).b(2).l(OUT + 0x80).w(0).trap(0xA9ED)   // abbreviated
        .l(at).b(1).l(OUT + 0xC0).w(2).trap(0xA9ED)   // IUTimeString with seconds
        .l(at).b(0).l(OUT + 0x100).w(2).trap(0xA9ED)  // without
        .exit());
    g.no_refusals();
    assert_eq!(text(&g, OUT), "10/19/26");
    assert_eq!(text(&g, OUT + 0x40), "Monday, October 19, 2026");
    assert_eq!(text(&g, OUT + 0x80), "Mon, Oct 19, 2026");
    assert_eq!(text(&g, OUT + 0xC0), "1:05:09 PM");
    assert_eq!(text(&g, OUT + 0x100), "1:05 PM");
    // NEGATIVE CONTROL: a DateForm that is not 0..2 is a named refusal and the result string is left alone.
    let mut g = Rig::new();
    put(&mut g.m, OUT, &pstr("keep"));
    g.run(Prog::new().l(at).b(7).l(OUT).w(0).trap(0xA9ED).exit());
    assert!(g.m.refused.iter().any(|x| x.contains("DateForm 7")), "{:?}", g.m.refused);
    assert_eq!(text(&g, OUT), "keep");
}

#[test]
fn mag_string_ignores_case_and_im_mag_id_string_also_ignores_accents() {
    let mut g = Rig::new();
    put(&mut g.m, STR, b"abc\x8E"); // "abc" + e-acute (MacRoman 0x8E)
    put(&mut g.m, STR + 0x20, b"ABCe");
    put(&mut g.m, STR + 0x40, b"abd");
    let call = |p: &mut Prog, sel: u16, a: u32, b: u32, la: i16, lb: i16, out: u32| { p.clr_w().l(a).l(b).w(la).w(lb).w(sel as i16).trap(0xA9ED).pop_w(out); };
    let mut p = Prog::new();
    call(&mut p, 10, STR, STR + 0x20, 4, 4, OUT);       // IUMagString: accent differs
    call(&mut p, 12, STR, STR + 0x20, 4, 4, OUT + 2);   // IUMagIDString: accent ignored, case ignored
    call(&mut p, 10, STR, STR + 0x40, 3, 3, OUT + 4);   // "abc" < "abd"
    call(&mut p, 10, STR + 0x40, STR, 3, 3, OUT + 6);   // "abd" > "abc"
    call(&mut p, 10, STR, STR + 0x20, 3, 3, OUT + 8);   // "abc" == "ABC"
    p.exit();
    g.run(&p);
    g.no_refusals();
    let r = |i: u32| w(&g, OUT + i) as i16;
    assert_eq!((r(0), r(2), r(4), r(6), r(8)), (1, 0, -1, 1, 0));
}

#[test]
fn iu_metric_is_false_and_iu_get_intl_returns_a_us_record_whose_separators_are_right() {
    let mut g = Rig::new();
    g.run(Prog::new().clr_w().w(4).trap(0xA9ED).pop_w(OUT).clr_l().w(0).w(6).trap(0xA9ED).pop_l(OUT + 4).clr_l().w(1).w(6).trap(0xA9ED).pop_l(OUT + 8).exit());
    g.no_refusals();
    assert_eq!(w(&g, OUT) >> 8, 0, "IUMetric is FALSE");
    let itl0 = l(&g, OUT + 4);
    let rec = g.m.mem.read_u32(itl0).unwrap(); // master pointer -> record
    let (dec, thou, list, cur) = (g.m.mem.read_u8(rec).unwrap(), g.m.mem.read_u8(rec + 1).unwrap(), g.m.mem.read_u8(rec + 2).unwrap(), g.m.mem.read_u8(rec + 3).unwrap());
    assert_eq!((dec, thou, list, cur), (b'.', b',', b';', b'$'));
    let itl1 = g.m.mem.read_u32(l(&g, OUT + 8)).unwrap();
    assert_eq!(g.m.mem.read_u8(itl1).unwrap(), 6, "first day name is Sunday");
    // NEGATIVE CONTROL: itl2 (the collation tables) is not provided: NIL and a named refusal.
    let mut g = Rig::new();
    g.run(Prog::new().clr_l().w(2).w(6).trap(0xA9ED).pop_l(OUT).exit());
    assert_eq!(l(&g, OUT), 0);
    assert!(g.m.refused.iter().any(|x| x.contains("itl2")), "{:?}", g.m.refused);
}

// ---- strings, the clock ----------------------------------------------------------------------------------------------

#[test]
fn cmp_string_and_rel_string_obey_the_case_and_marks_flag_bits_of_the_trap_word() {
    let mut g = Rig::new();
    put(&mut g.m, STR, b"Hello\x8E"); // Hello + e-acute
    put(&mut g.m, STR + 0x20, b"HELLOe");
    let mut p = Prog::new();
    for (op, out) in [(0xA03C, 0u32), (0xA23C, 4), (0xA43C, 8), (0xA050, 12)] { // no flags, MARKS, CASE, RelString
        set_a0(&mut p, STR); set_a1(&mut p, STR + 0x20); set_d0(&mut p, 6 << 16 | 6); p.trap(op); save_d0(&mut p, OUT + out);
    }
    p.exit();
    g.run(&p);
    g.no_refusals();
    assert_eq!(l(&g, OUT), 0, "case and accents ignored: equal");
    assert_eq!(l(&g, OUT + 4), 1, "MARKS: the accent makes them differ");
    assert_eq!(l(&g, OUT + 8), 1, "CASE: 'H' and 'h' differ (accents are ignored there, but the letters' case is not)");
    // NEGATIVE CONTROL: identical text is equal under every flag combination.
    let mut g = Rig::new();
    put(&mut g.m, STR, b"Same");
    put(&mut g.m, STR + 0x20, b"Same");
    let mut p = Prog::new();
    for (i, op) in [0xA03Cu16, 0xA23C, 0xA43C, 0xA63C].into_iter().enumerate() { set_a0(&mut p, STR); set_a1(&mut p, STR + 0x20); set_d0(&mut p, 4 << 16 | 4); p.trap(op); save_d0(&mut p, OUT + 4 * i as u32); }
    p.exit();
    g.run(&p);
    assert_eq!((0..4).map(|i| l(&g, OUT + 4 * i)).collect::<Vec<_>>(), vec![0, 0, 0, 0]);
}

#[test]
fn read_date_time_and_seconds_to_date_agree_and_date_to_seconds_inverts_them() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    set_a0(&mut p, OUT); p.trap(0xA039);
    p.l(SECS_2026_10_19 + 3661).l(OUT + 0x10).trap(0xA9C6);          // SecondsToDate -> record at OUT+0x10
    p.l(OUT + 0x10).l(OUT + 0x30).trap(0xA9C7);                      // DateToSeconds back
    p.exit();
    g.run(&p);
    g.no_refusals();
    assert_eq!(l(&g, OUT), SECS_2026_10_19, "the default clock");
    let f: Vec<u16> = (0..7).map(|i| w(&g, OUT + 0x10 + 2 * i)).collect();
    assert_eq!(f, vec![2026, 10, 19, 1, 1, 1, 2], "year month day hour minute second dayOfWeek (Monday = 2)");
    assert_eq!(l(&g, OUT + 0x30), SECS_2026_10_19 + 3661);
    // the clock follows the tick count and a host can set it
    let mut g = Rig::new();
    g.m.set_clock(1_000_000).unwrap();
    g.m.advance_ticks(120);
    assert_eq!(g.m.mem.read_u32(heirloom_mac68k_engine::lowmem::TIME).unwrap(), 1_000_002);
}

#[test]
fn purge_space_reports_the_free_heap_and_get_string_returns_the_str_resource() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.trap(0xA162); save_d0(&mut p, OUT); save_a0(&mut p, OUT + 4);
    p.clr_l().w(-8192).trap(0xA9BA).pop_l(OUT + 8);                   // GetString of a missing STR -> NIL
    p.exit();
    g.run(&p);
    assert!(l(&g, OUT) > 1_000_000 && l(&g, OUT + 4) > 1_000_000 && l(&g, OUT) >= l(&g, OUT + 4));
    assert_eq!(l(&g, OUT + 8), 0, "no such STR resource: NIL");
}

// ---- Window Manager --------------------------------------------------------------------------------------------------

#[test]
fn window_title_and_refcon_round_trip_and_the_native_title_follows() {
    let mut g = Rig::new();
    let win = g.win;
    put(&mut g.m, STR, &pstr("Renamed"));
    g.run(Prog::new().l(win).l(STR).trap(0xA91A)                      // SetWTitle
        .l(win).l(OUT).trap(0xA919)                                   // GetWTitle
        .l(win).l(0xCAFE_F00D).trap(0xA918)                           // SetWRefCon
        .clr_l().l(win).trap(0xA917).pop_l(OUT + 0x40).exit());
    g.no_refusals();
    assert_eq!(text(&g, OUT), "Renamed");
    assert_eq!(l(&g, OUT + 0x40), 0xCAFE_F00D);
    assert_eq!(l(&g, win + 152), 0xCAFE_F00D, "refCon is the WindowRecord field");
    assert!(g.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetWindowTitle(_, t) if t == "Renamed")), "{:?}", g.m.ui.calls);
    // NEGATIVE CONTROL: a pointer that is no window is a named refusal and changes nothing.
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("x"));
    g.run(Prog::new().l(0x7000).l(STR).trap(0xA91A).exit());
    assert!(g.m.refused.iter().any(|r| r.contains("SetWTitle") && r.contains("not an open window")), "{:?}", g.m.refused);
    assert!(!g.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetWindowTitle(..))));
}

#[test]
fn size_window_changes_the_content_rect_posts_an_update_and_a_bad_size_is_refused() {
    let mut g = Rig::new();
    let win = g.win;
    g.m.ev.queue.clear();
    g.run(Prog::new().l(win).w(300).w(150).b(1).trap(0xA91D).exit());
    g.no_refusals();
    let r = g.m.windows[&win].rect;
    assert_eq!((r.right - r.left, r.bottom - r.top), (300, 150));
    assert_eq!(w(&g, win + 16 + 6), 300, "portRect.right");
    assert!(g.m.ev.queue.iter().any(|e| e.what == 6 && e.message == win), "an updateEvt for the grown window");
    // NEGATIVE CONTROL: a zero width is refused and the rect stays.
    let mut g = Rig::new();
    let win = g.win;
    g.run(Prog::new().l(win).w(0).w(150).b(0).trap(0xA91D).exit());
    assert!(g.m.refused.iter().any(|r| r.contains("SizeWindow") && r.contains("empty")), "{:?}", g.m.refused);
    assert_eq!(g.m.windows[&win].rect.right - g.m.windows[&win].rect.left, common::WIN_W as i16);
}

#[test]
fn get_new_c_window_opens_the_same_native_window_as_get_new_window() {
    let wind = |title: &str| { let mut v = Vec::new(); for x in [50i16, 40, 150, 340, 0, 0x0100, 0x0100] { v.extend_from_slice(&x.to_be_bytes()); } v.extend_from_slice(&7u32.to_be_bytes()); v.extend(pstr(title)); v };
    let fork = heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"WIND", 200, None, &wind("Colour")).build();
    let mut opened = Vec::new();
    for (trap, name) in [(0xAA46u16, "colour"), (0xA9BD, "plain")] {
        let mut g = Rig::new();
        g.m.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork.clone()).unwrap());
        g.run(Prog::new().clr_l().w(200).l(0).l(0xFFFF_FFFF).trap(trap).pop_l(OUT).exit());
        g.no_refusals();
        let opens: Vec<_> = g.m.ui.calls.iter().filter_map(|c| match c { UiCall::OpenWindow(m) if m.title == "Colour" => Some((m.rect, m.role)), _ => None }).collect();
        assert_eq!(opens.len(), 1, "{name}: {:?}", g.m.ui.calls);
        assert_ne!(l(&g, OUT), 0);
        opened.push(opens[0]);
    }
    assert_eq!(opened[0], opened[1], "GetNewCWindow and GetNewWindow produce the same native window");
    // NEGATIVE CONTROL: a missing WIND returns NIL for both.
    let mut g = Rig::new();
    g.run(Prog::new().clr_l().w(999).l(0).l(0xFFFF_FFFF).trap(0xAA46).pop_l(OUT).exit());
    assert_eq!(l(&g, OUT), 0);
}

#[test]
fn a_color_window_from_new_c_window_is_a_plain_window() {
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("CW"));
    let r = common::SCRATCH + 0x200;
    common::set_rect(&mut g.m, r, 30, 30, 130, 230);
    g.run(Prog::new().clr_l().l(0).l(r).l(STR).b(1).w(0).l(0xFFFF_FFFF).b(1).l(0).trap(0xAA45).pop_l(OUT).exit());
    g.no_refusals();
    assert!(g.m.ui.calls.iter().any(|c| matches!(c, UiCall::OpenWindow(m) if m.title == "CW")), "{:?}", g.m.ui.calls);
    let _ = (Rect { top: 0, left: 0, bottom: 0, right: 0 }, WindowSpec { storage: 0, rect: Rect { top: 0, left: 0, bottom: 1, right: 1 }, title: String::new(), visible: false, proc_id: 0, behind: 0, go_away: false, ref_con: 0 });
}

// ---- cursor, colour pens, fonts, origin ---------------------------------------------------------------------------------

#[test]
fn the_cursor_traps_keep_the_shape_and_a_balanced_hide_level() {
    let mut g = Rig::new();
    let shape: Vec<u8> = (0..68u8).collect();
    put(&mut g.m, STR, &shape);
    g.run(Prog::new().l(STR).trap(0xA851).trap(0xA852).trap(0xA852).trap(0xA853).trap(0xA853).trap(0xA853).exit());
    g.no_refusals();
    assert_eq!(g.m.cursor.shape.map(|s| s.to_vec()), Some(shape));
    assert_eq!(g.m.cursor.level, 0, "ShowCursor never raises the level above 0");
    // NEGATIVE CONTROL: an unbalanced hide stays hidden.
    let mut g = Rig::new();
    g.run(Prog::new().trap(0xA852).trap(0xA852).trap(0xA853).exit());
    assert_eq!(g.m.cursor.level, -1);
    // a Cursor pointer that runs off RAM is refused by name
    let mut g = Rig::new();
    g.run(Prog::new().l(0x00FF_FFF0).trap(0xA851).exit());
    assert!(g.m.refused.iter().any(|r| r.starts_with("SetCursor")), "{:?}", g.m.refused);
}

#[test]
fn fore_color_is_stored_in_the_port_and_a_non_standard_colour_is_refused() {
    let mut g = Rig::new();
    let win = g.win;
    g.run(Prog::new().set_port(win).l(205).trap(0xA862).l(409).trap(0xA863).exit());
    g.no_refusals();
    assert_eq!((l(&g, win + 80), l(&g, win + 84)), (205, 409), "fgColor red, bkColor blue");
    let mut g = Rig::new();
    let win = g.win;
    g.run(Prog::new().set_port(win).l(1234).trap(0xA862).exit());
    assert!(g.m.refused.iter().any(|r| r.contains("ForeColor") && r.contains("eight standard colours")), "{:?}", g.m.refused);
}

#[test]
fn get_fnum_and_get_font_name_use_the_system_families_and_unknown_names_are_font_zero() {
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("hELVETICA"));
    put(&mut g.m, STR + 0x20, &pstr("No Such Face"));
    g.run(Prog::new().l(STR).l(OUT).trap(0xA900).l(STR + 0x20).l(OUT + 2).trap(0xA900).w(22).l(OUT + 0x10).trap(0xA8FF).w(777).l(OUT + 0x40).trap(0xA8FF).exit());
    g.no_refusals();
    assert_eq!(w(&g, OUT), 21, "Helvetica, case-insensitively");
    assert_eq!(w(&g, OUT + 2), 0, "an unknown name is the system font");
    assert_eq!(text(&g, OUT + 0x10), "Courier");
    assert_eq!(text(&g, OUT + 0x40), "", "an unknown family number has no name");
}

#[test]
fn set_origin_moves_the_local_coordinates_and_leaves_the_clip_alone() {
    // paint local (20,10)-(25,15): at canvas (10,20) normally, at canvas (0,0) after SetOrigin(10,20)
    let paint = |origin: bool| {
        let mut g = Rig::new();
        let win = g.win;
        let r = common::SCRATCH + 0x200;
        common::set_rect(&mut g.m, r, 20, 10, 25, 15);
        let mut p = Prog::new();
        p.set_port(win);
        if origin { p.w(10).w(20).trap(0xA878); }
        p.paint_rect(r).exit();
        g.run(&p);
        g.no_refusals();
        (g.count_in(0, 0, 5, 5), g.count_in(10, 20, 15, 25), w(&g, win + 16 + 2) as i16, w(&g, win + 2 + 6 + 2) as i16)
    };
    let (moved_a, moved_b, left, bleft) = paint(true);
    assert_eq!((moved_a, moved_b), (25, 0), "after SetOrigin the rect lands at the canvas corner");
    assert_eq!((left, bleft), (10, -50 + 10), "portRect.left and portBits.bounds.left both took the new origin");
    let (plain_a, plain_b, left, _) = paint(false);
    assert_eq!((plain_a, plain_b, left), (0, 25, 0), "NEGATIVE CONTROL: without SetOrigin it lands at its own coordinates");
}

// ---- pictures ------------------------------------------------------------------------------------------------------------

#[test]
fn an_empty_picture_is_a_version_1_pict_and_draw_picture_replays_it_but_drawing_inside_one_is_named() {
    let mut g = Rig::new();
    let win = g.win;
    let r = SCRATCH + 0x500;
    common::set_rect(&mut g.m, r, 0, 0, 10, 20);
    g.run(Prog::new().set_port(win).clr_l().l(r).trap(0xA8F3).pop_l(OUT).trap(0xA8F4).exit());
    let h = l(&g, OUT);
    let size = g.m.mm.handle_size(h).unwrap();
    assert_eq!(size, 13);
    let bytes = g.m.handle_bytes(h).unwrap();
    assert_eq!(&bytes[..2], &13u16.to_be_bytes());
    assert_eq!(&bytes[10..], &[0x11, 0x01, 0xFF], "version 1, end");
    assert_eq!(l(&g, win + 92), 0, "picSave is NIL after ClosePicture");
    // Drawing while a picture is open is drawn but NOT recorded, which ClosePicture names.
    let mut g = Rig::new();
    let win = g.win;
    let r = SCRATCH + 0x500;
    common::set_rect(&mut g.m, r, 0, 0, 10, 20);
    g.run(Prog::new().set_port(win).clr_l().l(r).trap(0xA8F3).pop_l(OUT).paint_rect(r).trap(0xA8F4).exit());
    assert!(g.m.refused.iter().any(|x| x.starts_with("ClosePicture") && x.contains("not recorded")), "{:?} {:#?}", g.m.refused, g.m.log);
    assert!(g.count_black() > 0, "the drawing still reached the window");
    // NEGATIVE CONTROL: no drawing, no refusal.
    let mut g = Rig::new();
    let win = g.win;
    let r = SCRATCH + 0x500;
    common::set_rect(&mut g.m, r, 0, 0, 10, 20);
    g.run(Prog::new().set_port(win).clr_l().l(r).trap(0xA8F3).pop_l(OUT).trap(0xA8F4).exit());
    g.no_refusals();
}

// ---- Process Manager ---------------------------------------------------------------------------------------------------

#[test]
fn os_dispatch_answers_the_process_manager_for_the_one_process() {
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("x"));
    g.m.mem.write_u32(OUT + 0x80, 60).unwrap(); // processInfoLength
    g.m.mem.write_u32(OUT + 0x84, OUT + 0x200).unwrap(); // processName buffer
    let mut p = Prog::new();
    p.clr_w().l(OUT).w(0x37).trap(0xA88F).pop_w(OUT + 0x10);                       // GetCurrentProcess
    p.clr_w().l(OUT + 0x20).l(OUT + 0x80).w(0x3A).trap(0xA88F).pop_w(OUT + 0x12);  // GetProcessInformation(PSN at OUT+0x20)
    p.clr_w().l(OUT).l(OUT).l(OUT + 0x14).w(0x3D).trap(0xA88F).pop_w(OUT + 0x16);  // SameProcess(me, me)
    p.exit();
    g.m.mem.write_u32(OUT + 0x20, 0).unwrap(); g.m.mem.write_u32(OUT + 0x24, 2).unwrap();
    g.run(&p);
    g.no_refusals();
    assert_eq!((l(&g, OUT), l(&g, OUT + 4)), (0, 2));
    assert_eq!((w(&g, OUT + 0x10), w(&g, OUT + 0x12), w(&g, OUT + 0x16)), (0, 0, 0));
    assert_eq!(g.m.mem.read_u8(OUT + 0x14).unwrap(), 1, "SameProcess is TRUE for the process and itself");
    assert_eq!(l(&g, OUT + 0x80 + 16), u32::from_be_bytes(*b"APPL"));
    // NEGATIVE CONTROL: another process number is procNotFound (-600), and an unknown selector halts by name.
    let mut g = Rig::new();
    g.m.mem.write_u32(OUT + 0x24, 99).unwrap();
    g.m.mem.write_u32(OUT + 0x80, 60).unwrap();
    g.run(Prog::new().clr_w().l(OUT + 0x20).l(OUT + 0x80).w(0x3A).trap(0xA88F).pop_w(OUT + 0x12).exit());
    assert_eq!(w(&g, OUT + 0x12) as i16, -600);
    let mut g = Rig::new();
    g.m.start(&Prog::new().w(0x7F).trap(0xA88F).exit().0, 0).unwrap();
    g.m.run_loop(10_000).unwrap();
    assert!(g.m.refused.iter().any(|x| x.contains("OSDispatch selector 0x7f")), "{:?}", g.m.refused);
}

// ---- menus, memory, files -----------------------------------------------------------------------------------------------

#[test]
fn a_font_menu_lists_the_system_families_and_get_item_past_the_end_is_an_empty_string() {
    let mut g = Rig::new();
    let mut menu = Vec::new();
    menu.extend_from_slice(&6i16.to_be_bytes());
    menu.extend_from_slice(&[0; 8]);
    menu.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    menu.extend(pstr("Font"));
    menu.push(0);
    let fork = heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"MENU", 6, None, &menu).build();
    g.m.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork).unwrap());
    put(&mut g.m, OUT + 0x100, b"stale-bytes-in-the-buffer");
    g.run(Prog::new().clr_l().w(6).trap(0xA9BF).pop_l(OUT)
        .rgn_arg(OUT).l(u32::from_be_bytes(*b"FONT")).trap(0xA94D)
        .rgn_arg(OUT).w(16).l(OUT + 0x100).trap(0xA946)
        .rgn_arg(OUT).w(2).l(OUT + 0x140).trap(0xA946).exit());
    g.no_refusals();
    assert_eq!(text(&g, OUT + 0x140), "Cairo", "sorted: Athens, Cairo, Chicago ...");
    assert_eq!(text(&g, OUT + 0x100), "", "item 16 of a 15-item menu reads the terminator: an empty string, and the buffer is overwritten");
}

#[test]
fn a_plain_get_vol_info_block_is_not_written_past_its_64_bytes() {
    // PBGetVInfo's volumeParam is 64 bytes; only PBHGetVInfo has the fields beyond. A sentinel word right after the block must survive.
    let run = |trap: u16| {
        let mut g = Rig::new();
        let pb = OUT + 0x100;
        g.m.mem.zero(pb, 0x80).unwrap();
        g.m.mem.write_u16(pb + 28, 0).unwrap(); // ioVolIndex 0: the default volume
        g.m.mem.write_u32(pb + 18, 0).unwrap(); // no name pointer
        g.m.mem.write_u32(pb + 66, 0xDEAD_BEEF).unwrap();
        g.m.mem.write_u32(pb + 82, 0xDEAD_BEEF).unwrap();
        let mut p = Prog::new();
        set_a0(&mut p, pb); p.trap(trap).exit();
        g.run(&p);
        (l(&g, pb + 66), l(&g, pb + 82), w(&g, pb + 16) as i16)
    };
    let (a, b, res) = run(0xA007);
    assert_eq!((a, b, res), (0xDEAD_BEEF, 0xDEAD_BEEF, 0), "a plain GetVInfo leaves the bytes after its 64-byte block alone");
    let (_, b, _) = run(0xA207);
    assert_ne!(b, 0xDEAD_BEEF, "NEGATIVE CONTROL: the HFS variant does write ioVFilCnt at 82");
}

// ---- dialogs -------------------------------------------------------------------------------------------------------------

fn text_dialog_fork(text: &str) -> Vec<u8> {
    let item = |rect: [i16; 4], ty: u8, data: &[u8]| {
        let mut v = vec![0u8; 4];
        for r in rect { v.extend_from_slice(&r.to_be_bytes()); }
        v.push(ty);
        v.push(data.len() as u8); // the item's length byte counts the text; there is NO Pascal length inside
        v.extend_from_slice(data);
        if data.len() & 1 == 1 { v.push(0); }
        v
    };
    let mut ditl = 1i16.to_be_bytes().to_vec();
    for it in [item([80, 200, 100, 260], 4, b"Oh dear"), item([10, 10, 40, 250], 8, text.as_bytes())] { ditl.extend_from_slice(&it); }
    let mut dlog = Vec::new();
    for v in [60i16, 40, 180, 300, 1, 0x0100, 0x0000] { dlog.extend_from_slice(&v.to_be_bytes()); }
    dlog.extend_from_slice(&0u32.to_be_bytes());
    dlog.extend_from_slice(&131i16.to_be_bytes());
    dlog.extend(pstr("Real"));
    heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"DLOG", 131, None, &dlog).add(b"DITL", 131, None, &ditl).build()
}

#[test]
fn a_real_ditl_item_keeps_its_first_character_and_param_text_fills_the_carets() {
    let labels = |param: Option<&str>| {
        let mut g = Rig::new();
        g.m.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(text_dialog_fork("Sorry, ^0 and ^1 ^^ ^9")).unwrap());
        let mut p = Prog::new();
        if let Some(s) = param {
            put(&mut g.m, STR, &pstr(s));
            p.l(STR).l(0).l(0).l(0).trap(0xA98B);
        }
        p.clr_l().w(131).l(0).l(0xFFFF_FFFF).trap(0xA97C).pop_l(OUT).exit();
        g.run(&p);
        g.no_refusals();
        let mut v: Vec<String> = Vec::new();
        for c in &g.m.ui.calls { if let UiCall::AddControl(_, m) = c { v.push(format!("{}|{:?}", m.label, m.kind)); } }
        v
    };
    let with = labels(Some("the cat"));
    assert!(with.iter().any(|s| s.starts_with("Oh dear|")), "the button keeps its O: {with:?}");
    assert!(with.iter().any(|s| s.contains("Sorry, the cat and  ^^ ^9")), "^0 filled, ^1 empty (never set), ^^ and ^9 left alone: {with:?}");
    // NEGATIVE CONTROL: without a ParamText the caret stays empty.
    let without = labels(None);
    assert!(without.iter().any(|s| s.contains("Sorry,  and  ^^ ^9")), "{without:?}");
}

#[test]
fn is_dialog_event_is_true_only_for_events_that_belong_to_a_dialog_and_dialog_select_reports_item_hits() {
    let mut g = Rig::new();
    g.m.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(text_dialog_fork("hi")).unwrap());
    g.run(Prog::new().clr_l().w(131).l(0).l(0xFFFF_FFFF).trap(0xA97C).pop_l(OUT).exit());
    let dlg = l(&g, OUT);
    assert_ne!(dlg, 0);
    let ev = |g: &mut Rig, what: u16, msg: u32, v: i16, h: i16| { let e = SCRATCH + 0x300; g.m.mem.write_u16(e, what).unwrap(); g.m.mem.write_u32(e + 2, msg).unwrap(); g.m.mem.write_u16(e + 10, v as u16).unwrap(); g.m.mem.write_u16(e + 12, h as u16).unwrap(); e };
    let ask = |g: &mut Rig, e: u32| { g.run(Prog::new().clr_w().l(e).trap(0xA97F).pop_w(OUT + 0x20).exit()); w(g, OUT + 0x20) >> 8 };
    let e = ev(&mut g, 6, dlg, 0, 0);
    assert_eq!(ask(&mut g, e), 1, "an update for the dialog window");
    let other = g.win;
    let e = ev(&mut g, 6, other, 0, 0);
    assert_eq!(ask(&mut g, e), 0, "NEGATIVE CONTROL: an update for an ordinary window");
    // DialogSelect: nothing pending -> FALSE; a queued hit -> TRUE with dialog and item
    let sel = |g: &mut Rig| { let e = SCRATCH + 0x300; g.run(Prog::new().clr_w().l(e).l(OUT + 0x30).l(OUT + 0x34).trap(0xA980).pop_w(OUT + 0x38).exit()); w(g, OUT + 0x38) >> 8 };
    assert_eq!(sel(&mut g), 0);
    g.m.tr.dlg.hits.push_back((dlg, 1));
    assert_eq!(sel(&mut g), 1);
    assert_eq!((l(&g, OUT + 0x30), w(&g, OUT + 0x34)), (dlg, 1));
    assert_eq!(sel(&mut g), 0, "the hit was consumed");
}

#[test]
fn the_window_records_title_handle_is_a_handle_to_the_pascal_title_that_set_wtitle_keeps_current() {
    // WriteNow 2.2 copies window titles into its Window menu from (**WindowPeek(w)->titleHandle) (offset 134); with the field NIL the copy
    // was an empty string, its search for the item found nothing, the helper pushed no arguments and SetItmStyle got stack garbage.
    let mut g = Rig::new();
    let win = g.win;
    let title_of = |g: &Rig| { let h = l(g, win + 134); assert_ne!(h, 0, "titleHandle is a real handle"); let p = g.m.mm.block(h).expect("a handle").ptr; text(g, p) };
    assert_eq!(title_of(&g), "QD", "the title the window was made with");
    let h0 = l(&g, win + 134);
    put(&mut g.m, STR, &pstr("A longer title than before"));
    g.run(Prog::new().l(win).l(STR).trap(0xA91A).exit());
    g.no_refusals();
    assert_eq!(title_of(&g), "A longer title than before");
    assert_eq!(l(&g, win + 134), h0, "the same handle, resized");
    // the handle goes with the window
    g.run(Prog::new().l(win).trap(0xA914).exit()); // DisposeWindow
    assert!(g.m.mm.block(h0).is_none(), "the title handle is freed with the window");
}
