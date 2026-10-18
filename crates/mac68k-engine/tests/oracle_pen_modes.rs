//! Pen-mode behaviour that the systemless oracle gets wrong (see oracle_systemless.rs DIVERGENCES), pinned to QuickDraw's
//! own source: DrawLine.a draws modes 8..15 (`AND #-8 / CMP #8`), the invert bit of the mode (`BCLR #2,D2 / NOT.L D7`) inverts the
//! PATTERN, and a line is one slab per scanline, so no pixel is touched twice.
mod common;
use common::*;

fn draw(mode: i16, bg_black: bool, twice: bool) -> Rig {
    let mut g = Rig::new();
    set_rect(&mut g.m, 0x3100, 0, 0, 40, 40);
    let mut p = Prog::new();
    if bg_black { p.paint_rect(0x3100); }
    p.pen_size(3, 2).pen_mode(mode).move_to(4, 4).line_to(36, 30);
    if twice { p.move_to(4, 4).line_to(36, 30); }
    g.run(p.exit());
    g.no_refusals();
    g
}

#[test]
fn not_pat_copy_draws_the_inverted_pattern() {
    let copy = draw(8, false, false);
    assert!(copy.count_black() > 60, "patCopy with the black pen blackens the line");
    assert_eq!(draw(12, false, false).count_black(), 0, "notPatCopy with the black pen draws white: nothing on white paper");
    let on_black = draw(12, true, false);
    assert!(on_black.count_black() < 40 * 40 - 60, "notPatCopy on black paper cuts a white line (modes 12..15 are not patCopy)");
}

#[test]
fn xor_touches_every_pixel_exactly_once() {
    let once = draw(10, false, false);
    assert_eq!(once.count_black(), draw(8, false, false).count_black(), "patXor once on white = the line's footprint: each pixel toggled once");
    assert_eq!(draw(10, false, true).count_black(), 0, "patXor twice restores the paper");
}

#[test]
fn not_pat_xor_and_not_pat_bic_follow_their_truth_tables_on_black_paper() {
    // black pen, inverted = white: notPatXor flips nothing (xor 0), notPatOr changes nothing, notPatBic clears nothing (bic 0)
    for mode in [13, 14, 15] {
        assert_eq!(draw(mode, true, false).count_black(), 40 * 40, "mode {mode}: the inverted black pattern is all zero bits, so black paper stays black");
    }
    assert!(draw(12, true, false).count_black() < 40 * 40, "negative control: notPatCopy does change black paper");
}
