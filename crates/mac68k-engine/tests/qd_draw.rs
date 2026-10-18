//! QuickDraw drawing as a program sees it: rectangles, pen state, transfer modes, patterns, ovals, lines, clipping.
mod common;
use common::*;

const R1: u32 = 0x3100;
const R2: u32 = 0x3108;
const PAT: u32 = 0x3110;

fn rig_with_rect(t: i16, l: i16, b: i16, r: i16) -> Rig {
    let mut g = Rig::new();
    set_rect(&mut g.m, R1, t, l, b, r);
    g
}

#[test]
fn paint_rect_blackens_exactly_the_rectangle() {
    let mut g = rig_with_rect(10, 20, 30, 60);
    g.run(Prog::new().paint_rect(R1).exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 20 * 40, "{}", g.pic(15, 5, 65, 35));
    assert!(g.black(20, 10) && g.black(59, 29), "first and last pixel");
    assert!(!g.black(19, 10) && !g.black(60, 29) && !g.black(20, 9) && !g.black(59, 30), "half-open: nothing outside");
}

#[test]
fn nothing_is_drawn_without_a_paint_call_negative_control() {
    let mut g = rig_with_rect(10, 20, 30, 60);
    g.run(Prog::new().exit());
    assert_eq!(g.count_black(), 0);
}

#[test]
fn frame_rect_uses_the_pen_size_and_leaves_the_inside_alone() {
    let mut g = rig_with_rect(10, 20, 30, 60);
    g.run(Prog::new().pen_size(3, 2).frame_rect(R1).exit());
    g.no_refusals();
    // 40x20 outer; inner (inset 3 wide, 2 tall) = 34x16
    assert_eq!(g.count_black(), 40 * 20 - 34 * 16, "{}", g.pic(15, 5, 65, 35));
    assert!(g.black(20, 10) && g.black(22, 11) && g.black(30, 11) && !g.black(23, 12) && !g.black(40, 20));
    // PenNormal restores a 1x1 pen
    let mut g = rig_with_rect(10, 20, 30, 60);
    g.run(Prog::new().pen_size(3, 2).pen_normal().frame_rect(R1).exit());
    assert_eq!(g.count_black(), 40 * 20 - 38 * 18);
}

#[test]
fn erase_uses_the_background_pattern_and_invert_twice_is_the_identity() {
    let mut g = rig_with_rect(10, 20, 30, 60);
    g.run(Prog::new().paint_rect(R1).erase_rect(R1).exit());
    assert_eq!(g.count_black(), 0, "EraseRect = bkPat (white)");
    let mut g = rig_with_rect(10, 20, 30, 60);
    set_rect(&mut g.m, R2, 15, 30, 25, 40);
    g.run(Prog::new().paint_rect(R2).invert_rect(R1).exit());
    assert_eq!(g.count_black(), 20 * 40 - 10 * 10, "inverting a rect over a black hole leaves the hole white and the rest black");
    let mut g = rig_with_rect(10, 20, 30, 60);
    set_rect(&mut g.m, R2, 15, 30, 25, 40);
    g.run(Prog::new().paint_rect(R2).invert_rect(R1).invert_rect(R1).exit());
    assert_eq!(g.count_black(), 10 * 10, "second InvertRect restores it");
}

#[test]
fn fill_rect_with_the_gray_pattern_is_aligned_to_the_screen_not_the_window() {
    let mut g = rig_with_rect(0, 0, 16, 16);
    put(&mut g.m, PAT, &[0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55]);
    g.run(Prog::new().fill_rect(R1, PAT).exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 128, "a checkerboard is half black");
    for (x, y) in [(0, 0), (1, 0), (0, 1), (5, 9), (6, 3)] {
        // screen alignment: the window's content starts at global (left 50, top 100)
        let (gx, gy) = (x + 50, y + 100);
        let expect = [0xAAu8, 0x55][(gy & 1) as usize] >> (7 - (gx & 7)) & 1 == 1;
        assert_eq!(g.black(x, y), expect, "pixel {x},{y}\n{}", g.pic(0, 0, 16, 8));
    }
    // the black pattern from QDGlobals (thePort-16) fills solid; the white one (thePort-8) draws nothing
    let mut g = rig_with_rect(0, 0, 16, 16);
    g.run(Prog::new().fill_rect(R1, GLOBALS - 16).exit());
    assert_eq!(g.count_black(), 256);
    let mut g = rig_with_rect(0, 0, 16, 16);
    g.run(Prog::new().paint_rect(R1).fill_rect(R1, GLOBALS - 8).exit());
    assert_eq!(g.count_black(), 0, "FillRect is patCopy: the white pattern erases");
}

#[test]
fn transfer_modes_on_a_half_black_background() {
    // background: left half of R1 black. then PaintRect over all of R1 with each pen mode (solid black pen pattern).
    let run = |mode: i16, pat: [u8; 8]| {
        let mut g = rig_with_rect(0, 0, 8, 16);
        set_rect(&mut g.m, R2, 0, 0, 8, 8);
        put(&mut g.m, PAT, &pat);
        g.run(Prog::new().paint_rect(R2).pen_pat(PAT).pen_mode(mode).paint_rect(R1).exit());
        g.no_refusals();
        (g.count_in(0, 0, 8, 8), g.count_in(8, 0, 16, 8))
    };
    let black = [0xFF; 8];
    let white = [0; 8];
    assert_eq!(run(8, black), (64, 64), "patCopy black: everything black");
    assert_eq!(run(8, white), (0, 0), "patCopy white: everything white");
    assert_eq!(run(9, white), (64, 0), "patOr white: the black half stays, the white half stays white");
    assert_eq!(run(10, black), (0, 64), "patXor black: the black half flips white, the white half flips black");
    assert_eq!(run(11, black), (0, 0), "patBic black: black pattern pixels become white");
    assert_eq!(run(11, white), (64, 0), "patBic white: nothing changes");
    assert_eq!(run(12, white), (64, 64), "notPatCopy of white = black");
    assert_eq!(run(13, white), (64, 64), "notPatOr white: inverted pattern is black everywhere");
    assert_eq!(run(14, black), (64, 0), "notPatXor black: xor with white = unchanged");
    assert_eq!(run(15, white), (0, 0), "notPatBic white: dest AND pattern clears wherever the pattern is white");
    assert_eq!(run(15, black), (64, 0), "notPatBic black: dest AND pattern leaves everything as it was");
}

#[test]
fn ovals_and_round_rects_are_inscribed_symmetric_and_framed_by_the_pen() {
    let mut g = rig_with_rect(10, 10, 50, 90);
    g.run(Prog::new().rect_op(0xA8B8, R1).exit()); // PaintOval
    g.no_refusals();
    let bounds_ok = g.black(50, 10) && g.black(50, 49) && g.black(10, 30) && g.black(89, 30);
    assert!(bounds_ok, "touches the middle of every side\n{}", g.pic(8, 8, 92, 52));
    assert!(!g.black(10, 10) && !g.black(89, 49), "corners stay white");
    for (x, y) in [(15, 14), (30, 20), (44, 45)] {
        assert_eq!(g.black(x, y), g.black(99 - x, y), "left/right symmetry");
        assert_eq!(g.black(x, y), g.black(x, 59 - y), "top/bottom symmetry");
    }
    let area = g.count_black() as f64;
    let ideal = std::f64::consts::PI * 40.0 * 20.0;
    assert!((area - ideal).abs() < ideal * 0.03, "area {area} vs ellipse {ideal}");
    // FrameOval leaves the centre white; FrameRoundRect likewise
    let mut g = rig_with_rect(10, 10, 50, 90);
    g.run(Prog::new().rect_op(0xA8B7, R1).exit());
    assert!(g.black(50, 10) && !g.black(50, 30) && g.black(10, 30), "{}", g.pic(8, 8, 92, 52));
    let mut g = rig_with_rect(10, 10, 50, 90);
    g.run(Prog::new().l(R1).w(16).w(16).trap(0xA8B0).exit());
    g.no_refusals();
    assert!(g.black(50, 10) && g.black(10, 30) && !g.black(10, 10) && !g.black(50, 30), "{}", g.pic(8, 8, 92, 52));
    // EraseOval / InvertOval
    let mut g = rig_with_rect(10, 10, 50, 90);
    g.run(Prog::new().paint_rect(R1).rect_op(0xA8B9, R1).exit());
    assert!(g.black(10, 10) && !g.black(50, 30), "erased oval inside a black rect leaves the corners");
}

#[test]
fn lines_use_the_pen_size_and_move_the_pen_and_a_hidden_pen_only_moves() {
    let mut g = Rig::new();
    g.run(Prog::new().move_to(10, 10).line_to(30, 10).exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 21, "a 1x1 pen from h=10 to h=30 inclusive: 21 pixels\n{}", g.pic(8, 8, 34, 13));
    let pen = g.m.mem.read_u32(g.win + 48).unwrap();
    assert_eq!(pen, (10 << 16) | 30, "pnLoc is (v=10, h=30)");
    let mut g = Rig::new();
    g.run(Prog::new().move_to(10, 10).line_to(40, 40).exit());
    assert_eq!(g.count_black(), 31, "a 45 degree line is one pixel per step");
    assert!(g.black(25, 25) && !g.black(25, 26));
    let mut g = Rig::new();
    g.run(Prog::new().pen_size(2, 3).move_to(10, 10).line_to(10, 20).exit());
    assert_eq!(g.count_black(), 2 * (3 + 10), "a 2x3 pen dragged 10 down covers a 2 x 13 column");
    // HidePen: nothing drawn, pen still moves; ShowPen undoes it
    let mut g = Rig::new();
    g.run(Prog::new().trap(0xA896).move_to(10, 10).line_to(30, 10).exit());
    assert_eq!(g.count_black(), 0);
    assert_eq!(g.m.mem.read_u32(g.win + 48).unwrap(), (10 << 16) | 30);
    let mut g = Rig::new();
    g.run(Prog::new().trap(0xA896).trap(0xA897).move_to(10, 10).line_to(30, 10).exit());
    assert_eq!(g.count_black(), 21);
    // Move/Line are relative
    let mut g = Rig::new();
    g.run(Prog::new().move_to(5, 5).w(10).w(0).trap(0xA894).w(0).w(4).trap(0xA892).exit());
    assert_eq!(g.m.mem.read_u32(g.win + 48).unwrap(), (9 << 16) | 15);
    assert_eq!(g.count_black(), 5);
}

#[test]
fn clip_rect_limits_drawing_and_wide_open_is_the_default() {
    let mut g = rig_with_rect(0, 0, 40, 40);
    set_rect(&mut g.m, R2, 10, 10, 20, 20);
    g.run(Prog::new().l(R2).trap(0xA87B).paint_rect(R1).exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 100, "only the clip survives");
    assert!(g.black(10, 10) && !g.black(9, 10) && !g.black(20, 20));
    let mut g = rig_with_rect(0, 0, 40, 40);
    g.run(Prog::new().paint_rect(R1).exit());
    assert_eq!(g.count_black(), 1600, "negative control: no ClipRect, no clipping");
}

#[test]
fn drawing_with_no_port_or_before_initgraf_is_a_named_refusal() {
    let mut g = rig_with_rect(0, 0, 10, 10);
    g.m.mem.write_u32(common::GLOBALS, 0).unwrap(); // thePort := NIL
    g.m.start(&Prog::new().paint_rect(R1).exit().0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    assert!(g.m.refused.iter().any(|r| r.contains("PaintRect") && r.contains("thePort is NIL")), "{:?}", g.m.refused);
}

#[test]
fn get_ccursor_answers_nil_and_says_so() {
    let mut g = Rig::new();
    g.m.mem.write_u32(SCRATCH, 0xDEAD_BEEF).unwrap();
    g.run(Prog::new().clr_l().w(128).trap(0xAA1B).pop_l(SCRATCH).exit());
    assert_eq!(g.m.mem.read_u32(SCRATCH).unwrap(), 0, "NIL handle: the program falls back to its B&W cursor");
    assert!(g.m.log.iter().any(|l| l.contains("GetCCursor 128 -> NIL") && l.contains("not implemented")));
    g.no_refusals();
}
