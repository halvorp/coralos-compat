//! gadtools.library DrawBevelBoxA is a native `Bevel` display-list op (never canvas pixels); frame types the kit has no op for are
//! refused by name (AROS gadtools drawbevelboxa.c, rom/intuition frameiclass.c).
mod gfxcommon;
use gfxcommon::*;
use amiga68k_engine::machine::GADTOOLS_BASE;
use guestui_abi::*;
use retro_common::Step;

const DRAW_BEVEL_BOX_A: i32 = -120;
const GT: u32 = TAG_USER + 0x0008_0000;

fn open(g: &mut G) -> (u32, u32) {
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 200), (6, 100)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 14, 0x800).unwrap();
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    let win = g.result(0);
    (win, g.r32(win + 50))
}

/// Call DrawBevelBoxA(rp, 10, 20, 41, 21, tags) and return how the program ended (a refusal is recorded in `refused`).
fn bevel(g: &mut G, rp: u32, tags: &[(u32, u32)]) -> Step {
    let t = g.tags(tags);
    g.a.start_hunk(&script(&[(GADTOOLS_BASE, DRAW_BEVEL_BOX_A, vec![a(0, rp), d(0, 10), d(1, 20), d(2, 41), d(3, 21), a(1, t)])])).unwrap();
    g.a.step(10_000_000).0
}

fn planes(g: &G) -> Vec<Plane> { g.a.ui.calls.iter().filter_map(|c| if let UiCall::PresentPlane(_, p) = c { Some(p.clone()) } else { None }).collect() }

#[test]
fn a_bevel_box_is_a_native_bevel_op_and_no_pixels() {
    let mut g = G::new_plane();
    let (_, rp) = open(&mut g);
    assert_eq!(bevel(&mut g, rp, &[(GT + 52, 1)]), Step::Exited);
    let ops = planes(&g).last().map(|p| p.ops.clone()).unwrap_or_default();
    assert!(ops.contains(&DrawOp::Bevel { rect: GuestRect { left: 10, top: 20, width: 41, height: 21 }, raised: true }), "{ops:?}");
    assert!(!ops.iter().any(|o| matches!(o, DrawOp::Pixels { .. } | DrawOp::Line { .. })), "system UI is never guest pixels or guest lines: {ops:?}");
    assert_eq!(bevel(&mut g, rp, &[(GT + 52, 1), (GT + 51, 1)]), Step::Exited);
    let ops = planes(&g).last().unwrap().ops.clone();
    assert!(ops.contains(&DrawOp::Bevel { rect: GuestRect { left: 10, top: 20, width: 41, height: 21 }, raised: false }), "GTBB_Recessed: {ops:?}");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

#[test]
fn without_visual_info_nothing_is_drawn() {
    let mut g = G::new_plane();
    let (_, rp) = open(&mut g);
    let before = planes(&g).len();
    assert_eq!(bevel(&mut g, rp, &[]), Step::Exited);
    assert_eq!(planes(&g).len(), before, "drawbevelboxa.c:69-71");
}

#[test]
fn ridge_and_icondropbox_are_refused_by_name_and_draw_nothing() {
    for (ft, name) in [(2, "BBFT_RIDGE"), (3, "BBFT_ICONDROPBOX")] {
        let mut g = G::new_plane();
        let (_, rp) = open(&mut g);
        let before = planes(&g).len();
        let step = bevel(&mut g, rp, &[(GT + 52, 1), (GT + 77, ft)]);
        assert_eq!(step, Step::Exited);
        assert!(g.a.refused.iter().any(|l| l.contains(name)), "{name} is named: {:?}", g.a.refused);
        assert_eq!(planes(&g).len(), before, "{name}: no op");
    }
}

#[test]
fn a_host_without_the_plane_route_is_refused_not_shown_blank() {
    let mut g = G::new();
    let (_, rp) = open(&mut g);
    let step = bevel(&mut g, rp, &[(GT + 52, 1)]);
    assert_eq!(step, Step::Exited);
    assert!(g.a.refused.iter().any(|l| l.contains("no plane route")), "{:?}", g.a.refused);
}
