//! TextEdit end to end: hand-assembled programs push the Pascal frames of `TENew` ... `TEClick` and the engine must keep a REAL
//! `TERec` (Inside Macintosh I "TextEdit"; field offsets as systemless and Executor write them), break lines the ROM's way,
//! draw the text, the highlight and the caret into the window, and edit through `TEKey`/`TEInsert`/`TEDelete`/the TE scrap.
mod common;
use common::*;
use heirloom_mac68k_engine::textedit::*;

const DEST: u32 = 0x3100;
const VIEW: u32 = 0x3108;
const TEXT: u32 = 0x3200;
const TEH: u32 = 0x3300; // where the program keeps the TEHandle
const UPD: u32 = 0x3110;

fn teh(g: &Rig) -> u32 { g.m.mem.read_u32(TEH).unwrap() }
fn rec(g: &Rig, off: u32) -> u16 { let p = g.m.mm.block(teh(g)).unwrap().ptr; g.m.mem.read_u16(p + off).unwrap() }
fn starts(g: &Rig) -> Vec<u16> { let n = rec(g, T_N_LINES); (0..=n).map(|i| rec(g, T_LINE_STARTS + 2 * u32::from(i))).collect() }
fn text(g: &Rig) -> Vec<u8> {
    let p = g.m.mm.block(teh(g)).unwrap().ptr;
    let ht = g.m.mem.read_u32(p + T_HTEXT).unwrap();
    let b = g.m.mm.block(ht).unwrap();
    g.m.mem.bytes(g.m.mem.span(b.ptr, u32::from(rec(g, T_LENGTH))).unwrap()).unwrap().to_vec()
}
fn sel(g: &Rig) -> (u16, u16) { (rec(g, T_SEL_START), rec(g, T_SEL_END)) }

/// `TENew(dest, view)` into `TEH`, with `dest` (top, left, bottom, right) and the view the same unless `view` is given.
fn te_new(p: &mut Prog) { p.clr_l().l(DEST).l(VIEW).trap(0xA9D2).pop_l(TEH); }
fn te_set_text(p: &mut Prog, s: &[u8]) { p.l(TEXT).l(s.len() as u32).rgn_arg(TEH).trap(0xA9CF); }
fn te_cal(p: &mut Prog) { p.rgn_arg(TEH).trap(0xA9D0); }
fn te_update(p: &mut Prog) { p.l(UPD).rgn_arg(TEH).trap(0xA9D3); }
fn te_key(p: &mut Prog, k: u8) { p.ch(k).rgn_arg(TEH).trap(0xA9DC); }
fn te_sel(p: &mut Prog, a: u32, b: u32) { p.l(a).l(b).rgn_arg(TEH).trap(0xA9D1); }
fn te_click(p: &mut Prog, v: i16, h: i16, extend: bool) { p.l((u32::from(v as u16) << 16) | u32::from(h as u16)).b(extend as u8).rgn_arg(TEH).trap(0xA9D4); }
fn one(p: &mut Prog, trap: u16) { p.rgn_arg(TEH).trap(trap); }

/// A rig with the record's rectangles set: an 80-wide, 40-high box at local (10, 10) (10 characters of 8 pixels per line).
fn rig() -> Rig {
    let mut g = Rig::new();
    assert_eq!(g.m.qd.font.text_length(b"a"), 8, "the stand-in font advances 8 pixels; the expectations below count in characters");
    set_rect(&mut g.m, DEST, 10, 10, 50, 90);
    set_rect(&mut g.m, VIEW, 10, 10, 50, 90);
    set_rect(&mut g.m, UPD, 0, 0, 120, 200);
    g
}
fn go(g: &mut Rig, f: impl FnOnce(&mut Prog)) { let mut p = Prog::new(); f(&mut p); g.run(p.exit()); }

#[test]
fn tenew_builds_a_real_record_with_the_ports_text_state() {
    let mut g = rig();
    go(&mut g, |p| { p.trap(0xA9CC); te_new(p); });
    g.no_refusals();
    let (h, p) = (teh(&g), g.m.mm.block(teh(&g)).unwrap().ptr);
    let r = |off| g.m.mem.read_i16(p + off).unwrap();
    assert_eq!([r(T_DEST), r(T_DEST + 2), r(T_DEST + 4), r(T_DEST + 6)], [10, 10, 50, 90], "destRect is the program's rectangle");
    assert_eq!([r(T_VIEW), r(T_VIEW + 2), r(T_VIEW + 4), r(T_VIEW + 6)], [10, 10, 50, 90]);
    assert_eq!((r(T_LINE_HEIGHT), r(T_ASCENT)), (g.m.qd.font.y_size as i16, g.m.qd.font.baseline as i16 + 1), "lineHeight / fontAscent come from the font that is drawn");
    assert_eq!((r(T_CR_ONLY), r(T_N_LINES), r(T_LENGTH), r(T_ACTIVE)), (1, 0, 0, 0), "wrap on, no lines, no text, inactive");
    assert_eq!(g.m.mem.read_u32(p + T_IN_PORT).unwrap(), g.win, "inPort is the port that was current");
    assert!(g.m.mm.block(g.m.mem.read_u32(p + T_HTEXT).unwrap()).is_some(), "hText is a live (empty) handle");
    assert_ne!(h, 0);
    assert_ne!(g.m.mem.read_u32(heirloom_mac68k_engine::lowmem::TE_SCRP_HANDLE).unwrap(), 0, "TEInit made the TE scrap");
}

#[test]
fn settext_then_caltext_breaks_lines_the_roms_way_and_a_cr_ends_a_line() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"hello world, this is text");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"hello world, this is text"); });
    assert_eq!((rec(&g, T_LENGTH), rec(&g, T_N_LINES)), (25, 0), "TESetText stores the text; it does not recalculate (the program calls TECalText)");
    go(&mut g, |p| { te_cal(p); });
    // 10 characters fit: break after the last space before the overflow
    assert_eq!(starts(&g), [0, 6, 13, 21, 25], "lines: 'hello ' 'world, ' 'this is ' 'text'");
    assert_eq!(rec(&g, T_N_LINES), 4);
    // a CR ends a line whatever the width
    put(&mut g.m, TEXT, b"ab\rcd");
    go(&mut g, |p| { te_set_text(p, b"ab\rcd"); te_cal(p); });
    assert_eq!(starts(&g), [0, 3, 5]);
    // crOnly < 0: no wrapping at all
    let h = teh(&g);
    let pr = g.m.mm.block(h).unwrap().ptr;
    g.m.mem.write_u16(pr + T_CR_ONLY, 0xFFFF).unwrap();
    put(&mut g.m, TEXT, b"hello world, this is text");
    go(&mut g, |p| { te_set_text(p, b"hello world, this is text"); te_cal(p); });
    assert_eq!(starts(&g), [0, 25], "crOnly -1: one line, however wide");
}

#[test]
fn teupdate_draws_the_text_clipped_to_the_view_and_erases_what_was_there() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"hello world, this is text");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"hello world, this is text"); te_cal(p); });
    assert_eq!(g.count_black(), 0, "nothing is drawn before TEUpdate");
    go(&mut g, |p| { te_update(p); });
    g.no_refusals();
    assert!(g.count_in(10, 10, 90, 50) > 40, "the text is in the view\n{}", g.pic(8, 8, 94, 52));
    assert_eq!(g.count_in(0, 0, 200, 10) + g.count_in(0, 50, 200, 120) + g.count_in(0, 10, 10, 50) + g.count_in(90, 10, 200, 50), 0, "nothing outside the view rectangle");
    // the second line starts one lineHeight lower than the first
    let lh = rec(&g, T_LINE_HEIGHT) as i32;
    assert!(g.count_in(10, 10 + lh, 90, 10 + 2 * lh) > 0, "line 2 is drawn a line height below line 1");
    // black pixels in the view are erased by the next update when the text is gone
    put(&mut g.m, TEXT, b"x");
    go(&mut g, |p| { te_set_text(p, b"x"); te_cal(p); te_update(p); });
    assert_eq!(g.count_in(10, 10 + lh, 90, 50), 0, "the old lines 2.. are erased: TEUpdate erases its target before it draws");
    assert!(g.count_in(10, 10, 90, 10 + lh) > 0);
}

#[test]
fn text_wider_than_the_view_is_clipped_to_the_view_not_to_the_destination() {
    let mut g = rig();
    set_rect(&mut g.m, VIEW, 10, 30, 50, 70); // the view is the middle half of the 80-wide destination
    put(&mut g.m, TEXT, b"WWWWWWWWWW");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"WWWWWWWWWW"); te_cal(p); te_update(p); });
    assert!(g.count_in(30, 10, 70, 50) > 40, "the middle of the line is drawn\n{}", g.pic(8, 8, 94, 30));
    assert_eq!(g.count_in(0, 0, 30, 120) + g.count_in(70, 0, 200, 120), 0, "the ends that fall outside the view are clipped");
}

#[test]
fn a_carriage_return_is_never_drawn_and_has_no_width() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"\r");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"\r"); te_cal(p); te_update(p); });
    assert_eq!(g.count_black(), 0, "a CR draws nothing\n{}", g.pic(8, 8, 30, 30));
}

#[test]
fn tekey_inserts_backspaces_and_moves_the_insertion_point() {
    let mut g = rig();
    go(&mut g, |p| { te_new(p); one(p, 0xA9D8); for k in *b"abc" { te_key(p, k); } });
    g.no_refusals();
    assert_eq!((text(&g), sel(&g), rec(&g, T_LENGTH)), (b"abc".to_vec(), (3, 3), 3));
    assert_eq!(starts(&g), [0, 3]);
    go(&mut g, |p| { te_key(p, 0x1C); te_key(p, 0x1C); te_key(p, b'X'); });
    assert_eq!((text(&g), sel(&g)), (b"aXbc".to_vec(), (2, 2)), "left, left, then type: inserted at the insertion point");
    go(&mut g, |p| { te_key(p, 0x08); });
    assert_eq!((text(&g), sel(&g)), (b"abc".to_vec(), (1, 1)), "backspace removes the character before the insertion point");
    go(&mut g, |p| { te_key(p, 0x7F); te_key(p, 0x1D); te_key(p, 0x1D); te_key(p, 0x1D); });
    assert_eq!((text(&g), sel(&g)), (b"ac".to_vec(), (2, 2)), "forward delete, then right x3 stops at the end");
    go(&mut g, |p| { te_sel(p, 0, 2); te_key(p, b'Z'); });
    assert_eq!((text(&g), sel(&g)), (b"Z".to_vec(), (1, 1)), "typing over a selection replaces it");
    go(&mut g, |p| { te_key(p, 0x03); });
    assert_eq!(text(&g), b"Z\r".to_vec(), "the keypad Enter is a carriage return");
    assert_eq!(starts(&g), [0, 2], "and the line table follows");
    go(&mut g, |p| { te_key(p, 0x08); te_key(p, 0x08); te_key(p, 0x08); });
    assert_eq!((text(&g), sel(&g)), (vec![], (0, 0)), "backspace at the start of the text deletes nothing");
    assert_eq!(rec(&g, T_N_LINES), 0);
}

#[test]
fn typing_wraps_at_the_destination_width() {
    let mut g = rig();
    go(&mut g, |p| { te_new(p); for k in *b"abcdefghijk" { te_key(p, k); } });
    assert_eq!(starts(&g), [0, 10, 11], "the 11th character starts the second line (10 fit)");
}

#[test]
fn teclick_puts_the_insertion_point_at_the_nearest_character_and_double_click_selects_a_word() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"foo bar baz");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"foo bar baz"); te_cal(p); });
    // 10 chars per line: "foo bar " / "baz"; the destination's top-left is (10, 10)
    go(&mut g, |p| { te_click(p, 12, 10 + 8 * 2 + 3, false); });
    assert_eq!(sel(&g), (2, 2), "3 pixels into the 3rd character is before it");
    go(&mut g, |p| { te_click(p, 12, 10 + 8 * 2 + 5, false); });
    assert_eq!(sel(&g), (3, 3), "5 pixels is past its middle: after it");
    go(&mut g, |p| { te_click(p, 12 + 12, 10 + 3, false); });
    assert_eq!(sel(&g), (8, 8), "a click on the second line lands there");
    // a double click: the same place again within DoubleTime ticks
    g.m.ev.ticks = 1000;
    go(&mut g, |p| { te_click(p, 12, 10 + 8 * 5 + 1, false); });
    assert_eq!(sel(&g), (5, 5));
    g.m.ev.ticks = 1010;
    go(&mut g, |p| { te_click(p, 12, 10 + 8 * 5 + 1, false); });
    assert_eq!(sel(&g), (4, 7), "double click selects 'bar'");
    // negative control: the same click after the interval is a plain click
    g.m.ev.ticks = 5000;
    go(&mut g, |p| { te_click(p, 12, 10 + 8 * 5 + 1, false); });
    assert_eq!(sel(&g), (5, 5), "too late for a double click");
    // shift extends
    go(&mut g, |p| { te_click(p, 12, 10 + 8 * 7 + 1, true); });
    assert_eq!(sel(&g), (5, 7), "shift-click extends to the click");
    g.no_refusals();
}

#[test]
fn an_active_selection_is_inverted_and_the_caret_blinks_with_teidle() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"hello");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"hello"); te_cal(p); te_update(p); });
    let plain = g.count_in(10, 10, 90, 50);
    go(&mut g, |p| { te_sel(p, 0, 5); one(p, 0xA9D8); });
    let lh = rec(&g, T_LINE_HEIGHT) as i32;
    assert_eq!(rec(&g, T_ACTIVE), 0xFF00, "TEActivate sets active");
    assert_eq!(g.count_in(10, 10, 90, 10 + lh), 5 * 8 * lh as usize - plain, "the 40x{lh} highlight is the text inverted");
    go(&mut g, |p| { one(p, 0xA9D9); });
    assert_eq!((rec(&g, T_ACTIVE), g.count_in(10, 10, 90, 50)), (0, plain), "TEDeactivate takes the highlight away");
    // the caret: inactive records show none; an active one blinks on TEIdle after CaretTime ticks
    go(&mut g, |p| { te_sel(p, 2, 2); one(p, 0xA9D8); });
    g.m.ev.ticks = 100;
    let before = g.count_in(10, 10, 90, 50);
    go(&mut g, |p| { one(p, 0xA9DA); });
    let on = g.count_in(10, 10, 90, 50);
    assert_ne!(on, before, "the caret appears after CaretTime ticks");
    go(&mut g, |p| { one(p, 0xA9DA); });
    assert_eq!(g.count_in(10, 10, 90, 50), on, "too soon: TEIdle does nothing");
    g.m.ev.ticks = 200;
    go(&mut g, |p| { one(p, 0xA9DA); });
    assert_eq!(g.count_in(10, 10, 90, 50), before, "and it goes away on the next blink");
    g.no_refusals();
}

#[test]
fn copy_cut_and_paste_go_through_the_te_scrap() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"hello");
    go(&mut g, |p| { p.trap(0xA9CC); te_new(p); te_set_text(p, b"hello"); te_cal(p); te_sel(p, 1, 4); one(p, 0xA9D5); });
    assert_eq!(g.m.mem.read_u16(heirloom_mac68k_engine::lowmem::TE_SCRP_LENGTH).unwrap(), 3, "TEScrpLength");
    assert_eq!(text(&g), b"hello".to_vec(), "TECopy leaves the text alone");
    go(&mut g, |p| { te_sel(p, 5, 5); one(p, 0xA9DB); });
    assert_eq!((text(&g), sel(&g)), (b"helloell".to_vec(), (8, 8)), "TEPaste inserts the scrap at the insertion point");
    go(&mut g, |p| { te_sel(p, 0, 5); one(p, 0xA9D6); });
    assert_eq!((text(&g), sel(&g), g.m.mem.read_u16(heirloom_mac68k_engine::lowmem::TE_SCRP_LENGTH).unwrap()), (b"ell".to_vec(), (0, 0), 5), "TECut removes the selection and keeps it");
    go(&mut g, |p| { te_sel(p, 0, 3); one(p, 0xA9D7); });
    assert_eq!(text(&g), Vec::<u8>::new(), "TEDelete");
    g.no_refusals();
}

#[test]
fn teinsert_and_alignment_and_scroll_move_things() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"ab");
    go(&mut g, |p| { te_new(p); p.l(TEXT).l(2).rgn_arg(TEH).trap(0xA9DE); });
    assert_eq!((text(&g), sel(&g)), (b"ab".to_vec(), (2, 2)), "TEInsert at the insertion point");
    go(&mut g, |p| { te_update(p); });
    let left = g.count_in(10, 10, 40, 50);
    assert!(left > 0 && g.count_in(60, 10, 90, 50) == 0, "flush left by default");
    go(&mut g, |p| { p.w(-1).rgn_arg(TEH).trap(0xA9DF).l(UPD).rgn_arg(TEH).trap(0xA9D3); });
    assert_eq!(rec(&g, T_JUST) as i16, -1);
    assert!(g.count_in(10, 10, 60, 50) == 0 && g.count_in(60, 10, 90, 50) > 0, "teFlushRight moves the text to the right edge\n{}", g.pic(8, 8, 94, 30));
    go(&mut g, |p| { p.w(0).rgn_arg(TEH).trap(0xA9DF); p.w(3).w(4).rgn_arg(TEH).trap(0xA9DD); });
    let p = g.m.mm.block(teh(&g)).unwrap().ptr;
    assert_eq!((g.m.mem.read_i16(p + T_DEST).unwrap(), g.m.mem.read_i16(p + T_DEST + 2).unwrap()), (14, 13), "TEScroll(dh 3, dv 4) moves destRect");
    g.no_refusals();
}

#[test]
fn tetextbox_draws_wrapped_text_into_the_box_and_erases_it_first() {
    let mut g = rig();
    put(&mut g.m, TEXT, b"hello world, this is text");
    set_rect(&mut g.m, UPD, 20, 20, 60, 100);
    // old ink in the box is erased
    go(&mut g, |p| { p.l(UPD).trap(0xA8A2); p.l(TEXT).l(25).l(UPD).w(0).trap(0xA9CE); });
    g.no_refusals();
    assert!(g.count_in(20, 20, 100, 60) > 40 && g.count_in(20, 20, 100, 60) < 20 * 80 / 2, "text, not a black box\n{}", g.pic(18, 18, 104, 64));
    assert_eq!(g.count_in(0, 0, 200, 20) + g.count_in(0, 60, 200, 120), 0);
}

#[test]
fn a_wild_handle_is_refused_by_name_in_every_routine_that_takes_one() {
    let mut g = rig();
    g.m.start(&{ let mut p = Prog::new(); p.l(UPD).l(0x1234_5678).trap(0xA9D3).exit(); p.0 }, 0).unwrap();
    let _ = g.m.run_loop(1_000_000);
    assert!(g.m.refused.iter().any(|r| r.contains("TEUpdate") && r.contains("not a live handle")), "{:?}", g.m.refused);
    // the same for the view routines and the dispatcher's
    let mut g = rig();
    g.m.start(&{ let mut p = Prog::new(); p.l(0x1234_5678).trap(0xA811).l(0x1234_5678); p.w(4).trap(0xA83D).exit(); p.0 }, 0).unwrap();
    let _ = g.m.run_loop(1_000_000);
    assert!(g.m.refused.iter().any(|r| r.contains("TESelView") && r.contains("not a live handle")), "{:?}", g.m.refused);
    assert!(g.m.refused.iter().any(|r| r.contains("TEGetStyleHandle") && r.contains("not a live handle")), "{:?}", g.m.refused);
}

// ---- styled TextEdit ----

const STYLE: u32 = 0x3500; // a TextStyle (12 bytes)
const NEW_STYLE: u32 = 0x3520;
const RES: u32 = 0x3540; // results land here

fn te_style_new(p: &mut Prog) { p.clr_l().l(DEST).l(VIEW).trap(0xA83E).pop_l(TEH); }
/// `_TEDispatch`: arguments are pushed by the caller, the selector word LAST.
fn disp(p: &mut Prog, sel: i16) { p.w(sel).trap(0xA83D); }
fn set_style(p: &mut Prog, mode: i16, at: u32, redraw: bool) { p.w(mode).l(at).b(redraw as u8).rgn_arg(TEH); disp(p, 1); }
fn put_style(g: &mut Rig, at: u32, font: i16, face: u8, size: i16) {
    let a = heirloom_mac68k_engine::textedit::styles::Attrs { font, face, size, color: [0; 3] };
    put(&mut g.m, at, &a.write());
}
fn style_rec(g: &Rig) -> (u32, u32) { // (the TEStyleRec's address, its handle)
    let p = g.m.mm.block(teh(g)).unwrap().ptr;
    let sh = g.m.mem.read_u32(p + T_TX_FONT).unwrap();
    (g.m.mm.block(sh).unwrap().ptr, sh)
}
fn runs(g: &Rig) -> Vec<(u16, i16)> {
    let (sp, _) = style_rec(g);
    let n = g.m.mem.read_u16(sp).unwrap();
    (0..=n as u32).map(|i| (g.m.mem.read_u16(sp + 20 + 4 * i).unwrap(), g.m.mem.read_i16(sp + 22 + 4 * i).unwrap())).collect()
}
fn style_face(g: &Rig, idx: usize) -> u8 {
    let (sp, _) = style_rec(g);
    let st = g.m.mm.block(g.m.mem.read_u32(sp + 4).unwrap()).unwrap().ptr;
    g.m.mem.read_u8(st + 18 * idx as u32 + 8).unwrap()
}

#[test]
fn testylenew_builds_the_style_structures_the_rom_builds() {
    let mut g = rig();
    go(&mut g, |p| { te_style_new(p); });
    g.no_refusals();
    assert_eq!((rec(&g, T_LINE_HEIGHT) as i16, rec(&g, T_ASCENT) as i16, rec(&g, 80) as i16), (-1, -1, -1), "lineHeight, fontAscent and txSize are -1 in a styled record");
    let (sp, sh) = style_rec(&g);
    assert_ne!(sh, 0);
    assert_eq!((g.m.mem.read_u16(sp).unwrap(), g.m.mem.read_u16(sp + 2).unwrap()), (1, 1), "nRuns 1, nStyles 1");
    assert_eq!(runs(&g), [(0, 0), (1, -1)], "one run of style 0 and the sentinel {{ teLength + 1, -1 }}");
    let st = g.m.mm.block(g.m.mem.read_u32(sp + 4).unwrap()).unwrap();
    assert_eq!(st.size, 18, "one 18-byte STElement");
    assert_eq!(g.m.mem.read_u16(st.ptr).unwrap(), 1, "stCount");
    assert_eq!(g.m.mm.block(g.m.mem.read_u32(sp + 8).unwrap()).unwrap().size, 4, "lhTab: one {{ height, ascent }}");
    let ns = g.m.mem.read_u32(sp + 16).unwrap();
    let scrap = g.m.mem.read_u32(g.m.mm.block(ns).unwrap().ptr + 4).unwrap();
    assert_eq!(g.m.mem.read_u16(g.m.mm.block(scrap).unwrap().ptr).unwrap(), 0, "the null style scrap holds no style yet");
    // negative control: a plain record has none of this
    let mut g2 = rig();
    go(&mut g2, |p| { te_new(p); });
    assert_eq!(rec(&g2, 80), g2.m.mem.read_u16(g2.win + 74).unwrap(), "a plain record's txSize is the port's");
}

#[test]
fn typing_in_a_styled_record_extends_the_run_and_the_sentinel() {
    let mut g = rig();
    go(&mut g, |p| { te_style_new(p); one(p, 0xA9D8); for k in *b"abc" { te_key(p, k); } });
    g.no_refusals();
    assert_eq!(text(&g), b"abc".to_vec());
    assert_eq!(runs(&g), [(0, 0), (4, -1)], "still one run; the sentinel follows teLength + 1");
    go(&mut g, |p| { te_key(p, 0x08); });
    assert_eq!(runs(&g), [(0, 0), (3, -1)]);
    // TESetText starts the runs over
    put(&mut g.m, TEXT, b"hello");
    go(&mut g, |p| { te_set_text(p, b"hello"); });
    assert_eq!(runs(&g), [(0, 0), (6, -1)]);
}

#[test]
fn tesetstyle_restyles_the_selection_splits_runs_and_tegetstyle_reads_them_back() {
    let mut g = rig();
    put_style(&mut g, STYLE, 0, 1, 0); // bold
    put(&mut g.m, TEXT, b"abcdef");
    go(&mut g, |p| { te_style_new(p); te_set_text(p, b"abcdef"); te_cal(p); te_sel(p, 1, 3); set_style(p, 2, STYLE, true); });
    g.no_refusals();
    assert_eq!(runs(&g), [(0, 0), (1, 1), (3, 0), (7, -1)], "doFace on 1..3 splits the run in three; the sentinel stays");
    assert_eq!((style_face(&g, 0), style_face(&g, 1)), (0, 1));
    // TEGetStyle(offset, VAR style, VAR lineHeight, VAR ascent, hTE): offsets 2 (bold) and 4 (plain)
    for (off, face) in [(2i16, 1u8), (4, 0)] {
        go(&mut g, |p| { p.w(off).l(NEW_STYLE).l(RES).l(RES + 2).rgn_arg(TEH); disp(p, 3); });
        assert_eq!(g.m.mem.read_u8(NEW_STYLE + 2).unwrap(), face, "face at {off}");
        assert_eq!(g.m.mem.read_u16(RES).unwrap(), g.m.qd.font.y_size as u16, "lineHeight");
    }
    // the bold run is drawn bold: more ink in its two cells than the same two cells plain
    let bold = g.count_in(10 + 8, 10, 10 + 24, 10 + g.m.qd.font.y_size as i32);
    let plain = g.count_in(10 + 24, 10, 10 + 40, 10 + g.m.qd.font.y_size as i32);
    assert!(bold > plain, "bold {bold} vs plain {plain}\n{}", g.pic(8, 8, 60, 22));
    // restyle everything plain again: the bold style is no longer used and goes
    put_style(&mut g, STYLE, 0, 0, 0);
    go(&mut g, |p| { te_sel(p, 0, 6); set_style(p, 2, STYLE, true); });
    assert_eq!(runs(&g), [(0, 0), (7, -1)]);
    let (sp, _) = style_rec(&g);
    assert_eq!(g.m.mem.read_u16(sp + 2).unwrap(), 1, "nStyles back to 1");
}

#[test]
fn a_style_set_at_the_insertion_point_is_the_style_of_the_next_typed_text_until_a_click() {
    let mut g = rig();
    put_style(&mut g, STYLE, 0, 2, 0); // italic
    go(&mut g, |p| { te_style_new(p); one(p, 0xA9D8); te_key(p, b'a'); set_style(p, 2, STYLE, true); te_key(p, b'b'); te_key(p, b'c'); });
    g.no_refusals();
    assert_eq!(runs(&g), [(0, 0), (1, 1), (4, -1)], "'a' plain, 'bc' italic: the null style applied to the typed text");
    // a click ends a pending null style: set bold, click, type
    put_style(&mut g, STYLE, 0, 1, 0);
    go(&mut g, |p| { set_style(p, 2, STYLE, true); te_click(p, 12, 10 + 8, false); te_key(p, b'x'); });
    assert_eq!(runs(&g), [(0, 0), (2, 1), (5, -1)], "x (typed at offset 1) took the style of the character before it, plain 'a', not the abandoned bold: no new run");
}

#[test]
fn tecontinuousstyle_tells_which_attributes_are_the_same_through_the_selection() {
    let mut g = rig();
    put_style(&mut g, STYLE, 0, 1, 0);
    put(&mut g.m, TEXT, b"abcd");
    go(&mut g, |p| { te_style_new(p); te_set_text(p, b"abcd"); te_cal(p); te_sel(p, 0, 2); set_style(p, 2, STYLE, false); te_sel(p, 0, 4); });
    g.m.mem.write_u16(RES, 2 | 1).unwrap(); // ask about doFace | doFont
    go(&mut g, |p| { p.clr_w().l(RES).l(NEW_STYLE).rgn_arg(TEH); disp(p, 10); p.pop_w(RES + 4); });
    assert_eq!(g.m.mem.read_u16(RES).unwrap(), 1, "mode out: the font is continuous, the face is not");
    assert_eq!(g.m.mem.read_u16(RES + 4).unwrap(), 0, "FALSE: not everything asked about is continuous");
    // negative control: over the bold run alone the face is continuous too
    go(&mut g, |p| { te_sel(p, 0, 2); });
    g.m.mem.write_u16(RES, 2 | 1).unwrap();
    go(&mut g, |p| { p.clr_w().l(RES).l(NEW_STYLE).rgn_arg(TEH); disp(p, 10); p.pop_w(RES + 4); });
    assert_eq!((g.m.mem.read_u16(RES).unwrap(), g.m.mem.read_u16(RES + 4).unwrap(), g.m.mem.read_u8(NEW_STYLE + 2).unwrap()), (3, 0x0100, 1));
}

#[test]
fn tegetpoint_tegetheight_tegetoffset_and_tenumstyles_answer_from_the_layout() {
    let mut g = rig();
    put_style(&mut g, STYLE, 0, 1, 0);
    put(&mut g.m, TEXT, b"hello world, this is text"); // 4 lines of 10 characters (see the plain-record test)
    go(&mut g, |p| { te_style_new(p); te_set_text(p, b"hello world, this is text"); te_cal(p); te_sel(p, 2, 4); set_style(p, 2, STYLE, false); });
    let lh = g.m.qd.font.y_size as i32;
    let asc = g.m.qd.font.baseline as i32 + 1;
    // TEGetPoint(offset 8: line 2 starts at 6, so 'w'+2): v = destTop + line * lh + ascent, h = destLeft + width of the line's first chars
    go(&mut g, |p| { p.clr_l().w(8).rgn_arg(TEH); disp(p, 8); p.pop_l(RES); });
    assert_eq!(g.m.mem.read_u32(RES).unwrap(), (((10 + lh + asc) as u32) << 16) | (10 + 16), "line 1 (0-based), two characters in");
    // TEGetHeight(endLine, startLine): lines 1..=4 are four lines high
    go(&mut g, |p| { p.clr_l().l(4).l(1).rgn_arg(TEH); disp(p, 9); p.pop_l(RES); });
    assert_eq!(g.m.mem.read_u32(RES).unwrap(), (4 * lh) as u32);
    go(&mut g, |p| { p.clr_l().l(2).l(2).rgn_arg(TEH); disp(p, 9); p.pop_l(RES); });
    assert_eq!(g.m.mem.read_u32(RES).unwrap(), lh as u32, "one line");
    // TEGetOffset(pt): the same answer TEClick would use
    go(&mut g, |p| { p.clr_w().l((u32::from((10 + lh + 2) as u16) << 16) | u32::from((10 + 16 + 3) as u16)).rgn_arg(TEH).trap(0xA83C).pop_w(RES); });
    assert_eq!(g.m.mem.read_u16(RES).unwrap(), 8, "3 pixels into the third character of line 1 is before it: offset 6 + 2");
    // TENumStyles(rangeStart, rangeEnd): runs inside the range, counting the start
    go(&mut g, |p| { p.clr_l().l(0).l(25).rgn_arg(TEH); disp(p, 13); p.pop_l(RES); });
    assert_eq!(g.m.mem.read_u32(RES).unwrap(), 3, "plain, bold, plain");
    go(&mut g, |p| { p.clr_l().l(0).l(2).rgn_arg(TEH); disp(p, 13); p.pop_l(RES); });
    assert_eq!(g.m.mem.read_u32(RES).unwrap(), 1);
    g.no_refusals();
}

#[test]
fn outline_highlight_frames_a_deactivated_selection_only_when_the_feature_is_set() {
    // SimpleText 1.3 sets teFOutlineHilite (feature 2): a record that is not active shows its selection as an outline, the way a window
    // that is not frontmost shows its text selection.
    let mut g = rig();
    put(&mut g.m, TEXT, b"hello");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"hello"); te_cal(p); te_sel(p, 0, 5); te_update(p); });
    let lh = rec(&g, T_LINE_HEIGHT) as i32;
    let plain = g.count_in(10, 10, 90, 50);
    let left_edge = |g: &Rig| g.count_in(10, 10, 11, 10 + lh);
    assert!(left_edge(&g) < lh as usize, "no frame yet: the edge column is not solid");
    go(&mut g, |p| { p.clr_w().w(2).w(1).rgn_arg(TEH); disp(p, 14); p.pop_w(RES); p.clr_w().w(2).w(-1).rgn_arg(TEH); disp(p, 14); p.pop_w(RES + 2); });
    assert_eq!((g.m.mem.read_u16(RES).unwrap(), g.m.mem.read_u16(RES + 2).unwrap()), (0, 1), "set answers the previous state (clear); a test then reads set");
    go(&mut g, |p| { te_update(p); });
    assert_eq!(left_edge(&g), lh as usize, "the outline's left edge is solid black");
    assert_eq!(g.count_in(10, 10 + lh - 1, 50, 10 + lh), 40, "and so is its bottom edge");
    assert!(g.count_in(10, 10, 90, 50) > plain, "the frame adds ink around the text");
    // an ACTIVE record inverts as before (no frame): TEActivate then update
    go(&mut g, |p| { one(p, 0xA9D8); });
    assert_eq!(g.count_in(10, 10, 90, 10 + lh), 5 * 8 * lh as usize - plain, "active: the plain inversion");
    // clearing the feature takes the frame away again
    go(&mut g, |p| { one(p, 0xA9D9); p.clr_w().w(2).w(0).rgn_arg(TEH); disp(p, 14); p.pop_w(RES); te_update(p); });
    assert_eq!((g.m.mem.read_u16(RES).unwrap(), left_edge(&g) < lh as usize, g.count_in(10, 10, 90, 50)), (1, true, plain));
    g.no_refusals();
}

#[test]
fn unimplemented_dispatch_selectors_and_feature_setting_are_named_and_pop_their_frames() {
    let mut g = rig();
    go(&mut g, |p| { te_style_new(p); });
    // TEStylePaste (selector 0) is not implemented: it pops its frame (a marker pushed first comes back intact) and says so
    go(&mut g, |p| { p.w(0x7777).rgn_arg(TEH); disp(p, 0); p.pop_w(RES); });
    assert_eq!(g.m.mem.read_u16(RES).unwrap(), 0x7777, "the frame was popped exactly");
    assert!(g.m.refused.iter().any(|r| r.contains("TEStylePaste")), "{:?}", g.m.refused);
    g.m.refused.clear();
    // TEFeatureFlag(feature, action): test reads clear, set is refused by name
    go(&mut g, |p| { p.clr_w().w(0).w(-1).rgn_arg(TEH); disp(p, 14); p.pop_w(RES); });
    assert_eq!((g.m.mem.read_u16(RES).unwrap(), g.m.refused.len()), (0, 0));
    go(&mut g, |p| { p.clr_w().w(3).w(1).rgn_arg(TEH); disp(p, 14); p.pop_w(RES); }); // feature 3 (inline input), action set
    assert!(g.m.refused.iter().any(|r| r.contains("TEFeatureFlag")), "{:?}", g.m.refused);
    // a selector that does not exist cannot be popped: named, stack untouched
    g.m.refused.clear();
    go(&mut g, |p| { disp(p, 99); });
    assert!(g.m.refused.iter().any(|r| r.contains("selector 99")), "{:?}", g.m.refused);
}

#[test]
fn teautoview_pinscroll_and_selview_keep_the_selection_in_the_view() {
    let mut g = rig();
    // 4-line text in a view two lines high
    let lh = g.m.qd.font.y_size as i16;
    set_rect(&mut g.m, VIEW, 10, 10, 10 + 2 * lh, 90);
    set_rect(&mut g.m, DEST, 10, 10, 200, 90);
    put(&mut g.m, TEXT, b"hello world, this is text");
    go(&mut g, |p| { te_new(p); te_set_text(p, b"hello world, this is text"); te_cal(p); te_sel(p, 25, 25); });
    let dest_top = |g: &Rig| g.m.mem.read_i16(g.m.mm.block(teh(g)).unwrap().ptr + T_DEST).unwrap();
    // auto-scroll off: TESelView does nothing
    go(&mut g, |p| { one(p, 0xA811); });
    assert_eq!(dest_top(&g), 10, "auto-scroll is off by default");
    // TEAutoView(TRUE): the caret on line 4 scrolls into the two-line view (the least scroll: two lines up)
    go(&mut g, |p| { p.b(1).rgn_arg(TEH).trap(0xA813); one(p, 0xA811); });
    assert_eq!(dest_top(&g), 10 - 2 * lh, "scrolled so the last line is the view's last line");
    // TEPinScroll never scrolls past the text: asking for ten lines up from here only goes to the end
    go(&mut g, |p| { p.w(0).w(-10 * lh).rgn_arg(TEH).trap(0xA812); });
    assert_eq!(dest_top(&g), 10 - 2 * lh, "already at the end");
    // ...nor below the top
    go(&mut g, |p| { p.w(0).w(10 * lh).rgn_arg(TEH).trap(0xA812); });
    assert_eq!(dest_top(&g), 10, "back at the top and no further");
    // TEFeatureFlag(teFAutoScroll) reads and writes the same flag and answers the previous state
    go(&mut g, |p| { p.clr_w().w(0).w(-1).rgn_arg(TEH); disp(p, 14); p.pop_w(RES); });
    assert_eq!(g.m.mem.read_u16(RES).unwrap(), 1, "test: set");
    go(&mut g, |p| { p.clr_w().w(0).w(0).rgn_arg(TEH); disp(p, 14); p.pop_w(RES); p.clr_w().w(0).w(-1).rgn_arg(TEH); disp(p, 14); p.pop_w(RES + 2); });
    assert_eq!((g.m.mem.read_u16(RES).unwrap(), g.m.mem.read_u16(RES + 2).unwrap()), (1, 0), "clear returns the previous state; the flag is clear now");
    g.no_refusals();
}
