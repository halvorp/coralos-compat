//! Fonts and text: the resident topaz 8 as a real guest TextFont, OpenFont/AskFont/SetFont, TextLength/TextExtent/TextFit,
//! Text on a bitmap, soft styles, and a font the PROGRAM builds in guest memory (proportional, kerned) drawn by the same
//! renderer. Controls: other sizes/names are not resident, other characters have other widths.
mod gfxcommon;
use gfxcommon::*;

const ROM_FONT: u32 = 0x8000;
const OPEN_FONT: i32 = -72;
const CLOSE_FONT: i32 = -78;
const SET_FONT: i32 = -66;
const ADD_FONT: i32 = -480;
const TEXT_EXTENT: i32 = -690;
const TEXT_FIT: i32 = -696;
const SET_SOFT_STYLE: i32 = -90;
const ASK_SOFT_STYLE: i32 = -84;

fn text_attr(g: &mut G, name: &str, y: u16) -> u32 {
    let n = g.s(name);
    let ta = g.zeroed(8);
    g.a.mem.write_u32(ta, n).unwrap();
    g.a.mem.write_u16(ta + 4, y).unwrap();
    ta
}

#[test]
fn topaz8_is_resident_with_real_metrics_and_other_fonts_are_not() {
    let mut g = G::new();
    let t8 = text_attr(&mut g, "topaz.font", 8);
    let t9 = text_attr(&mut g, "topaz.font", 9);
    let up = text_attr(&mut g, "TOPAZ.FONT", 8);
    let other = text_attr(&mut g, "nosuch.font", 8);
    g.run(&[gfx(OPEN_FONT, &[a(0, t8)]), gfx(OPEN_FONT, &[a(0, t9)]), gfx(OPEN_FONT, &[a(0, up)]), gfx(OPEN_FONT, &[a(0, other)])]);
    assert_eq!([g.result(0), g.result(1), g.result(2), g.result(3)], [ROM_FONT, 0, ROM_FONT, 0], "size 9 and an unknown name are not resident");
    let tf = ROM_FONT;
    assert_eq!((g.r16(tf + 20), g.r16(tf + 24), g.r16(tf + 26), g.r8(tf + 32), g.r8(tf + 33), g.r16(tf + 38)), (8, 8, 6, 32, 255, 192), "YSize XSize Baseline LoChar HiChar Modulo");
    assert_eq!(g.r16(tf + 30), 2, "tf_Accessors counts the two OpenFonts");
    let loc = g.r32(tf + 40);
    assert_eq!(g.r32(loc + 4 * (b'A' - 32) as u32), (33 * 8) << 16 | 8, "CharLoc of 'A': bit offset 264, width 8");
    assert_eq!(g.r8(g.r32(tf + 34) + 192 + 33), 0x18, "CharData row 1 of 'A' (Modulo 192 * 1 + byte 33) is 0x18");
    g.run(&[gfx(CLOSE_FONT, &[a(1, ROM_FONT)])]);
    assert_eq!(g.r16(tf + 30), 1);
}

#[test]
fn text_length_extent_and_fit_follow_the_font_advance() {
    let mut g = G::new();
    let (_bm, rp) = g.bitmap_rp(64, 16, 3);
    let hello = g.s("Hello");
    let long = g.s("abcdefgh");
    let te = g.zeroed(12);
    let fit_te = g.zeroed(12);
    g.run(&[gfx(TEXT_LENGTH, &[a(1, rp), a(0, hello), d(0, 5)]), gfx(TEXT_LENGTH, &[a(1, rp), a(0, hello), d(0, 0)]),
        gfx(TEXT_EXTENT, &[a(1, rp), a(0, hello), d(0, 5), a(2, te)]),
        gfx(TEXT_FIT, &[a(1, rp), a(0, long), d(0, 8), a(2, fit_te), a(3, 0), d(1, 1), d(2, 20), d(3, 8)]),
        gfx(TEXT_FIT, &[a(1, rp), a(0, long + 7), d(0, 8), a(2, 0), a(3, 0), di(1, -1), d(2, 24), d(3, 8)]),
        gfx(TEXT_FIT, &[a(1, rp), a(0, long), d(0, 8), a(2, 0), a(3, 0), d(1, 1), d(2, 20), d(3, 7)])]);
    assert_eq!([g.result(0), g.result(1)], [40, 0]);
    assert_eq!([g.r16(te), g.r16(te + 2)], [40, 8], "te_Width, te_Height");
    assert_eq!([g.r16(te + 4) as i16, g.r16(te + 6) as i16, g.r16(te + 8) as i16, g.r16(te + 10) as i16], [0, -6, 39, 1], "te_Extent: MinX MinY MaxX MaxY");
    assert_eq!(g.result(3), 2, "20 pixels fit two 8-pixel characters");
    assert_eq!(g.r16(fit_te), 16, "the extent of the fitted text");
    assert_eq!(g.result(4), 3, "from the end, strDirection -1");
    assert_eq!(g.result(5), 0, "a constraining height of 7 < 8 fits nothing");
}

#[test]
fn text_draws_glyphs_at_the_baseline_advances_the_cursor_and_jam2_fills_the_cell() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(32, 16, 3);
    let ab = g.s("AB");
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 1)]), gfx(SET_BPEN, &[a(1, rp), d(0, 3)]), gfx(MOVE, &[a(1, rp), d(0, 4), d(1, 10)]), gfx(TEXT, &[a(1, rp), a(0, ab), d(0, 2)])]);
    assert_eq!((g.r16(rp + 36), g.r16(rp + 38)), (20, 10), "cp_x advanced by 16");
    // 'A' cell is x 4..12, y 4..12 (top = 10 - baseline 6); row 1 of A is 0x18: x=3,4 within the glyph
    assert_eq!(g.rows(bm, 4, 5, 8, 1)[0], [3, 3, 3, 1, 1, 3, 3, 3]);
    assert_eq!(g.rows(bm, 4, 4, 8, 1)[0], [3; 8], "row 0 of the cell: background (JAM2)");
    assert_eq!(g.px(bm, 3, 5), Some(0), "control: left of the cell is untouched");
    assert_eq!(g.px(bm, 20, 5), Some(0), "control: right of the last cell");
    g.run(&[gfx(SET_DRMD, &[a(1, rp), d(0, 0)]), gfx(MOVE, &[a(1, rp), d(0, 4), d(1, 14)]), gfx(TEXT, &[a(1, rp), a(0, ab), d(0, 2)])]);
    assert_eq!(g.px(bm, 4, 12), Some(0), "JAM1 leaves the cell's paper: glyph column 0 of row 4 is blank, and nothing was painted there before");
    assert_eq!(g.px(bm, 5, 12), Some(1), "control: the ink of row 4 (0x7e) is drawn");
}

#[test]
fn soft_styles_are_masked_by_what_the_font_already_has_and_bold_widens_the_ink() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(32, 16, 3);
    let i = g.s("I");
    g.run(&[gfx(ASK_SOFT_STYLE, &[a(1, rp)]), gfx(SET_SOFT_STYLE, &[a(1, rp), d(0, 2), d(1, 7)]), gfx(SET_APEN, &[a(1, rp), d(0, 1)]), gfx(SET_DRMD, &[a(1, rp), d(0, 0)]),
        gfx(MOVE, &[a(1, rp), d(0, 0), d(1, 10)]), gfx(TEXT, &[a(1, rp), a(0, i), d(0, 1)])]);
    assert_eq!(g.result(0) & 7, 7, "topaz has none of underline/bold/italic natively");
    assert_eq!(g.r8(rp + 56), 2, "AlgoStyle = bold");
    let plain = g.rows(bm, 0, 6, 10, 1)[0].iter().filter(|&&p| p == 1).count();
    let (bm2, rp2) = g.bitmap_rp(32, 16, 3);
    g.run(&[gfx(SET_APEN, &[a(1, rp2), d(0, 1)]), gfx(SET_DRMD, &[a(1, rp2), d(0, 0)]), gfx(MOVE, &[a(1, rp2), d(0, 0), d(1, 10)]), gfx(TEXT, &[a(1, rp2), a(0, i), d(0, 1)])]);
    let normal = g.rows(bm2, 0, 6, 10, 1)[0].iter().filter(|&&p| p == 1).count();
    assert!(plain > normal, "bold smears the glyph one pixel: {plain} > {normal}");
}

#[test]
fn a_font_the_program_builds_in_guest_memory_is_added_opened_and_drawn() {
    let mut g = G::new();
    // glyph strip (modulo 2, 4 rows): 'A' bits 0-2, 'B' bits 4-5, default glyph bits 8-10
    let (arow, brow) = ([0b111u8, 0b101, 0b111, 0b101], [0b11u8, 0b11, 0b01, 0b11]);
    let mut strip = vec![];
    for r in 0..4 { strip.push(arow[r] << 5 | brow[r] << 2); strip.push(0xE0); }
    let data = g.bytes(&strip);
    let loc = g.bytes(&[0, 0, 0, 3, 0, 4, 0, 2, 0, 8, 0, 3]);
    let space = g.words(&[4, 3, 5]);
    let kern = g.words(&[0, 1, 0]);
    let name = g.s("mine.font");
    let tf = g.zeroed(52);
    let m = &mut g.a.mem;
    m.write_u8(tf + 8, 12).unwrap(); m.write_u32(tf + 10, name).unwrap();
    m.write_u16(tf + 20, 4).unwrap(); m.write_u8(tf + 23, 0x20).unwrap(); m.write_u16(tf + 24, 4).unwrap(); m.write_u16(tf + 26, 3).unwrap();
    m.write_u8(tf + 32, 65).unwrap(); m.write_u8(tf + 33, 66).unwrap(); m.write_u32(tf + 34, data).unwrap(); m.write_u16(tf + 38, 2).unwrap();
    m.write_u32(tf + 40, loc).unwrap(); m.write_u32(tf + 44, space).unwrap(); m.write_u32(tf + 48, kern).unwrap();
    let ta = text_attr(&mut g, "mine.font", 4);
    let (bm, rp) = g.bitmap_rp(24, 8, 3);
    let (ab, ac) = (g.s("AB"), g.s("AC"));
    g.run(&[gfx(OPEN_FONT, &[a(0, ta)]), gfx(ADD_FONT, &[a(1, tf)]), gfx(OPEN_FONT, &[a(0, ta)]), gfx(SET_FONT, &[a(1, rp), a(0, tf)]),
        gfx(TEXT_LENGTH, &[a(1, rp), a(0, ab), d(0, 2)]), gfx(TEXT_LENGTH, &[a(1, rp), a(0, ac), d(0, 2)])]);
    assert_eq!((g.result(0), g.result(2)), (0, tf), "not resident until AddFont");
    assert_eq!((g.r16(rp + 58), g.r16(rp + 60), g.r16(rp + 62)), (4, 4, 3), "SetFont copies TxHeight/TxWidth/TxBaseline");
    assert_eq!((g.result(4), g.result(5)), (8, 9), "advance = CharSpace + CharKern; chars outside lo..hi use the default glyph's space");
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 5)]), gfx(SET_DRMD, &[a(1, rp), d(0, 0)]), gfx(MOVE, &[a(1, rp), d(0, 0), d(1, 3)]), gfx(TEXT, &[a(1, rp), a(0, ab), d(0, 2)])]);
    let rows = g.rows(bm, 0, 0, 9, 4);
    let ink: Vec<String> = rows.iter().map(|r| r.iter().map(|&p| if p == 5 { '#' } else { '.' }).collect()).collect();
    // A at x 0..3 (kern 0); B drawn at cursor 4 + kern 1 = x 5..7
    assert_eq!(ink, ["###..##..", "#.#..##..", "###...#..", "#.#..##.."].map(String::from).to_vec());
}
