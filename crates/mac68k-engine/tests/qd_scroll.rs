//! `ScrollRect(dstRect, dh, dv, updateRgn)`: the bits inside dstRect ∩ visRgn ∩ clipRgn move by (dh, dv), the area they vacate is
//! erased to the background pattern and returned in `updateRgn` (QuickDraw Bitmaps.a ScrollRect: srcRgn - dstRgn).
mod common;
use common::*;

const RECT: u32 = 0x3200;
const RGN: u32 = 0x3210;

fn scroll(p: &mut Prog, dh: i16, dv: i16) { p.l(RECT).w(dh).w(dv).rgn_arg(RGN).trap(0xA8EF); }
fn update(g: &Rig) -> retro_raster::Region { g.m.read_region(g.m.mem.read_u32(RGN).unwrap()).unwrap() }

fn rig() -> Rig {
    let mut g = Rig::new();
    set_rect(&mut g.m, RECT, 0, 0, 40, 40);
    g
}

#[test]
fn the_bits_move_the_vacated_area_is_erased_and_returned_in_update_rgn() {
    let mut g = rig();
    let mut p = Prog::new();
    set_rect(&mut g.m, 0x3220, 10, 10, 20, 20);
    set_rect(&mut g.m, 0x3230, 30, 30, 40, 40);
    set_rect(&mut g.m, 0x3240, 50, 50, 52, 52);
    p.new_rgn(RGN).paint_rect(0x3220).paint_rect(0x3230).paint_rect(0x3240);
    scroll(&mut p, 5, 3);
    g.run(p.exit());
    g.no_refusals();
    assert!(g.black(15, 13) && g.black(24, 22), "the square now covers x 15..25, y 13..23\n{}", g.pic(0, 0, 48, 48));
    assert!(!g.black(10, 10) && !g.black(14, 12), "its old top-left corner is erased");
    assert!(g.black(50, 50), "pixels outside dstRect are not touched");
    assert!(!g.black(43, 41) && !g.black(44, 42), "bits scrolled past dstRect's edge are lost, not drawn outside it\n{}", g.pic(30, 30, 50, 50));
    assert!(g.black(39, 39) && g.black(35, 33), "the part of the corner square still inside the rectangle");
    let u = update(&g);
    assert!(u.contains(4, 30) && u.contains(30, 2) && u.contains(0, 0), "the vacated left and top strips");
    assert!(!u.contains(5, 30) && !u.contains(30, 3) && !u.contains(41, 41), "nothing else");
}

#[test]
fn nothing_scrolls_when_the_pen_is_hidden_or_the_offset_is_zero_and_update_rgn_comes_back_empty() {
    for hide in [false, true] {
        let mut g = rig();
        set_rect(&mut g.m, 0x3220, 10, 10, 20, 20);
        let mut p = Prog::new();
        p.new_rgn(RGN).rgn_arg(RGN).w(1).w(1).w(9).w(9).trap(0xA8DE);
        p.paint_rect(0x3220);
        if hide { p.trap(0xA896); }
        scroll(&mut p, if hide { 5 } else { 0 }, if hide { 3 } else { 0 });
        g.run(p.exit());
        g.no_refusals();
        assert!(g.black(10, 10) && g.black(19, 19) && !g.black(20, 20) && !g.black(14, 12 + 20), "unchanged (hidden pen: {hide})");
        assert!(update(&g).is_empty(), "updateRgn was set empty (hidden pen: {hide})");
    }
}

#[test]
fn only_the_part_inside_the_clip_region_scrolls() {
    let mut g = rig();
    set_rect(&mut g.m, 0x3220, 0, 0, 40, 40);
    set_rect(&mut g.m, 0x3230, 0, 0, 20, 40); // clip: the left half only
    let mut p = Prog::new();
    p.new_rgn(RGN).l(0x3230).trap(0xA87B).paint_rect(0x3220); // ClipRect, then fill the whole rectangle black
    scroll(&mut p, 10, 0);
    g.run(p.exit());
    g.no_refusals();
    assert!(!g.black(5, 5) && g.black(15, 5), "inside the clip the black moved right by 10 and left a white strip\n{}", g.pic(0, 0, 40, 8));
    assert!(update(&g).contains(5, 5) && !update(&g).contains(25, 5), "update = srcRgn - dstRgn, srcRgn being rect ∩ clip");
}
