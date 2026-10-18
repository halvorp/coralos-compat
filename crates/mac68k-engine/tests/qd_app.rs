//! An assembled classic-style application (qdapp.s) draws its window on an update event; the host sees one
//! `present_content` with the right pixels. Every expectation below is derived from the shapes in qdapp.s, not read back
//! from the engine.
mod common;
use common::*;
use guestui_abi::{pixel_hash, GuestRect, UiCall};
use mac68k_engine::fixture::QDAPP;
use mac68k_engine::Step;
use retro_raster::font::Font;

fn frames(g: &Rig) -> Vec<(GuestRect, u64)> {
    g.m.ui.calls.iter().filter_map(|c| if let UiCall::PresentContent { damage, hash, .. } = c { Some((*damage, *hash)) } else { None }).collect()
}
fn glyph_at(g: &Rig, ch: u8, x: i32, baseline: i32) -> bool {
    let f = Font::topaz8();
    (0..8).all(|gy| (0..8).all(|gx| g.black(x + gx, baseline - 7 + gy) == f.glyph_bit(f.slot(ch), gx, gy)))
}
fn gray(x: i32, y: i32) -> bool { // qd.gray = $AA55AA55 AA55AA55, aligned to the port's LOCAL coordinates (x, y)
    [0xAAu8, 0x55][(y & 1) as usize] >> (7 - (x & 7)) & 1 == 1
}

#[test]
fn the_application_draws_on_its_update_event_and_the_host_gets_one_frame() {
    let mut g = Rig::canvas();
    let win = g.win;
    g.m.post(6, win, 0, 0); // updateEvt
    g.m.start(QDAPP, 0).unwrap();
    let r = g.m.run_loop(2_000_000).unwrap();
    assert!(r.stats.blocked, "the app is waiting in GetNextEvent: {:#?}", g.m.log);
    g.no_refusals();
    assert_eq!(g.m.mem.read_u16(0x3000).unwrap(), 1, "one update handled");

    // ---- exact pixels, from qdapp.s
    // r1 (top10 left10 bottom50 right90) framed with a 2x2 pen
    assert!(g.black(10, 10) && g.black(11, 11) && !g.black(12, 12) && g.black(89, 49) && g.black(88, 30) && !g.black(87, 30), "{}", g.pic(8, 8, 94, 54));
    // r2 painted black (rows 60..80, cols 10..50)
    assert!(g.black(10, 60) && g.black(49, 79) && !g.black(50, 60) && !g.black(10, 80));
    // r3 filled with qd.gray (rows 60..80, cols 60..100): a checkerboard aligned to the SCREEN
    assert!((60..100).all(|x| (60..80).all(|y| g.black(x, y) == gray(x, y))), "{}", g.pic(58, 58, 102, 82));
    assert_eq!(g.count_in(60, 60, 100, 80), 400);
    // r4 painted oval (rows 90..118, cols 10..60): the centre is black, the corner is not
    assert!(g.black(35, 104) && !g.black(10, 90) && !g.black(59, 117));
    // r5 round-rect frame (rows 90..118, cols 70..130): edge middles black, centre white, corner cut
    assert!(g.black(100, 90) && g.black(70, 104) && !g.black(100, 104) && !g.black(70, 90));
    // the line (140,10) -> (190,40) with a 1x1 pen
    assert!(g.black(140, 10) && g.black(190, 40) && g.black(165, 25));
    // r6 inverted inside r1's white interior (rows 20..30, cols 20..30)
    assert_eq!(g.count_in(20, 20, 30, 30), 100);
    // "QD" at the pen (110,30) in the stand-in font
    assert!(glyph_at(&g, b'Q', 110, 30) && glyph_at(&g, b'D', 118, 30), "{}", g.pic(108, 22, 128, 34));

    // ---- one frame, whole canvas, hash of exactly those pixels
    let f = frames(&g);
    assert_eq!(f.len(), 1, "{:?}", g.m.ui.calls);
    assert_eq!(f[0].0, GuestRect { left: 0, top: 0, width: 200, height: 120 });
    let native = g.m.windows[&win].native.unwrap();
    let shown = g.m.ui.canvases[&native.id].2.clone();
    assert_eq!(f[0].1, pixel_hash(&shown));
    assert_eq!(shown.iter().filter(|&&p| p == 0).count(), g.count_black());
    // a golden hash: any change to scan conversion, patterns or the font shows up here
    assert_eq!(f[0].1, GOLDEN, "pixel hash of the whole window (update GOLDEN deliberately if the drawing changed)");

    // ---- negative controls
    // a second update event with nothing invalid: BeginUpdate narrows the visRgn to nothing, so the redraw lands nowhere
    g.m.post(6, win, 0, 0);
    let before = g.count_black();
    let r = g.m.run_loop(2_000_000).unwrap();
    assert!(r.stats.blocked);
    assert_eq!(g.m.mem.read_u16(0x3000).unwrap(), 2, "the app did handle the second update");
    assert_eq!(g.count_black(), before, "...but nothing was invalid, so nothing was repainted");
    assert_eq!(frames(&g).len(), 1, "and no second frame was presented");
    // InvalRect on a small area makes the next update redraw only there (the erase + shapes clipped to it)
    set_rect(&mut g.m, 0x3100, 56, 6, 84, 54);
    g.m.start(&Prog::new().set_port(win).l(0x3100).trap(0xA928).exit().0, 0).unwrap(); // InvalRect
    g.m.run_loop(100_000).unwrap();
    g.m.ev.queue.retain(|e| e.what == 6); // keep the update event InvalRect posted
    g.m.start(QDAPP, 0).unwrap();
    let r = g.m.run_loop(2_000_000).unwrap();
    assert!(r.stats.blocked);
    let f = frames(&g);
    assert_eq!(f.len(), 2, "{:?}", g.m.ui.calls);
    assert_eq!(f[1].0.width < 200 && f[1].0.height < 120, true, "a partial update presents only the damaged part: {:?}", f[1].0);
    assert!(f[1].0.left >= 6 && f[1].0.top >= 56, "{:?}", f[1].0);

    // keyDown ends the program
    g.m.post(3, 0x61, 0, 0);
    let (step, _) = g.m.step(1_000_000);
    assert_eq!(step, Step::Exited);
}

const GOLDEN: u64 = 1141123241323567110; // re-recorded 2026-10-18 when lines (DrawLine.a slabs) and ovals (DrawArc.a INITOVAL/BUMPOVAL) became the original algorithms; see tests/oracle/systemless
