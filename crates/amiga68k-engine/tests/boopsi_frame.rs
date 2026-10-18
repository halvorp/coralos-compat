//! frameiclass drawing through DrawImageState (AROS frameiclass.c draw_frameiclass / DrawFrame): the bevel pens, the selected and recessed
//! swaps, the ridge's inner bevel, EdgesOnly. Pens of the engine screen: SHINEPEN white (2), SHADOWPEN the border colour (4), FILLPEN blue (3),
//! BACKGROUNDPEN grey (0).
mod gfxcommon;
use gfxcommon::*;

const NEW_OBJECT_A: i32 = -636;
const DRAW_IMAGE_STATE: i32 = -618;
const IA: u32 = TAG_USER + 0x20000;
const SHINE: u8 = 2;
const SHADOW: u8 = retro_theme::amiga::DRI_PENS[retro_theme::amiga::SHADOWPEN] as u8; // 4 (AROS: 1)
const FILL: u8 = 3;
const BG: u8 = 0;

/// Draw a frameiclass object made from `tags` (IA_ tag pairs) at (1,1) in `state` on a 20x14 bitmap; returns the bitmap rows.
fn drawn(tags: &[(u32, u32)], state: u32) -> Vec<Vec<u8>> {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(20, 14, 8);
    let cn = g.s("frameiclass");
    let t = g.tags(tags);
    g.run(&[intu(NEW_OBJECT_A, &[a(0, 0), a(1, cn), a(2, t)])]);
    let im = g.result(0);
    assert_ne!(im, 0);
    g.run(&[gfx(SET_RAST, &[a(1, rp), d(0, 9)]), intu(DRAW_IMAGE_STATE, &[a(0, rp), a(1, im), d(0, 1), d(1, 1), d(2, state), a(2, 0)])]);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
    g.rows(bm, 0, 0, 20, 14)
}

const SET_RAST: i32 = -234;
const W_H: [(u32, u32); 2] = [(IA + 3, 10), (IA + 4, 8)];

#[test]
fn a_button_frame_has_shine_on_top_and_left_shadow_on_bottom_and_right_and_a_background_inside() {
    let r = drawn(&[W_H[0], W_H[1], (IA + 0x1b, 1)], 0);
    assert_eq!(r[1][1..10], [SHINE; 9], "top edge");
    assert_eq!(r[1][10], SHADOW, "the right edge starts at the top-right corner (DrawFrame draws it from (left+w, top)): that pixel is shadow");
    assert_eq!((0..8).map(|y| r[1 + y][1]).collect::<Vec<_>>(), [SHINE; 8], "left edge");
    assert_eq!(r[8][2..11], [SHADOW; 9], "bottom edge (from x+1)");
    assert_eq!((1..8).map(|y| r[1 + y][10]).collect::<Vec<_>>(), [SHADOW; 7], "right edge");
    assert_eq!(r[4][5], BG, "the inside is the background pen");
    assert_eq!(r[0][0], 9, "control: nothing outside the 10x8 box was touched (the pre-fill)");
}

#[test]
fn selected_swaps_the_bevel_pens_and_fills_with_the_fill_pen_and_recessed_swaps_them_too() {
    let sel = drawn(&[W_H[0], W_H[1], (IA + 0x1b, 1)], 1);
    assert_eq!((sel[1][3], sel[8][3], sel[4][5]), (SHADOW, SHINE, FILL), "selected: top is the shadow pen, bottom the shine pen, the inside the fill pen");
    let rec = drawn(&[W_H[0], W_H[1], (IA + 0x1b, 1), (IA + 0x15, 1)], 0);
    assert_eq!((rec[1][3], rec[8][3], rec[4][5]), (SHADOW, SHINE, BG), "recessed swaps the pens, the inside stays the background");
    let both = drawn(&[W_H[0], W_H[1], (IA + 0x1b, 1), (IA + 0x15, 1)], 1);
    assert_eq!((both[1][3], both[8][3]), (SHINE, SHADOW), "selected and recessed cancel out");
}

#[test]
fn a_ridge_draws_an_inner_bevel_in_the_opposite_pens_and_edges_only_leaves_the_inside_alone() {
    let r = drawn(&[W_H[0], W_H[1], (IA + 0x1b, 2)], 0);
    assert_eq!((r[1][4], r[2][4]), (SHINE, SHADOW), "outer top shine, then the inner bevel's top (in the shadow pen) one row lower");
    assert_eq!((r[8][4], r[7][4]), (SHADOW, SHINE), "outer bottom shadow, inner bottom shine");
    assert_eq!(r[4][5], BG);
    let e = drawn(&[W_H[0], W_H[1], (IA + 0x1b, 1), (IA + 0x17, 1)], 0);
    assert_eq!(e[4][5], 9, "IA_EdgesOnly: the inside keeps what was there");
    assert_eq!(e[1][3], SHINE, "control: the edges are still drawn");
}
