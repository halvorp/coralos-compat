mod gfxcommon;
use gfxcommon::*;
use heirloom_ami68k_engine::graphics::layout::WD_RPORT;
use heirloom_ami68k_engine::tags::TAG_USER;
use heirloom_ami68k_engine::window::LVO_OPEN_WINDOW_TAG_LIST;
use heirloom_present::plane::{moved, Rec};
use heirloom_ui_abi::*;

const WA: u32 = TAG_USER + 99;
const WA_LEFT: u32 = WA + 1;
const WA_TOP: u32 = WA + 2;
const WA_WIDTH: u32 = WA + 3;
const WA_HEIGHT: u32 = WA + 4;
const WA_IDCMP: u32 = WA + 7;
const WA_FLAGS: u32 = WA + 8;
const WFLG_BORDERLESS: u32 = 0x800;

fn open(g: &mut G, idcmp: u32, wflags: u32) -> (u32, u32, UiObjectRef) {
    let tags = g.tags(&[
        (WA_LEFT, 0),
        (WA_TOP, 0),
        (WA_WIDTH, 200),
        (WA_HEIGHT, 100),
        (WA_IDCMP, idcmp),
        (WA_FLAGS, WFLG_BORDERLESS | wflags),
    ]);
    g.run(&[intu(LVO_OPEN_WINDOW_TAG_LIST, &[a(0, 0), a(1, tags)])]);
    let win = g.result(0);
    let wrp = g.r32(win + WD_RPORT);
    (win, wrp, g.a.windows[&win])
}

fn planes(g: &G) -> Vec<Plane> { g.a.ui.calls.iter().filter_map(|c| if let UiCall::PresentPlane(_, p) = c { Some(p.clone()) } else { None }).collect() }

fn draw_offscreen(g: &mut G) -> (u32, u32) {
    let rp_bm = g.bitmap_rp(100, 60, 3);
    draw_into(g, rp_bm)
}

/// A PROGRAM-OWNED BitMap: the struct and its three planes in guest memory (AllocBitMap's are host-backed, Planes[] NULL, so
/// nothing can write them behind the engine's back), on a fresh RastPort.
fn guest_planes_rp(g: &mut G) -> (u32, u32) {
    let (bpr, rows, depth) = (14u32, 60u32, 3u32); // 112 px wide, word-aligned rows
    let bm = g.zeroed(40);
    g.a.mem.write_u16(bm, bpr as u16).unwrap();
    g.a.mem.write_u16(bm + 2, rows as u16).unwrap();
    g.a.mem.write_u8(bm + 5, depth as u8).unwrap();
    for i in 0..depth { let plane = g.zeroed(bpr * rows); g.a.mem.write_u32(bm + 8 + 4 * i, plane).unwrap(); }
    let rp = g.zeroed(100);
    g.run(&[gfx(INIT_RASTPORT, &[a(1, rp)])]);
    g.a.mem.write_u32(rp + 4, bm).unwrap();
    (bm, rp)
}

fn draw_into(g: &mut G, (bm, rp): (u32, u32)) -> (u32, u32) {
    g.set_topaz(rp);
    let copy = g.s("Copy");
    g.run(&[
        gfx(SET_APEN, &[a(1, rp), d(0, 3)]),
        gfx(RECT_FILL, &[a(1, rp), d(0, 4), d(1, 4), d(2, 23), d(3, 13)]),
        gfx(SET_APEN, &[a(1, rp), d(0, 1)]),
        gfx(MOVE, &[a(1, rp), d(0, 32), d(1, 24)]),
        gfx(TEXT, &[a(1, rp), a(0, copy), d(0, 4)]),
    ]);
    (bm, rp)
}

fn source_fill(g: &G, bm: u32) -> DrawOp {
    g.a.gfx
        .surfaces
        .get(bm)
        .expect("offscreen surface")
        .rec
        .iter()
        .find_map(|rec| match rec {
            Rec::Op { op: op @ DrawOp::Fill { .. }, .. } => Some(op.clone()),
            _ => None,
        })
        .expect("source Fill op")
}

fn source_text(g: &G, bm: u32) -> (DrawOp, (i32, i32, i32, i32)) {
    g.a.gfx
        .surfaces
        .get(bm)
        .expect("offscreen surface")
        .rec
        .iter()
        .find_map(|rec| match rec {
            Rec::Op {
                op: op @ DrawOp::Text { string, .. },
                ..
            } if string == "Copy" => {
                let b = rec.bbox();
                Some((op.clone(), (b.x0, b.y0, b.width() as i32, b.height() as i32)))
            }
            _ => None,
        })
        .expect("source Text op")
}

fn overlaps(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

fn has_copy_text(ops: &[DrawOp]) -> bool {
    ops.iter().any(|op| matches!(op, DrawOp::Text { string, .. } if string == "Copy"))
}

#[test]
fn offscreen_fill_and_text_splice_into_window_plane_and_non_copy_minterm_becomes_pixels_hole() {
    let mut g = G::new_plane();
    let (_win, wrp, _) = open(&mut g, 0, 0);
    let (bm, _brp) = draw_offscreen(&mut g);
    let src_fill = source_fill(&g, bm);
    let (src_text, (tx, ty, tw, th)) = source_text(&g, bm);

    g.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm),
            d(0, 0),
            d(1, 0),
            a(1, wrp),
            d(2, 20),
            d(3, 10),
            d(4, 80),
            d(5, 50),
            d(6, 0xC0),
        ],
    )]);

    assert!(g.a.refused.is_empty());
    let ps = planes(&g);
    assert_eq!(ps.len(), 1);
    let ops = &ps[0].ops;

    let expected_fill = moved(&src_fill, (20, 10));
    let expected_text = moved(&src_text, (20, 10));
    assert!(ops.contains(&expected_fill), "ops: {ops:?}");
    assert!(ops.contains(&expected_text), "ops: {ops:?}");

    let dst_text_box = (tx + 20, ty + 10, tw, th);
    for op in ops {
        if let DrawOp::Pixels { rect, .. } = op {
            let r = (rect.left, rect.top, rect.width as i32, rect.height as i32);
            assert!(!overlaps(r, dst_text_box), "Pixels {r:?} overlaps text {dst_text_box:?}");
        }
    }

    // Negative control: minterm 0x50 is not a straight copy and must emit a Pixels hole.
    let mut g_ctrl = G::new_plane();
    let (_win_c, wrp_c, _) = open(&mut g_ctrl, 0, 0);
    let (bm_c, _brp_c) = draw_offscreen(&mut g_ctrl);
    g_ctrl.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm_c),
            d(0, 0),
            d(1, 0),
            a(1, wrp_c),
            d(2, 20),
            d(3, 10),
            d(4, 80),
            d(5, 50),
            d(6, 0x50),
        ],
    )]);

    assert!(g_ctrl.a.refused.is_empty());
    let ps_c = planes(&g_ctrl);
    assert_eq!(ps_c.len(), 1);
    let ops_c = &ps_c[0].ops;
    assert!(!has_copy_text(ops_c), "unexpected Text in 0x50 blit: {ops_c:?}");
    assert!(
        ops_c.iter().any(|op| matches!(
            op,
            DrawOp::Pixels { rect, .. }
                if rect.left == 20 && rect.top == 10 && rect.width == 80 && rect.height == 50
        )),
        "expected 80x50 Pixels hole at (20, 10), got: {ops_c:?}"
    );
}

#[test]
fn direct_plane_ram_poke_invalidates_offscreen_surface_and_falls_back_to_pixels() {
    let mut g = G::new_plane();
    let (_win, wrp, _) = open(&mut g, 0, 0);
    let rp_bm = guest_planes_rp(&mut g);
    let (bm, _brp) = draw_into(&mut g, rp_bm);

    let plane0 = g.r32(bm + 8);
    assert_ne!(plane0, 0);
    let old = g.r8(plane0);
    g.a.mem.write_u8(plane0, old ^ 0xFF).unwrap();

    g.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm),
            d(0, 0),
            d(1, 0),
            a(1, wrp),
            d(2, 20),
            d(3, 10),
            d(4, 80),
            d(5, 50),
            d(6, 0xC0),
        ],
    )]);

    assert!(g.a.refused.is_empty());
    let ps = planes(&g);
    assert_eq!(ps.len(), 1);
    let ops = &ps[0].ops;
    assert!(!has_copy_text(ops), "poked surface should not splice Text: {ops:?}");
    assert!(
        ops.iter().any(|op| matches!(
            op,
            DrawOp::Pixels { rect, .. }
                if rect.left == 20 && rect.top == 10 && rect.width == 80 && rect.height == 50
        )),
        "expected Pixels fallback over copied rect, got: {ops:?}"
    );

    // Negative control: without the plane RAM poke, the same blit splices the Text op.
    let mut g_ctrl = G::new_plane();
    let (_win_c, wrp_c, _) = open(&mut g_ctrl, 0, 0);
    let rp_bm_c = guest_planes_rp(&mut g_ctrl);
    let (bm_c, _brp_c) = draw_into(&mut g_ctrl, rp_bm_c);
    g_ctrl.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm_c),
            d(0, 0),
            d(1, 0),
            a(1, wrp_c),
            d(2, 20),
            d(3, 10),
            d(4, 80),
            d(5, 50),
            d(6, 0xC0),
        ],
    )]);
    let ps_c = planes(&g_ctrl);
    assert_eq!(ps_c.len(), 1);
    assert!(has_copy_text(&ps_c[0].ops));
}

#[test]
fn source_rect_straddling_text_bbox_converts_the_sliced_part_into_a_pixels_hole() {
    let mut g = G::new_plane();
    let (_win, wrp, _) = open(&mut g, 0, 0);
    let (bm, _brp) = draw_offscreen(&mut g);
    let (_src_text, (tx, ty, tw, th)) = source_text(&g, bm);
    let cut = tx + tw / 2;
    assert!(tx < cut && cut < tx + tw);

    g.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm),
            d(0, 0),
            d(1, 0),
            a(1, wrp),
            d(2, 12),
            d(3, 8),
            d(4, cut as u32),
            d(5, 60),
            d(6, 0xC0),
        ],
    )]);

    assert!(g.a.refused.is_empty());
    let ps = planes(&g);
    assert_eq!(ps.len(), 1);
    let ops = &ps[0].ops;
    assert!(!has_copy_text(ops), "straddled Text must not splice as Text: {ops:?}");

    let expected_left = 12 + tx;
    let expected_top = 8 + ty;
    let expected_width = cut - tx;
    let expected_height = th;
    assert!(
        ops.iter().any(|op| matches!(
            op,
            DrawOp::Pixels { rect, .. }
                if rect.left == expected_left
                    && rect.top == expected_top
                    && rect.width == expected_width
                    && rect.height == expected_height
        )),
        "expected Pixels hole at ({expected_left}, {expected_top}, {expected_width}, {expected_height}), got: {ops:?}"
    );

    // Negative control: blitting the full source rect containing the whole text bbox splices Text.
    let mut g_ctrl = G::new_plane();
    let (_win_c, wrp_c, _) = open(&mut g_ctrl, 0, 0);
    let (bm_c, _brp_c) = draw_offscreen(&mut g_ctrl);
    let (src_text_c, (tx_c, ty_c, tw_c, th_c)) = source_text(&g_ctrl, bm_c);
    assert!(tx_c >= 0 && ty_c >= 0 && tx_c + tw_c <= 100 && ty_c + th_c <= 60);

    g_ctrl.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm_c),
            d(0, 0),
            d(1, 0),
            a(1, wrp_c),
            d(2, 12),
            d(3, 8),
            d(4, 100),
            d(5, 60),
            d(6, 0xC0),
        ],
    )]);

    let ps_c = planes(&g_ctrl);
    assert_eq!(ps_c.len(), 1);
    let expected_text = moved(&src_text_c, (12, 8));
    assert!(ps_c[0].ops.contains(&expected_text), "ops: {:?}", ps_c[0].ops);
}

#[test]
fn freeing_bitmap_drops_its_recorded_surface_so_a_new_bitmap_does_not_inherit_ops() {
    let mut g = G::new_plane();
    let (_win, wrp, _) = open(&mut g, 0, 0);
    let (bm_a, _rp_a) = draw_offscreen(&mut g);
    assert!(g.a.gfx.surfaces.get(bm_a).is_some());

    g.run(&[gfx(FREE_BITMAP, &[a(0, bm_a)])]);
    assert!(g.a.gfx.surfaces.get(bm_a).is_none());

    let (bm_b, _rp_b) = g.bitmap_rp(100, 60, 3);
    g.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm_b),
            d(0, 0),
            d(1, 0),
            a(1, wrp),
            d(2, 20),
            d(3, 10),
            d(4, 80),
            d(5, 50),
            d(6, 0xC0),
        ],
    )]);

    assert!(g.a.refused.is_empty());
    for p in planes(&g) {
        assert!(!has_copy_text(&p.ops), "freed surface ops leaked into new bitmap: {:?}", p.ops);
    }

    // Negative control: blitting bitmap A before freeing splices the recorded Text.
    let mut g_ctrl = G::new_plane();
    let (_win_c, wrp_c, _) = open(&mut g_ctrl, 0, 0);
    let (bm_c, _rp_c) = draw_offscreen(&mut g_ctrl);
    g_ctrl.run(&[gfx(
        BLT_BITMAP_RASTPORT,
        &[
            a(0, bm_c),
            d(0, 0),
            d(1, 0),
            a(1, wrp_c),
            d(2, 20),
            d(3, 10),
            d(4, 80),
            d(5, 50),
            d(6, 0xC0),
        ],
    )]);
    let ps_c = planes(&g_ctrl);
    assert_eq!(ps_c.len(), 1);
    assert!(has_copy_text(&ps_c[0].ops));
}

#[test]
fn surface_splice_logs_once_per_window() {
    let mut g = G::new_plane();
    let (_win, wrp, _) = open(&mut g, 0, 0);
    let (bm, _brp) = draw_offscreen(&mut g);

    let count_logs = |g: &G| {
        g.a.log
            .iter()
            .filter(|s| s.contains("[AMI-SURFACE-SPLICE]"))
            .count()
    };

    assert_eq!(count_logs(&g), 0);

    g.run(&[
        gfx(
            BLT_BITMAP_RASTPORT,
            &[
                a(0, bm),
                d(0, 0),
                d(1, 0),
                a(1, wrp),
                d(2, 20),
                d(3, 10),
                d(4, 80),
                d(5, 50),
                d(6, 0xC0),
            ],
        ),
        gfx(
            BLT_BITMAP_RASTPORT,
            &[
                a(0, bm),
                d(0, 0),
                d(1, 0),
                a(1, wrp),
                d(2, 40),
                d(3, 20),
                d(4, 80),
                d(5, 50),
                d(6, 0xC0),
            ],
        ),
    ]);

    assert_eq!(count_logs(&g), 1);
}