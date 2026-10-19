//! `SetPortBits` on a WINDOW's port: the program points `portBits` at an offscreen BitMap (it draws there, not into the window) and
//! later back at a saved copy of the screen's BitMap (it draws into the window again). The idiom of a user item that composes its
//! picture offscreen and `CopyBits`es it (WriteNow's ruler).
mod common;
use common::*;

const OFF: u32 = 0x3400; // BitMap: baseAddr, rowBytes, bounds
const OFF_PIXELS: u32 = 0x3500;
const SAVED: u32 = 0x3440; // a copy of the window's own portBits
const RECT: u32 = 0x3200;

fn rig() -> Rig {
    let mut g = Rig::new();
    put(&mut g.m, OFF, &OFF_PIXELS.to_be_bytes());
    put(&mut g.m, OFF + 4, &8u16.to_be_bytes());
    set_rect(&mut g.m, OFF + 6, 0, 0, 16, 64);
    let bits = g.m.mem.bytes(g.m.mem.span(g.win + 2, 14).unwrap()).unwrap().to_vec();
    put(&mut g.m, SAVED, &bits);
    set_rect(&mut g.m, RECT, 2, 2, 10, 10);
    g
}

fn offscreen_black(g: &Rig) -> usize { g.m.mem.bytes(g.m.mem.span(OFF_PIXELS, 8 * 16).unwrap()).unwrap().iter().map(|b| b.count_ones() as usize).sum() }

#[test]
fn drawing_follows_portbits_to_the_offscreen_bitmap_and_back_to_the_window() {
    let mut g = rig();
    let mut p = Prog::new();
    p.set_port(g.win).l(OFF).trap(0xA875).paint_rect(RECT);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 0, "nothing reached the window while portBits named the offscreen map");
    assert_eq!(offscreen_black(&g), 64, "the 8x8 square is in the offscreen BitMap");
    let mut p = Prog::new();
    p.l(SAVED).trap(0xA875).paint_rect(RECT);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 64, "portBits points at the screen copy again: the square is in the window");
    assert_eq!(offscreen_black(&g), 64, "and the offscreen map kept its square");
}

#[test]
fn a_saved_copy_of_the_screen_bitmap_is_the_window_as_a_copybits_destination() {
    let mut g = rig();
    // draw a square offscreen, then CopyBits it into the window through the SAVED screen copy (not the port's own struct)
    set_rect(&mut g.m, 0x3220, 0, 0, 16, 64);
    set_rect(&mut g.m, 0x3230, 20, 20, 36, 84);
    let mut p = Prog::new();
    p.set_port(g.win).l(OFF).trap(0xA875).paint_rect(RECT).l(SAVED).trap(0xA875);
    p.l(OFF).l(SAVED).l(0x3220).l(0x3230).w(0).clr_l().trap(0xA8EC);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 64, "the offscreen square landed in the window");
    assert!(g.black(22, 22) && g.black(29, 29) && !g.black(21, 21), "at the destination rectangle's offset\n{}", g.pic(18, 18, 34, 34));
}
