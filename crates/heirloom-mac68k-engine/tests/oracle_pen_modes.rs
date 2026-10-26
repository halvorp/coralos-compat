//! Pen-mode behaviour that the systemless oracle gets wrong (see oracle_systemless.rs DIVERGENCES), pinned to QuickDraw's
//! own source: DrawLine.a draws modes 8..15 (`AND #-8 / CMP #8`), the invert bit of the mode (`BCLR #2,D2 / NOT.L D7`) inverts the
//! PATTERN, and a line is one slab per scanline, so no pixel is touched twice.
mod common;
use common::*;

fn draw(mode: i16, bg_black: bool, twice: bool) -> Rig { draw_with(mode, bg_black, twice, None) }

fn draw_with(mode: i16, bg_black: bool, twice: bool, pat: Option<[u8; 8]>) -> Rig {
    let mut g = Rig::new();
    if let Some(p) = pat { put(&mut g.m, 0x3200, &p); }
    set_rect(&mut g.m, 0x3100, 0, 0, 40, 40);
    let mut p = Prog::new();
    if bg_black { p.paint_rect(0x3100); }
    if pat.is_some() { p.pen_pat(0x3200); }
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

/// With the black pen the inverted pattern is all zero bits, so modes 13..15 are no-ops there and nothing can fail. A checker pen
/// (P and its complement are disjoint, F = the line's footprint) makes the truth tables observable:
///   white paper: notPatCopy / notPatOr / notPatXor all draw F & ~P (= B); notPatBic = paper & P = white.
///   black paper: notPatOr stays black; notPatXor and notPatBic both leave paper minus B.
#[test]
fn not_pat_or_xor_bic_follow_their_truth_tables_with_a_checker_pen() {
    let chk = Some([0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55]);
    let n = |mode, black| draw_with(mode, black, false, chk).count_black();
    let (a, b) = (n(8, false), n(12, false));
    assert!(a > 20 && b > 20, "both halves of the checker are drawn (P: {a}, ~P: {b})");
    assert_eq!(a + b, draw(8, false, false).count_black(), "P and ~P partition the line's footprint");
    assert_eq!(n(13, false), b, "notPatOr on white = F & ~P");
    assert_eq!(n(14, false), b, "notPatXor on white = F & ~P");
    assert_eq!(n(15, false), 0, "notPatBic on white clears nothing");
    assert_eq!(n(13, true), 40 * 40, "notPatOr on black changes nothing");
    assert_eq!(n(14, true), 40 * 40 - b, "notPatXor on black flips F & ~P to white");
    assert_eq!(n(15, true), 40 * 40 - b, "notPatBic on black clears F & ~P");
    assert_ne!(n(14, true), n(13, true), "negative control: a dropped mode would leave the paper alone");
}

#[test]
fn not_pat_xor_and_not_pat_bic_with_the_black_pen_change_nothing_on_black_paper() {
    for mode in [13, 14, 15] {
        assert_eq!(draw(mode, true, false).count_black(), 40 * 40, "mode {mode}: the inverted black pattern is all zero bits, so black paper stays black");
    }
    assert!(draw(12, true, false).count_black() < 40 * 40, "negative control: notPatCopy does change black paper");
}
