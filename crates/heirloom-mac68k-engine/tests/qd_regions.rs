//! Regions as guest records behind real handles: the packed format, the set operations, hit tests, clip regions,
//! OpenRgn/CloseRgn and handle hygiene.
mod common;
use common::*;
use heirloom_mac68k_engine::qd::regions::{pack, unpack};
use heirloom_raster::{Rect, Region};

const A: u32 = 0x3200;
const B: u32 = 0x3204;
const C: u32 = 0x3208;

fn set_rgn(p: &mut Prog, at: u32, l: i16, t: i16, r: i16, b: i16) {
    p.rgn_arg(at).w(l).w(t).w(r).w(b).trap(0xA8DE);
}
fn three(p: &mut Prog, trap: u16) { p.rgn_arg(A).rgn_arg(B).rgn_arg(C).trap(trap); }
fn bytes(g: &Rig, at: u32) -> Vec<u8> {
    let h = g.m.mem.read_u32(at).unwrap();
    let b = g.m.mm.block(h).unwrap();
    g.m.mem.bytes(g.m.mem.span(b.ptr, b.size).unwrap()).unwrap().to_vec()
}
fn words(b: &[u8]) -> Vec<u16> { b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect() }
fn two_squares() -> Prog {
    let mut p = Prog::new();
    p.new_rgn(A).new_rgn(B).new_rgn(C);
    set_rgn(&mut p, A, 0, 0, 4, 4); // left, top, right, bottom
    set_rgn(&mut p, B, 2, 2, 6, 6);
    p
}

#[test]
fn a_fresh_region_is_the_ten_byte_empty_record_and_a_rect_region_is_ten_bytes() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.new_rgn(A).new_rgn(B);
    set_rgn(&mut p, B, 5, 3, 15, 9);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(words(&bytes(&g, A)), vec![10, 0, 0, 0, 0], "rgnSize 10, bbox all zero");
    assert_eq!(words(&bytes(&g, B)), vec![10, 3, 5, 9, 15], "rgnSize 10, bbox = top,left,bottom,right");
}

#[test]
fn union_of_overlapping_squares_packs_into_the_documented_scan_line_format() {
    let mut g = Rig::new();
    let mut p = two_squares();
    three(&mut p, 0xA8E5); // UnionRgn
    g.run(p.exit());
    g.no_refusals();
    // size 44; bbox 0,0,6,6; then per changing line: v, inversion points, $7FFF; final $7FFF
    let want = vec![44u16, 0, 0, 6, 6, 0, 0, 4, 0x7FFF, 2, 4, 6, 0x7FFF, 4, 0, 2, 0x7FFF, 6, 2, 6, 0x7FFF, 0x7FFF];
    assert_eq!(words(&bytes(&g, C)), want);
    // and it reads back as the same pixel set
    let r = unpack(&bytes(&g, C)).unwrap();
    let mut expect = Region::from_rect(Rect::new(0, 0, 4, 4));
    expect.or_rect(Rect::new(2, 2, 6, 6));
    assert_eq!(r, expect);
    // negative control: a corrupted record is refused, not guessed at
    let mut bad = bytes(&g, C);
    bad.truncate(bad.len() - 2);
    assert!(unpack(&bad).is_err());
    let mut bad = bytes(&g, C);
    bad[1] = 46;
    assert!(unpack(&bad).is_err(), "rgnSize that disagrees with the record");
}

#[test]
fn pack_unpack_round_trips_awkward_shapes() {
    for g in [
        Region::new(),
        Region::from_rect(Rect::new(-5, -5, 5, 5)),
        { let mut g = Region::from_rect(Rect::new(0, 0, 30, 10)); g.clear_rect(Rect::new(10, 3, 20, 7)); g },
        { let mut g = Region::from_rect(Rect::new(0, 0, 3, 3)); g.or_rect(Rect::new(10, 10, 13, 13)); g },
    ] {
        assert_eq!(unpack(&pack(&g)).unwrap(), g);
    }
}

#[test]
fn sect_diff_xor_and_the_hit_tests() {
    let mut g = Rig::new();
    let mut p = two_squares();
    three(&mut p, 0xA8E4); // SectRgn -> C
    p.clr_w().l((3 << 16) | 3).rgn_arg(C).trap(0xA8E8).pop_w(SCRATCH); // PtInRgn((v3,h3), C)
    p.clr_w().l((1 << 16) | 1).rgn_arg(C).trap(0xA8E8).pop_w(SCRATCH + 2); // (1,1): in A only
    p.clr_w().rgn_arg(C).trap(0xA8E2).pop_w(SCRATCH + 4); // EmptyRgn(C)
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(words(&bytes(&g, C)), vec![10, 2, 2, 4, 4], "the intersection of two squares is a rect region: 10 bytes");
    assert_eq!(g.m.mem.read_u16(SCRATCH).unwrap(), 0x0100, "(3,3) is in both");
    assert_eq!(g.m.mem.read_u16(SCRATCH + 2).unwrap(), 0, "(1,1) is not in the intersection (negative control)");
    assert_eq!(g.m.mem.read_u16(SCRATCH + 4).unwrap(), 0, "not empty");

    let mut g = Rig::new();
    let mut p = two_squares();
    three(&mut p, 0xA8E6); // DiffRgn: A - B
    p.clr_w().l((3 << 16) | 3).rgn_arg(C).trap(0xA8E8).pop_w(SCRATCH);
    p.clr_w().l((1 << 16) | 1).rgn_arg(C).trap(0xA8E8).pop_w(SCRATCH + 2);
    g.run(p.exit());
    assert_eq!((g.m.mem.read_u16(SCRATCH).unwrap(), g.m.mem.read_u16(SCRATCH + 2).unwrap()), (0, 0x0100));

    let mut g = Rig::new();
    let mut p = two_squares();
    three(&mut p, 0xA8E7); // XorRgn
    p.clr_w().l((3 << 16) | 3).rgn_arg(C).trap(0xA8E8).pop_w(SCRATCH);
    p.clr_w().l((5 << 16) | 5).rgn_arg(C).trap(0xA8E8).pop_w(SCRATCH + 2);
    g.run(p.exit());
    assert_eq!((g.m.mem.read_u16(SCRATCH).unwrap(), g.m.mem.read_u16(SCRATCH + 2).unwrap()), (0, 0x0100), "the overlap is excluded, B's own corner is in");
}

#[test]
fn disjoint_sect_is_empty_equal_and_rect_in_rgn_work_and_offset_moves_the_box() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.new_rgn(A).new_rgn(B).new_rgn(C);
    set_rgn(&mut p, A, 0, 0, 4, 4);
    set_rgn(&mut p, B, 10, 10, 14, 14);
    three(&mut p, 0xA8E4);
    p.clr_w().rgn_arg(C).trap(0xA8E2).pop_w(SCRATCH); // EmptyRgn -> TRUE
    p.clr_w().rgn_arg(A).rgn_arg(B).trap(0xA8E3).pop_w(SCRATCH + 2); // EqualRgn(A,B) -> FALSE
    p.rgn_arg(B).w(-10).w(-10).trap(0xA8E0); // OffsetRgn(B, -10, -10) -> same as A
    p.clr_w().rgn_arg(A).rgn_arg(B).trap(0xA8E3).pop_w(SCRATCH + 4); // EqualRgn -> TRUE
    set_rect(&mut g.m, 0x3300, 3, 3, 9, 9);
    p.clr_w().l(0x3300).rgn_arg(A).trap(0xA8E9).pop_w(SCRATCH + 6); // RectInRgn(r, A) -> TRUE (overlaps 3..4)
    set_rect(&mut g.m, 0x3308, 4, 4, 9, 9);
    p.clr_w().l(0x3308).rgn_arg(A).trap(0xA8E9).pop_w(SCRATCH + 8); // does not overlap
    g.run(p.exit());
    g.no_refusals();
    let rd = |o| g.m.mem.read_u16(SCRATCH + o).unwrap();
    assert_eq!((rd(0), rd(2), rd(4), rd(6), rd(8)), (0x0100, 0, 0x0100, 0x0100, 0));
    assert_eq!(words(&bytes(&g, C)), vec![10, 0, 0, 0, 0], "an empty result is the all-zero 10-byte record");
}

#[test]
fn paint_rgn_draws_the_shape_and_clip_to_a_region_clips_to_the_shape() {
    let mut g = Rig::new();
    let mut p = two_squares();
    three(&mut p, 0xA8E5);
    p.rgn_arg(C).trap(0xA8D3); // PaintRgn
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 16 + 16 - 4, "{}", g.pic(0, 0, 8, 8));
    assert!(g.black(1, 1) && g.black(5, 5) && !g.black(5, 1) && !g.black(1, 5));
    // SetClip(L-shaped region) then PaintRect(everything): only the shape survives
    let mut g = Rig::new();
    let mut p = two_squares();
    three(&mut p, 0xA8E5);
    p.rgn_arg(C).trap(0xA879); // SetClip
    set_rect(&mut g.m, 0x3300, 0, 0, 20, 20);
    p.paint_rect(0x3300);
    p.rgn_arg(A).trap(0xA87A); // GetClip(A): A now holds a copy of the clip
    p.clr_w().rgn_arg(A).rgn_arg(C).trap(0xA8E3).pop_w(SCRATCH);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 28);
    assert_eq!(g.m.mem.read_u16(SCRATCH).unwrap(), 0x0100, "GetClip returned what SetClip stored");
}

#[test]
fn open_rgn_close_rgn_records_framed_shapes_and_draws_nothing() {
    let mut g = Rig::new();
    set_rect(&mut g.m, 0x3300, 10, 20, 30, 60);
    let mut p = Prog::new();
    p.new_rgn(A).trap(0xA8DA); // OpenRgn
    p.frame_rect(0x3300);
    p.rgn_arg(A).trap(0xA8DB); // CloseRgn(A)
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 0, "the pen is hidden while a region is open");
    assert_eq!(words(&bytes(&g, A)), vec![10, 10, 20, 30, 60], "a framed rect makes a rect region");
    assert_eq!(g.m.mem.read_u16(g.win + 66).unwrap(), 0, "CloseRgn showed the pen again (pnVis back to 0)");
    assert_eq!(g.m.mem.read_u32(g.win + 96).unwrap(), 0, "rgnSave cleared");
    // a line inside OpenRgn is refused by name (polygon outlines are not recorded), not silently dropped
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.new_rgn(A).trap(0xA8DA).move_to(0, 0).line_to(5, 5).rgn_arg(A).trap(0xA8DB).exit();
    g.m.start(&p.0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    assert!(g.m.refused.iter().any(|r| r.contains("LineTo while a region is open")), "{:?}", g.m.refused);
}

#[test]
fn dispose_rgn_frees_and_a_bad_handle_is_refused_by_name() {
    let mut g = Rig::new();
    let before = g.m.mm.live_handles();
    let mut p = Prog::new();
    p.new_rgn(A);
    p.rgn_arg(A).trap(0xA8D9); // DisposeRgn
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.m.mm.live_handles(), before, "NewRgn + DisposeRgn leaves no handle behind");
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.l(0x1234_5678).trap(0xA8D9);
    p.exit();
    g.m.start(&p.0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    assert!(g.m.refused.iter().any(|r| r.contains("DisposeRgn") && r.contains("not a region handle")), "{:?}", g.m.refused);
}

#[test]
fn disposing_a_window_frees_its_regions() {
    let mut g = Rig::new();
    let with_window = g.m.mm.live_handles();
    g.run(Prog::new().l(g.win).trap(0xA914).exit()); // DisposeWindow
    assert_eq!(g.m.mm.live_handles(), with_window - 6, "visRgn, clipRgn, the WindowRecord's strucRgn, contRgn and updateRgn and its titleHandle are freed with the window");
}
