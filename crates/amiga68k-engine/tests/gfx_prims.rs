//! graphics.library drawing primitives through the real LVO bridge: a generated call script makes each call with exact
//! register arguments; pixels are read back from the host bitmap behind the guest's BitMap and, where the API has one,
//! through ReadPixel. Every behaviour test has a negative control (the same thing without the call, or the other mode).
mod gfxcommon;
use gfxcommon::*;

const CP_X: u32 = 36;
const CP_Y: u32 = 38;

#[test]
fn rect_fill_paints_the_inclusive_rectangle_in_the_pen_and_complement_inverts_it() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(32, 16, 3);
    assert!(g.rows(bm, 0, 0, 32, 16).iter().flatten().all(|&p| p == 0), "control: a fresh bitmap is pen 0");
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 5)]), gfx(RECT_FILL, &[a(1, rp), d(0, 2), d(1, 2), d(2, 9), d(3, 5)])]);
    let r = g.rows(bm, 0, 0, 12, 8);
    assert_eq!(r[2], [0, 0, 5, 5, 5, 5, 5, 5, 5, 5, 0, 0], "x 2..=9 inclusive");
    assert_eq!((r[1][5], r[5][5], r[6][5]), (0, 5, 0), "y 2..=5 inclusive");
    g.run(&[gfx(SET_DRMD, &[a(1, rp), d(0, 2)]), gfx(RECT_FILL, &[a(1, rp), d(0, 4), d(1, 3), d(2, 12), d(3, 3)])]);
    assert_eq!(g.px(bm, 5, 3), Some(5 ^ 7), "COMPLEMENT inverts within the depth");
    assert_eq!(g.px(bm, 11, 3), Some(7), "complement of pen 0 at depth 3 is 7");
    g.run(&[gfx(RECT_FILL, &[a(1, rp), d(0, 4), d(1, 3), d(2, 12), d(3, 3)])]);
    assert_eq!(g.px(bm, 5, 3), Some(5), "twice restores");
}

#[test]
fn jam1_leaves_the_paper_and_the_rect_is_clipped_to_the_bitmap() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(8, 8, 3);
    let pat = g.words(&[0xAAAA, 0xAAAA]);
    g.a.mem.write_u32(rp + 8, pat).unwrap(); // AreaPtrn; AreaPtSz stays 0 = no pattern
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 6)]), gfx(SET_BPEN, &[a(1, rp), d(0, 2)]), gfx(SET_DRMD, &[a(1, rp), d(0, 0)]),
        gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 7), d(2, 7), d(3, 7)])]);
    assert_eq!(g.rows(bm, 0, 7, 4, 1)[0], [6, 6, 6, 6], "control: AreaPtSz 0 means solid");
    g.a.mem.write_u8(rp + 29, 1).unwrap(); // 2^1 pattern rows
    g.run(&[gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 7), d(3, 0)])]);
    assert_eq!(g.rows(bm, 0, 0, 4, 1)[0], [6, 0, 6, 0], "JAM1 + pattern 0xAAAA: ink columns only");
    g.run(&[gfx(SET_DRMD, &[a(1, rp), d(0, 1)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 1), d(2, 7), d(3, 1)])]);
    assert_eq!(g.rows(bm, 0, 1, 4, 1)[0], [6, 2, 6, 2], "JAM2: the paper in the background pen");
    g.run(&[gfx(RECT_FILL, &[a(1, rp), di(0, -5), di(1, -5), d(2, 100), d(3, 100)])]);
    assert!(g.a.refused.is_empty(), "an oversize rectangle clips to the bitmap without a fault: {:?}", g.a.refused);
}

#[test]
fn draw_moves_the_cursor_and_plots_a_bresenham_line_with_the_line_pattern() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(16, 8, 3);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 3)]), gfx(MOVE, &[a(1, rp), d(0, 1), d(1, 1)]), gfx(DRAW, &[a(1, rp), d(0, 6), d(1, 6)])]);
    assert!((1..=6).all(|i| g.px(bm, i, i) == Some(3)));
    assert_eq!(g.px(bm, 2, 1), Some(0), "control: off the diagonal");
    assert_eq!((g.r16(rp + CP_X), g.r16(rp + CP_Y)), (6, 6), "the cursor is where the line ended");
    g.a.mem.write_u16(rp + 34, 0xF0F0).unwrap(); // LinePtrn
    g.run(&[gfx(MOVE, &[a(1, rp), d(0, 0), d(1, 7)]), gfx(DRAW, &[a(1, rp), d(0, 15), d(1, 7)])]);
    assert_eq!(g.rows(bm, 0, 7, 16, 1)[0], [3, 3, 3, 3, 0, 0, 0, 0, 3, 3, 3, 3, 0, 0, 0, 0], "LinePtrn 0xF0F0, bit 15 first");
    g.run(&[gfx(MOVE, &[a(1, rp), di(0, -20), di(1, -20)]), gfx(DRAW, &[a(1, rp), d(0, 40), d(1, 40)])]);
    assert!(g.a.refused.is_empty(), "a line far outside the bitmap is clipped, not refused: {:?}", g.a.refused);
}

#[test]
fn write_pixel_and_read_pixel_agree_and_report_outside_as_minus_one() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(8, 8, 3);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 4)]), gfx(WRITE_PIXEL, &[a(1, rp), d(0, 3), d(1, 2)]), gfx(READ_PIXEL, &[a(1, rp), d(0, 3), d(1, 2)]),
        gfx(READ_PIXEL, &[a(1, rp), d(0, 4), d(1, 2)]), gfx(READ_PIXEL, &[a(1, rp), d(0, 20), d(1, 2)]), gfx(WRITE_PIXEL, &[a(1, rp), di(0, -1), d(1, 2)])]);
    assert_eq!([g.result(1), g.result(2), g.result(3), g.result(4), g.result(5)], [0, 4, 0, u32::MAX, u32::MAX]);
    assert_eq!(g.px(bm, 3, 2), Some(4));
}

#[test]
fn erase_rect_uses_the_background_pen_and_scroll_raster_moves_the_content_and_erases_the_strip() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(16, 4, 3);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 5)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 15), d(3, 3)]), gfx(SET_BPEN, &[a(1, rp), d(0, 2)]),
        gfx(ERASE_RECT, &[a(1, rp), d(0, 4), d(1, 1), d(2, 7), d(3, 2)])]);
    assert_eq!(g.rows(bm, 3, 1, 6, 1)[0], [5, 2, 2, 2, 2, 5]);
    assert_eq!(g.px(bm, 5, 0), Some(5), "control: outside the erased rectangle");
    // a marker column at x=8, then scroll the whole width left by 3 (dx = +3: the view moves right)
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 7)]), gfx(RECT_FILL, &[a(1, rp), d(0, 8), d(1, 0), d(2, 8), d(3, 3)]), gfx(SET_BPEN, &[a(1, rp), d(0, 1)]),
        gfx(SCROLL_RASTER, &[a(1, rp), d(0, 3), d(1, 0), d(2, 0), d(3, 0), d(4, 15), d(5, 3)])]);
    assert_eq!(g.px(bm, 5, 0), Some(7), "the marker moved from x=8 to x=5");
    assert_eq!(g.px(bm, 8, 0), Some(5), "control: x=8 now holds what was at x=11");
    assert_eq!(g.rows(bm, 13, 0, 3, 1)[0], [1, 1, 1], "the uncovered strip is erased with the background pen");
}

#[test]
fn bltbitmap_copies_with_every_common_minterm_and_clips() {
    let mut g = G::new();
    let (sbm, srp) = g.bitmap_rp(8, 4, 3);
    let (dbm, drp) = g.bitmap_rp(8, 4, 3);
    g.run(&[gfx(SET_APEN, &[a(1, srp), d(0, 0b011)]), gfx(RECT_FILL, &[a(1, srp), d(0, 0), d(1, 0), d(2, 7), d(3, 3)]),
        gfx(SET_APEN, &[a(1, drp), d(0, 0b101)]), gfx(RECT_FILL, &[a(1, drp), d(0, 0), d(1, 0), d(2, 7), d(3, 3)])]);
    let blt = |g: &mut G, mt: u32, mask: u32, dx: u32| g.run(&[gfx(BLT_BITMAP, &[a(0, sbm), d(0, 0), d(1, 0), a(1, dbm), d(2, dx), d(3, 0), d(4, 2), d(5, 1), d(6, mt), d(7, mask), a(2, 0)])]);
    for (mt, want) in [(0xC0u32, 0b011u8), (0x30, !0b011 & 7), (0x50, !0b101 & 7), (0x60, 0b011 ^ 0b101), (0x80, 0b011 & 0b101), (0xE0, 0b011 | 0b101)] {
        g.run(&[gfx(SET_APEN, &[a(1, drp), d(0, 0b101)]), gfx(RECT_FILL, &[a(1, drp), d(0, 0), d(1, 0), d(2, 7), d(3, 3)])]);
        blt(&mut g, mt, 0xFF, 1);
        assert_eq!(g.px(dbm, 1, 0), Some(want), "minterm {mt:#x}");
        assert_eq!((g.px(dbm, 0, 0), g.px(dbm, 3, 0)), (Some(0b101), Some(0b101)), "control: outside the 2x1 area, minterm {mt:#x}");
    }
    g.run(&[gfx(SET_APEN, &[a(1, drp), d(0, 0b101)]), gfx(RECT_FILL, &[a(1, drp), d(0, 0), d(1, 0), d(2, 7), d(3, 3)])]);
    blt(&mut g, 0xC0, 0b001, 4);
    assert_eq!(g.px(dbm, 4, 0), Some(0b101 & !1 | (0b011 & 1)), "mask 1: only plane 0 comes across");
    blt(&mut g, 0xC0, 0xFF, 7);
    assert_eq!((g.px(dbm, 7, 0), g.result(0)), (Some(0b011), 3), "the 2-wide blit at x=7 clips to one pixel; BltBitMap returns the planes");
}

#[test]
fn clipblit_between_rastports_and_within_one_bitmap_overlapping() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(16, 2, 3);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 1)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 3), d(3, 1)]), gfx(SET_APEN, &[a(1, rp), d(0, 2)]),
        gfx(RECT_FILL, &[a(1, rp), d(0, 4), d(1, 0), d(2, 7), d(3, 1)])]);
    g.run(&[gfx(CLIP_BLIT, &[a(0, rp), d(0, 0), d(1, 0), a(1, rp), d(2, 2), d(3, 0), d(4, 8), d(5, 2), d(6, 0xC0)])]);
    assert_eq!(g.rows(bm, 0, 0, 10, 1)[0], [1, 1, 1, 1, 1, 1, 2, 2, 2, 2], "an overlapping blit shifted right by 2 without smearing");
    assert_eq!(g.px(bm, 1, 1), Some(1), "control: untouched left of the destination");
}

#[test]
fn blt_template_draws_ink_in_the_pen_and_the_paper_only_in_jam2() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(16, 4, 3);
    let tmpl = g.words(&[0b1010_0000_0000_0000, 0b1100_0000_0000_0000]);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 6)]), gfx(SET_BPEN, &[a(1, rp), d(0, 3)]), gfx(SET_DRMD, &[a(1, rp), d(0, 0)]),
        gfx(BLT_TEMPLATE, &[a(0, tmpl), d(0, 0), d(1, 2), a(1, rp), d(2, 1), d(3, 0), d(4, 4), d(5, 2)])]);
    assert_eq!(g.rows(bm, 1, 0, 4, 2), vec![vec![6, 0, 6, 0], vec![6, 6, 0, 0]], "JAM1: ink only");
    g.run(&[gfx(SET_DRMD, &[a(1, rp), d(0, 1)]), gfx(BLT_TEMPLATE, &[a(0, tmpl), d(0, 0), d(1, 2), a(1, rp), d(2, 1), d(3, 2), d(4, 4), d(5, 2)])]);
    assert_eq!(g.rows(bm, 1, 2, 4, 1)[0], [6, 3, 6, 3], "JAM2: the paper in the background pen");
    g.run(&[gfx(SET_DRMD, &[a(1, rp), d(0, 4)]), gfx(BLT_TEMPLATE, &[a(0, tmpl), d(0, 0), d(1, 2), a(1, rp), d(2, 8), d(3, 0), d(4, 4), d(5, 1)])]);
    assert_eq!(g.rows(bm, 8, 0, 4, 1)[0], [0, 6, 0, 6], "JAM1|INVERSVID draws where the template is 0");
}

#[test]
fn blt_mask_bitmap_rastport_only_changes_where_the_mask_is_set() {
    let mut g = G::new();
    let (sbm, srp) = g.bitmap_rp(16, 2, 3);
    let (dbm, drp) = g.bitmap_rp(16, 2, 3);
    g.run(&[gfx(SET_APEN, &[a(1, srp), d(0, 7)]), gfx(RECT_FILL, &[a(1, srp), d(0, 0), d(1, 0), d(2, 15), d(3, 1)])]);
    let mask = g.words(&[0b1100_1100_0000_0000, 0b0011_0011_0000_0000]);
    g.run(&[gfx(BLT_MASK_BITMAP_RASTPORT, &[a(0, sbm), d(0, 0), d(1, 0), a(1, drp), d(2, 0), d(3, 0), d(4, 8), d(5, 2), d(6, 0xC0), a(2, mask)])]);
    assert_eq!(g.rows(dbm, 0, 0, 8, 1)[0], [7, 7, 0, 0, 7, 7, 0, 0]);
    assert_eq!(g.rows(dbm, 0, 1, 8, 1)[0], [0, 0, 7, 7, 0, 0, 7, 7], "row 1 uses mask row 1");
}

#[test]
fn set_rast_and_clear_eol_and_the_write_mask() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(24, 16, 3);
    g.run(&[gfx(SET_RAST, &[a(1, rp), d(0, 4)])]);
    assert!(g.rows(bm, 0, 0, 24, 16).iter().flatten().all(|&p| p == 4));
    g.run(&[gfx(SET_BPEN, &[a(1, rp), d(0, 1)]), gfx(MOVE, &[a(1, rp), d(0, 8), d(1, 10)]), gfx(CLEAR_EOL, &[a(1, rp)])]);
    assert_eq!(g.rows(bm, 6, 3, 4, 1)[0], [4, 4, 4, 4], "control: above the text line");
    assert_eq!(g.rows(bm, 6, 4, 4, 1)[0], [4, 4, 1, 1], "from x=8 to the right edge, top = cp_y - baseline(6) = 4");
    assert_eq!(g.px(bm, 23, 11), Some(1));
    assert_eq!(g.px(bm, 8, 12), Some(4), "one text line (8 rows) high: rows 4..=11");
    g.a.mem.write_u8(rp + 24, 0b001).unwrap(); // Mask: only plane 0 writable
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 7)]), gfx(WRITE_PIXEL, &[a(1, rp), d(0, 0), d(1, 0)])]);
    assert_eq!(g.px(bm, 0, 0), Some(0b101), "pen 4 with only bit 0 replaced by pen 7's bit 0");
}

#[test]
fn unusable_bitmaps_and_depths_are_named_refusals_never_faults() {
    let mut g = G::new();
    let rp = g.zeroed(100);
    let junk = g.zeroed(40);
    g.run(&[gfx(INIT_RASTPORT, &[a(1, rp)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 3), d(3, 3)]), gfx(ALLOC_BITMAP, &[d(0, 8), d(1, 8), d(2, 12), d(3, 0), a(0, 0)]),
        gfx(BLT_BITMAP, &[a(0, junk), d(0, 0), d(1, 0), a(1, junk), d(2, 0), d(3, 0), d(4, 1), d(5, 1), d(6, 0xC0), d(7, 0xFF), a(2, 0)])]);
    let r = &g.a.refused;
    assert!(r[0].contains("neither a Layer nor a BitMap"), "{r:?}");
    assert!(r[1].contains("AllocBitMap") && r[1].contains("12"), "{r:?}");
    assert!(r[2].contains("not a usable bitmap"), "{r:?}");
    assert_eq!(g.result(2), 0, "a refused AllocBitMap returns NULL");
}
