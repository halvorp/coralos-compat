//! CopyBits (window <-> offscreen BitMap, masks, modes) and text.
mod common;
use common::*;
use retro_raster::font::Font;

const BM: u32 = 0x3300; // an offscreen BitMap: base, rowBytes, bounds
const BITS: u32 = 0x3400; // its 32x16 pixels, 4 bytes per row
const PORT: u32 = 0x3500; // an OpenPort'd GrafPort record (108 bytes)
const R1: u32 = 0x3100;
const R2: u32 = 0x3108;

fn offscreen(g: &mut Rig) {
    put(&mut g.m, BM, &[0, 0, 0x34, 0x00, 0, 4, 0, 0, 0, 0, 0, 16, 0, 32]); // base 0x3400, rowBytes 4, bounds 0,0,16,32
}
/// Draw into the offscreen BitMap (via OpenPort + SetPortBits), then make the window current again.
fn draw_offscreen(p: &mut Prog, draw: impl FnOnce(&mut Prog), win: u32) {
    p.l(PORT).trap(0xA86F).l(BM).trap(0xA875);
    draw(p);
    p.set_port(win);
}
fn copy(p: &mut Prog, src: u32, dst: u32, sr: u32, dr: u32, mode: i16, mask: u32) { p.l(src).l(dst).l(sr).l(dr).w(mode).l(mask).trap(0xA8EC); }

#[test]
fn drawing_into_an_offscreen_bitmap_writes_real_packed_bits_in_guest_memory() {
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 2, 4, 5, 12);
    let win = g.win;
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, win);
    g.run(p.exit());
    g.no_refusals();
    // rows 2..5, columns 4..12 black: byte 0 = 0x0F (cols 4..7), byte 1 = 0xF0 (cols 8..11)
    for y in 0..16u32 {
        let row = g.m.mem.bytes(g.m.mem.span(BITS + y * 4, 4).unwrap()).unwrap().to_vec();
        assert_eq!(row, if (2..5).contains(&y) { vec![0x0F, 0xF0, 0, 0] } else { vec![0; 4] }, "row {y}");
    }
    assert_eq!(g.count_black(), 0, "the window is untouched (negative control)");
}

#[test]
fn copy_bits_from_offscreen_to_the_window_with_src_copy_and_not_src_copy() {
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 2, 4, 5, 12);
    set_rect(&mut g.m, R2, 0, 0, 16, 32); // whole offscreen
    set_rect(&mut g.m, 0x3110, 10, 20, 26, 52); // destination rect in the window (local)
    let win = g.win;
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, win);
    copy(&mut p, BM, win + 2, R2, 0x3110, 0, 0);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 3 * 8, "the 8x3 block arrives\n{}", g.pic(18, 8, 54, 28));
    assert!(g.black(24, 12) && g.black(31, 14) && !g.black(23, 12) && !g.black(32, 12) && !g.black(24, 15));
    // notSrcCopy (4): the source inverted fills the whole destination rect except the block
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 2, 4, 5, 12);
    set_rect(&mut g.m, R2, 0, 0, 16, 32);
    set_rect(&mut g.m, 0x3110, 10, 20, 26, 52);
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, g.win);
    copy(&mut p, BM, g.win + 2, R2, 0x3110, 4, 0);
    g.run(p.exit());
    assert_eq!(g.count_black(), 16 * 32 - 24);
    assert!(!g.black(24, 12) && g.black(20, 10));
}

#[test]
fn copy_bits_from_the_window_into_the_offscreen_bitmap_packs_the_bits_back() {
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 0, 0, 4, 8);
    set_rect(&mut g.m, R2, 0, 0, 16, 32);
    let win = g.win;
    let mut p = Prog::new();
    p.paint_rect(R1);
    p.l(PORT).trap(0xA86F).l(BM).trap(0xA875).set_port(win); // an offscreen port exists, window current
    copy(&mut p, win + 2, BM, R2, R2, 0, 0);
    g.run(p.exit());
    g.no_refusals();
    let row0 = g.m.mem.bytes(g.m.mem.span(BITS, 4).unwrap()).unwrap().to_vec();
    assert_eq!(row0, vec![0xFF, 0, 0, 0]);
    let row4 = g.m.mem.bytes(g.m.mem.span(BITS + 16, 4).unwrap()).unwrap().to_vec();
    assert_eq!(row4, vec![0; 4], "rows below the painted block stay white");
}

#[test]
fn a_mask_region_limits_the_copy_and_xor_twice_restores() {
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 0, 0, 16, 32);
    set_rect(&mut g.m, 0x3110, 0, 0, 16, 32);
    set_rect(&mut g.m, 0x3118, 0, 0, 16, 16); // mask: the left half, dst local coordinates
    let win = g.win;
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, win);
    p.new_rgn(0x3200).rgn_arg(0x3200).l(0x3118).trap(0xA8DF); // RectRgn
    p.l(BM).l(win + 2).l(R1).l(0x3110).w(0).rgn_arg(0x3200).trap(0xA8EC);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 16 * 16, "only the masked half was copied");
    assert!(g.black(15, 15) && !g.black(16, 0) && !g.black(31, 15));
    // srcXor (2) twice is the identity
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 0, 0, 16, 32);
    set_rect(&mut g.m, 0x3110, 0, 0, 16, 32);
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, g.win);
    copy(&mut p, BM, g.win + 2, R1, 0x3110, 2, 0);
    g.run(p.exit());
    assert_eq!(g.count_black(), 512);
    let mut p = Prog::new();
    copy(&mut p, BM, g.win + 2, R1, 0x3110, 2, 0);
    g.run(p.exit());
    assert_eq!(g.count_black(), 0, "xor twice");
}

#[test]
fn copy_bits_within_a_window_scrolls_overlapping_regions_correctly() {
    let mut g = Rig::new();
    set_rect(&mut g.m, R1, 10, 10, 14, 30);
    set_rect(&mut g.m, R2, 0, 0, 40, 40);
    set_rect(&mut g.m, 0x3110, 3, 3, 43, 43); // down and right by 3, overlapping the source
    let win = g.win;
    let mut p = Prog::new();
    p.paint_rect(R1);
    copy(&mut p, win + 2, win + 2, R2, 0x3110, 0, 0);
    g.run(p.exit());
    g.no_refusals();
    // srcCopy replaces the whole destination rect with the shifted source: exactly one block, at (13,13), none left at (10,10)
    assert_eq!(g.count_black(), 4 * 20, "{}", g.pic(8, 8, 36, 20));
    assert!(g.black(13, 13) && g.black(32, 16) && !g.black(10, 10) && !g.black(12, 12) && !g.black(33, 13), "{}", g.pic(8, 8, 36, 20));
}

#[test]
fn unimplemented_copy_bits_cases_are_refused_by_name_and_touch_nothing() {
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 0, 0, 16, 32);
    set_rect(&mut g.m, R2, 0, 0, 8, 16); // half-size destination: scaling
    let win = g.win;
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, win);
    copy(&mut p, BM, win + 2, R1, R2, 0, 0);
    p.exit();
    g.m.start(&p.0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    // scaling is StretchBits now (qd/stretch.rs): an all-black 32x16 shrunk to 16x8 is an all-black 16x8
    assert!(g.m.refused.is_empty(), "{:?}", g.m.refused);
    assert_eq!(g.count_black(), 16 * 8, "shrunk by 2: the OR of black pixels is black");
    assert_eq!(g.count_in(0, 0, 16, 8), 16 * 8);
    // a color PixMap (rowBytes high bit) is refused by name
    let mut g = Rig::new();
    offscreen(&mut g);
    put(&mut g.m, BM + 4, &[0x80, 0x04]);
    set_rect(&mut g.m, R1, 0, 0, 16, 32);
    let mut p = Prog::new();
    copy(&mut p, BM, g.win + 2, R1, R1, 0, 0);
    p.exit();
    g.m.start(&p.0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    assert!(g.m.refused.iter().any(|r| r.contains("PixMap")), "{:?}", g.m.refused);
    // a pattern transfer mode with a source
    let mut g = Rig::new();
    set_rect(&mut g.m, R1, 0, 0, 8, 8);
    let mut p = Prog::new();
    copy(&mut p, g.win + 2, g.win + 2, R1, R1, 9, 0);
    p.exit();
    g.m.start(&p.0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    assert!(g.m.refused.iter().any(|r| r.contains("transfer mode 9")), "{:?}", g.m.refused);
}

// ---------------------------------------------------------------- text

const STR: u32 = 0x3600;

/// The pen's baseline is a line between rows: the topaz glyph's rows 0..7 occupy `baseline - 7 ..= baseline` (QuickDraw/oracle: the
/// last row of a glyph without descender is `baseline - 1`; the Amiga's baseline ROW is not the Mac's).
fn glyph_matches(g: &Rig, ch: u8, x: i32, baseline: i32) -> bool {
    let f = Font::topaz8();
    (0..8).all(|gy| (0..8).all(|gx| g.black(x + gx, baseline - 7 + gy) == f.glyph_bit(f.slot(ch), gx, gy)))
}

#[test]
fn draw_string_renders_the_font_glyphs_at_the_pen_and_advances_it() {
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("Hi"));
    g.run(Prog::new().move_to(10, 30).draw_string(STR).exit());
    g.no_refusals();
    assert!(glyph_matches(&g, b'H', 10, 30) && glyph_matches(&g, b'i', 18, 30), "{}", g.pic(8, 22, 30, 34));
    assert!(g.count_black() > 10);
    assert_eq!(g.m.mem.read_u32(g.win + 48).unwrap(), (30 << 16) | 26, "pen advanced by two 8-pixel cells");
    // a different origin draws the same glyph elsewhere and not here (negative control)
    assert!(!glyph_matches(&g, b'H', 11, 30));
}

#[test]
fn string_width_char_width_text_width_and_font_info_agree_with_what_is_drawn() {
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("Hello"));
    put(&mut g.m, STR + 0x20, b"abcdef");
    let mut p = Prog::new();
    p.clr_w().l(STR).trap(0xA88C).pop_w(SCRATCH); // StringWidth
    p.clr_w().ch(b'W').trap(0xA88D).pop_w(SCRATCH + 2); // CharWidth
    p.clr_w().l(STR + 0x20).w(1).w(3).trap(0xA886).pop_w(SCRATCH + 4); // TextWidth(buf, 1, 3) = "bcd"
    p.l(SCRATCH + 8).trap(0xA88B); // GetFontInfo
    g.run(p.exit());
    g.no_refusals();
    let rd = |o| g.m.mem.read_u16(SCRATCH + o).unwrap();
    assert_eq!((rd(0), rd(2), rd(4)), (40, 8, 24));
    assert_eq!((rd(8), rd(10), rd(12), rd(14)), (7, 1, 8, 0), "ascent, descent, widMax, leading of the stand-in font");
}

#[test]
fn text_modes_src_or_leaves_the_paper_src_copy_paints_it_and_xor_twice_vanishes() {
    let run = |mode: i16, twice: bool| {
        let mut g = Rig::new();
        put(&mut g.m, STR, &pstr("A"));
        set_rect(&mut g.m, R1, 20, 8, 40, 40);
        let mut p = Prog::new();
        p.paint_rect(R1).w(mode).trap(0xA889).move_to(10, 30).draw_string(STR);
        if twice { p.move_to(10, 30).draw_string(STR); }
        g.run(p.exit());
        g.no_refusals();
        g
    };
    let before = 20 * 32;
    let or = run(1, false);
    assert_eq!(or.count_black(), before, "srcOr onto black changes nothing visible");
    let copy = run(0, false);
    assert!(copy.count_black() < before && glyph_matches(&copy, b'A', 10, 30), "srcCopy paints white paper into the black rect, and the letter is the black ink on it");
    let x1 = run(2, false);
    assert!(x1.count_black() < before, "srcXor: ink flips black to white");
    let x2 = run(2, true);
    assert_eq!(x2.count_black(), before, "xor twice restores the rect");
}

#[test]
fn bold_adds_ink_and_a_hidden_pen_draws_no_text_but_moves() {
    let plain = { let mut g = Rig::new(); put(&mut g.m, STR, &pstr("Hi")); g.run(Prog::new().move_to(10, 30).draw_string(STR).exit()); g.count_black() };
    let bold = { let mut g = Rig::new(); put(&mut g.m, STR, &pstr("Hi")); g.run(Prog::new().ch(1).trap(0xA888).move_to(10, 30).draw_string(STR).exit()); g.count_black() };
    assert!(bold > plain, "bold {bold} vs plain {plain}");
    let mut g = Rig::new();
    put(&mut g.m, STR, &pstr("Hi"));
    g.run(Prog::new().trap(0xA896).move_to(10, 30).draw_string(STR).exit());
    assert_eq!(g.count_black(), 0);
    assert_eq!(g.m.mem.read_u32(g.win + 48).unwrap(), (30 << 16) | 26);
}

#[test]
fn text_font_and_size_are_stored_and_logged_as_the_named_substitution() {
    let mut g = Rig::new();
    g.run(Prog::new().w(3).trap(0xA887).w(9).trap(0xA88A).exit()); // TextFont(geneva=3) TextSize(9)
    assert_eq!((g.m.mem.read_u16(g.win + 68).unwrap(), g.m.mem.read_u16(g.win + 74).unwrap()), (3, 9));
    assert!(g.m.log.iter().any(|l| l.contains("TextFont 3") && l.contains("topaz-8 stand-in")), "{:?}", g.m.log);
}

#[test]
fn draw_char_reads_the_char_from_the_low_byte_and_text_face_stores_the_style_there() {
    let ink = |c: u8| { let mut g = Rig::new(); g.run(Prog::new().move_to(10, 30).ch(c).trap(0xA883).exit()); g.no_refusals(); (g.count_black(), g.pic(10, 24, 18, 32)) };
    let (a, b) = (ink(b'A'), ink(b'B'));
    assert!(a.0 > 0 && a.1 != b.1, "DrawChar('A') and ('B') draw different glyphs\n{}\n{}", a.1, b.1);
    assert_eq!(ink(b' ').0, 0, "negative control: a space draws no ink, so the 'A' above was read from the right byte");
    let mut g = Rig::new();
    g.run(Prog::new().ch(1).trap(0xA888).exit());
    assert_eq!(g.m.mem.read_u8(g.win + 70).unwrap(), 1, "TextFace(bold) lands in txFace");
    let mut g = Rig::new();
    g.run(Prog::new().ch(4).trap(0xA888).move_to(10, 30).ch(b'A').trap(0xA883).exit());
    assert_eq!(g.m.mem.read_u8(g.win + 70).unwrap(), 4);
}

#[test]
fn a_port_pointer_near_the_top_of_the_address_space_is_a_named_refusal_not_a_panic() {
    for (name, mut p) in [
        ("PaintRect", { let mut p = Prog::new(); p.l(SCRATCH).trap(0xA8A2); p }),
        ("DrawString", { let mut p = Prog::new(); p.l(STR).trap(0xA884); p }),
        ("DrawText", { let mut p = Prog::new(); p.l(0xFFFF_FFF0).w(100).w(1).trap(0xA885); p }),
        ("TextFace", { let mut p = Prog::new(); p.ch(1).trap(0xA888); p }),
        ("PenSize", { let mut p = Prog::new(); p.pen_size(2, 2); p }),
        ("OpenRgn", { let mut p = Prog::new(); p.trap(0xA8DA); p }),
    ] {
        let mut g = Rig::new();
        put(&mut g.m, STR, &pstr("x"));
        set_rect(&mut g.m, SCRATCH, 0, 0, 5, 5);
        g.m.mem.write_u32(GLOBALS, 0xFFFF_FFF0).unwrap(); // thePort
        g.m.start(&p.exit().0, 0).unwrap();
        let _ = g.m.run_loop(1_000_000);
        assert!(!g.m.refused.is_empty(), "{name}: expected a named refusal, got {:?}", g.m.log);
    }
}

/// A 16x32 black offscreen bitmap copied to the window at local (0,0)..(16,32) while the window's clip is the 4x4 rect at (2,2).
fn clipped_copy(the_port_is_offscreen: bool, hide_pen: bool) -> Rig {
    const CLIP: u32 = 0x3200;
    let mut g = Rig::new();
    offscreen(&mut g);
    set_rect(&mut g.m, R1, 0, 0, 16, 32);
    set_rect(&mut g.m, 0x3110, 0, 0, 16, 32);
    set_rect(&mut g.m, 0x3118, 2, 2, 6, 6);
    let win = g.win;
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.paint_rect(R1); }, win);
    p.new_rgn(CLIP).rect_rgn(CLIP, 0x3118).rgn_arg(CLIP).trap(0xA879); // SetClip on the window
    if hide_pen { p.trap(0xA896); }
    if the_port_is_offscreen { p.set_port(PORT); }
    copy(&mut p, BM, win + 2, R1, 0x3110, 0, 0);
    g.run(p.exit());
    g.no_refusals();
    g
}

#[test]
fn copy_bits_is_clipped_only_when_the_destination_is_the_current_ports_bits() {
    let g = clipped_copy(false, false);
    assert_eq!(g.count_black(), 16, "thePort is the window: its clipRgn applies\n{}", g.pic(0, 0, 10, 10));
    let g = clipped_copy(true, false);
    assert_eq!(g.count_black(), 16 * 32, "negative control: thePort is another port, so the destination is drawn wide open (NOTPORT)");
}

#[test]
fn copy_bits_to_the_current_port_draws_nothing_while_its_pen_is_hidden() {
    let g = clipped_copy(false, true);
    assert_eq!(g.count_black(), 0, "StdBits: pnVis < 0 goes home");
    let g = clipped_copy(true, true);
    assert_eq!(g.count_black(), 16 * 32, "negative control: the hidden pen belongs to the window, not to the offscreen thePort");
}

#[test]
fn init_port_restores_the_screen_bitmap_the_port_rect_and_the_vis_region() {
    let mut g = Rig::new();
    offscreen(&mut g);
    let win = g.win;
    let mut p = Prog::new();
    p.l(PORT).trap(0xA86F).l(BM).trap(0xA875).l(PORT).trap(0xA86D).set_port(win); // OpenPort, SetPortBits(BM), InitPort
    let mut before = { let mut q = Prog::new(); q.l(PORT).trap(0xA86F).l(BM).trap(0xA875).set_port(win); q };
    let mut h = Rig::new();
    offscreen(&mut h);
    h.run(before.exit());
    let bits = |g: &Rig| g.m.mem.bytes(g.m.mem.span(PORT + 2, 14).unwrap()).unwrap().to_vec();
    let screen = g.m.mem.bytes(g.m.mem.span(GLOBALS - mac68k_engine::quickdraw::OFF_SCREEN_BITS, 14).unwrap()).unwrap().to_vec();
    assert_eq!(bits(&h), g.m.mem.bytes(g.m.mem.span(BM, 14).unwrap()).unwrap().to_vec(), "negative control: SetPortBits did point the port at BM");
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(bits(&g), screen, "InitPort: portBits := screenBits");
    let rect = g.m.mem.bytes(g.m.mem.span(PORT + 16, 8).unwrap()).unwrap().to_vec();
    assert_eq!(rect, screen[6..14].to_vec(), "portRect := screenBits.bounds");
    let vis = g.m.mem.read_u32(PORT + 24).unwrap();
    let r = g.m.read_region(vis).unwrap();
    assert_eq!((r.bounds().x0, r.bounds().y0, r.bounds().x1, r.bounds().y1), (i32::from(i16::from_be_bytes([screen[8], screen[9]])), i32::from(i16::from_be_bytes([screen[6], screen[7]])), i32::from(i16::from_be_bytes([screen[12], screen[13]])), i32::from(i16::from_be_bytes([screen[10], screen[11]]))), "visRgn := the screen bounds");
}

const FMETRIC: u32 = SCRATCH + 0x40;

#[test]
fn font_metrics_is_the_fmetricrec_of_the_font_get_font_info_and_char_width_describe() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.w(3).trap(0xA887).w(12).trap(0xA88A).w(1).trap(0xA888); // TextFont 3, TextSize 12, TextFace bold
    p.l(SCRATCH).trap(0xA88B); // GetFontInfo, for the same port
    p.l(FMETRIC).trap(0xA835); // FontMetrics
    for ch in [b'W', b' ', b'i'] { p.clr_w().ch(ch).trap(0xA88D).pop_w(SCRATCH + 0x20 + 2 * (ch as u32 % 7)); }
    g.run(p.exit());
    g.no_refusals();
    let (m, fixed) = (&g.m.mem, |o: u32| g.m.mem.read_u32(FMETRIC + o).unwrap());
    // ascent, descent, leading, widMax: Fixed, and the INTEGERs GetFontInfo returned (ascent, descent, widMax, leading)
    let info = |o: u32| u32::from(m.read_u16(SCRATCH + o).unwrap()) << 16;
    assert_eq!((fixed(0), fixed(4), fixed(8), fixed(12)), (info(0), info(2), info(6), info(4)));
    assert_eq!((fixed(0), fixed(4), fixed(8), fixed(12)), (7 << 16, 1 << 16, 0, 8 << 16));
    // wTabHandle: a live handle to the 1072-byte WidthTable, also in WidthTabHandle ($B2A) and WidthPtr ($B10)
    let h = fixed(16);
    assert_eq!(m.read_u32(0xB2A).unwrap(), h);
    let ptr = g.m.mm.block(h).expect("wTabHandle is a Memory Manager handle").ptr;
    assert_eq!((g.m.mm.handle_size(h).unwrap(), m.read_u32(0xB10).unwrap(), m.read_u32(h).unwrap()), (1072, ptr, ptr));
    let tab = |o: u32| m.read_u32(ptr + o).unwrap();
    let tab16 = |o: u32| m.read_u16(ptr + o).unwrap();
    // tabData[c] = Fixed CharWidth(c): 'W', ' ' and 'i' were measured above at SCRATCH + 0x20 + ...
    for ch in [b'W', b' ', b'i'] {
        assert_eq!(tab(4 * ch as u32), u32::from(m.read_u16(SCRATCH + 0x20 + 2 * (ch as u32 % 7)).unwrap()) << 16, "tabData[{}]", ch as char);
    }
    assert_eq!((tab(4 * b'W' as u32), tab(4 * b' ' as u32)), (8 << 16, 8 << 16));
    // fID 3, fSize 12, face 1 (bold), device 0, inNumer/inDenom 1/1, aFID 3, aFace bold, tabSize 1072
    assert_eq!([tab16(1036), tab16(1038), tab16(1040), tab16(1042)], [3, 12, 1, 0]);
    assert_eq!([tab16(1044), tab16(1046), tab16(1048), tab16(1050), tab16(1052)], [1, 1, 1, 1, 3]);
    assert_eq!((m.read_u8(ptr + 1058).unwrap(), m.read_u8(ptr + 1059).unwrap(), tab16(1070)), (0, 1, 1072));
}

#[test]
fn font_metrics_called_twice_keeps_one_width_table_and_refreshes_it_for_the_current_font() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.w(4).trap(0xA887).l(FMETRIC).trap(0xA835).w(9).trap(0xA887).l(FMETRIC + 0x20).trap(0xA835);
    g.run(p.exit());
    g.no_refusals();
    let (h1, h2) = (g.m.mem.read_u32(FMETRIC + 16).unwrap(), g.m.mem.read_u32(FMETRIC + 0x20 + 16).unwrap());
    assert_eq!(h1, h2, "one global table");
    let ptr = g.m.mm.block(h1).unwrap().ptr;
    assert_eq!(g.m.mem.read_u16(ptr + 1036).unwrap(), 9, "rebuilt for the font in use");
}

#[test]
fn measure_text_gives_the_left_edge_of_every_character_and_the_end() {
    let mut g = Rig::new();
    let w = g.m.qd.font.text_length(b"a") as u16;
    put(&mut g.m, 0x3300, b"abc");
    let mut p = Prog::new();
    p.w(3).l(0x3300).l(0x3400).trap(0xA837);
    g.run(p.exit());
    g.no_refusals();
    let locs: Vec<u16> = (0..4).map(|i| g.m.mem.read_u16(0x3400 + 2 * i).unwrap()).collect();
    assert_eq!(locs, [0, w, 2 * w, 3 * w], "charLocs[0] = 0, then each cumulative width, count + 1 entries");
    // negative control: a count of 0 writes only charLocs[0]
    g.m.mem.write_u16(0x3420, 0x5555).unwrap();
    let mut p = Prog::new();
    p.w(0).l(0x3300).l(0x3420).trap(0xA837);
    g.run(p.exit());
    assert_eq!(g.m.mem.read_u16(0x3420).unwrap(), 0);
    assert_eq!(g.m.mem.read_u16(0x3422).unwrap(), 0, "and nothing past it");
}

#[test]
fn text_drawn_into_a_port_over_an_empty_bitmap_is_clipped_away_not_refused() {
    // WriteNow 2.2 measures text in a scratch port whose BitMap has bounds (0,0,0,104): zero rows. There is nothing to draw on, so nothing is drawn.
    let mut g = Rig::new();
    put(&mut g.m, BM, &[0, 0, 0x34, 0x00, 0, 14, 0, 0, 0, 0, 0, 0, 0, 104]); // base 0x3400, rowBytes 14, bounds 0,0,0,104
    put(&mut g.m, 0x3600, &pstr("abc"));
    let win = g.win;
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.move_to(5, 5).draw_string(0x3600); }, win);
    g.run(p.exit());
    g.no_refusals();
    assert!(g.m.log.iter().any(|l| l.contains("has an empty BitMap: nothing is drawn")), "{:#?}", g.m.log);
    assert_eq!(g.count_black(), 0, "and the window is untouched");
    // negative control: the same string into a BitMap with rows is drawn
    put(&mut g.m, BM, &[0, 0, 0x34, 0x00, 0, 14, 0, 0, 0, 0, 0, 16, 0, 104]);
    let mut p = Prog::new();
    draw_offscreen(&mut p, |p| { p.move_to(5, 10).draw_string(0x3600); }, win);
    g.run(p.exit());
    g.no_refusals();
    let ink = g.m.mem.bytes(g.m.mem.span(0x3400, 16 * 14).unwrap()).unwrap().iter().filter(|&&b| b != 0).count();
    assert!(ink > 0, "text drawn into a real offscreen BitMap");
}
