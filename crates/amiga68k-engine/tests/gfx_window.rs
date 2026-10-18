//! A guest program draws into its window: real binary-level fixture (tests/guest/gfxdraw.s) and call scripts, checked
//! through what the UI host receives (`present_content`): the frame count, the damage rectangle, the resolved pixels and
//! their hash. Controls: the other draw mode, a bordered window's origin shift, a step with nothing drawn.
mod gfxcommon;
use amiga68k_engine::{Amiga, Step};
use gfxcommon::*;
use guestui_abi::*;
use retro_hunk::writer::{build, Hunk};

const GFXDRAW: &[u8] = include_bytes!("guest/gfxdraw.bin");
const GREY0: u32 = 0xAAAAAA; // pen 0
const BLACK: u32 = 0x000000; // pen 1
const WHITE: u32 = 0xFFFFFF; // pen 2
const BLUE: u32 = 0x6688BB; // pen 3

fn run_fixture(mode: u32, wflags: u32) -> Amiga<RecordingGuestUi> {
    let mut code = GFXDRAW.to_vec();
    code[4..8].copy_from_slice(&mode.to_be_bytes());
    code[8..12].copy_from_slice(&wflags.to_be_bytes());
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.start_hunk(&build(&[Hunk::code(code, vec![])], None)).unwrap();
    let (s, _) = a.step(1_000_000);
    assert_eq!(s, Step::Blocked, "{:?} {:?}", a.refused, a.log);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    a
}

fn canvas(a: &Amiga<RecordingGuestUi>) -> (i32, i32, Vec<u32>) {
    let id = *a.ui.canvases.keys().next().expect("a content frame");
    a.ui.canvases[&id].clone()
}
fn at(c: &(i32, i32, Vec<u32>), x: i32, y: i32) -> u32 { c.2[(y * c.0 + x) as usize] }
fn presents(a: &Amiga<RecordingGuestUi>) -> Vec<&UiCall> { a.ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. })).collect() }

#[test]
fn a_program_draws_a_rect_and_text_and_the_host_receives_one_frame_with_the_pixels() {
    let a = run_fixture(1, 0x800);
    let c = canvas(&a);
    assert_eq!((c.0, c.1), (200, 100), "a borderless 200x100 window: the canvas is the whole window");
    assert_eq!(presents(&a).len(), 1, "one frame for a whole drawing step, not one per primitive");
    let UiCall::PresentContent { damage, hash, .. } = presents(&a)[0] else { unreachable!() };
    assert_eq!(*damage, GuestRect { left: 0, top: 0, width: 200, height: 100 }, "the first frame is the whole canvas");
    assert_eq!(*hash, pixel_hash(&c.2));
    // RectFill(10,10)-(29,19) in pen 2
    assert_eq!((at(&c, 10, 10), at(&c, 29, 19), at(&c, 9, 10), at(&c, 30, 10), at(&c, 10, 20)), (WHITE, WHITE, GREY0, GREY0, GREY0));
    assert_eq!(c.2.iter().filter(|&&p| p == WHITE).count(), 20 * 10);
    // Text "AB" at (40,30), JAM2 pen 1 on pen 3: cells x 40..56, y 24..32
    assert_eq!(at(&c, 40, 24), BLUE, "the cell's paper is the background pen");
    assert_eq!(at(&c, 43, 25), BLACK, "row 1 of 'A' (0x18) sets x=3,4");
    assert_eq!(at(&c, 56, 24), GREY0, "control: right of the last cell");
    let ink: u32 = (b"AB").iter().map(|&ch| (0..8u32).map(|row| (retro_raster::topaz8::CHAR_DATA[(row * 192 + (ch as u32 - 32)) as usize]).count_ones()).sum::<u32>()).sum();
    assert_eq!(c.2.iter().filter(|&&p| p == BLACK).count() as u32, ink, "exactly the glyph ink bits of 'A' and 'B' in topaz 8");
    assert_eq!(c.2.iter().filter(|&&p| p == BLUE).count() as u32, 16 * 8 - ink);
    // the golden fingerprint of the whole frame (pin; re-derive by dumping the canvas if the font or palette is changed on purpose)
    assert_eq!(pixel_hash(&c.2), 4293717206870754438, "golden hash of the whole 200x100 frame (the pixel assertions above pin its content)");
}

#[test]
fn jam1_text_leaves_the_background_unlike_jam2() {
    let j2 = canvas(&run_fixture(1, 0x800));
    let j1 = canvas(&run_fixture(0, 0x800));
    assert_eq!((at(&j2, 40, 24), at(&j1, 40, 24)), (BLUE, GREY0));
    assert_eq!(at(&j1, 43, 25), BLACK, "the ink is the same");
    assert_ne!(pixel_hash(&j1.2), pixel_hash(&j2.2));
}

#[test]
fn a_bordered_window_draws_relative_to_its_top_left_and_the_canvas_is_the_inner_area() {
    let c = canvas(&run_fixture(1, 0));
    assert_eq!((c.0, c.1), (200 - 8, 100 - 8), "4-pixel borders all round (no title): inner 192x92");
    assert_eq!(at(&c, 6, 6), WHITE, "RPort (10,10) is canvas (6,6): the frame is outside the canvas");
    assert_eq!(at(&c, 10, 10), WHITE);
    assert_eq!(at(&c, 5, 5), GREY0);
    assert_eq!(c.2.iter().filter(|&&p| p == WHITE).count(), 200);
}

#[test]
fn later_steps_publish_only_their_damage_palette_changes_republish_everything_and_idle_steps_publish_nothing() {
    let mut g = G::new();
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 20u16), (2, 10), (4, 100), (6, 60)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 14, 0x800).unwrap();
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    let win = g.result(0);
    let rp = g.r32(win + 50);
    assert_ne!(rp, 0);
    assert_eq!(g.r32(win + 124) != 0, true, "Window.WLayer is real");
    assert!(presents_of(&g).is_empty(), "nothing drawn yet: nothing published");
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 3)]), gfx(RECT_FILL, &[a(1, rp), d(0, 5), d(1, 6), d(2, 14), d(3, 8)])]);
    assert_eq!(presents_of(&g).len(), 1);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 2)]), gfx(RECT_FILL, &[a(1, rp), d(0, 50), d(1, 40), d(2, 59), d(3, 41)])]);
    let ps = presents_of(&g);
    assert_eq!(ps.len(), 2);
    assert_eq!(ps[1], GuestRect { left: 50, top: 40, width: 10, height: 2 }, "the second frame is only what changed");
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 2)])]);
    assert_eq!(presents_of(&g).len(), 2, "control: a step that draws nothing publishes nothing");
    g.run(&[gfx(SET_RGB32, &[a(0, 0), d(0, 3), d(1, 0xFF00_0000), d(2, 0), d(3, 0)])]);
    let ps = presents_of(&g);
    assert_eq!(ps[2], GuestRect { left: 0, top: 0, width: 100, height: 60 }, "a palette change republishes the canvas");
    let id = *g.a.ui.canvases.keys().next().unwrap();
    let c = &g.a.ui.canvases[&id];
    assert_eq!(c.2[(6 * 100 + 5) as usize], 0xFF0000, "pen 3 is now red in the already-drawn rectangle");
    assert_eq!(c.2[(41 * 100 + 50) as usize], WHITE);
}

fn presents_of(g: &G) -> Vec<GuestRect> {
    g.a.ui.calls.iter().filter_map(|c| if let UiCall::PresentContent { damage, .. } = c { Some(*damage) } else { None }).collect()
}

#[test]
fn drawing_on_the_window_frame_is_dropped_by_name_and_clips_to_the_canvas() {
    let mut g = G::new();
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 80), (6, 40)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    let win = g.result(0);
    let (rp, brp) = (g.r32(win + 50), g.r32(win + 58));
    g.run(&[gfx(SET_APEN, &[a(1, brp), d(0, 2)]), gfx(RECT_FILL, &[a(1, brp), d(0, 0), d(1, 0), d(2, 20), d(3, 20)]),
        gfx(SET_APEN, &[a(1, rp), d(0, 2)]), gfx(RECT_FILL, &[a(1, rp), di(0, -50), di(1, -50), d(2, 5), d(3, 5)])]);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
    assert!(g.a.log.iter().any(|l| l.contains("RectFill") && l.contains("dropped") && l.contains("native")), "the BorderRPort draw is logged as dropped");
    let id = *g.a.ui.canvases.keys().next().unwrap();
    let c = &g.a.ui.canvases[&id];
    assert_eq!(c.2.iter().filter(|&&p| p == WHITE).count(), 4, "BorderRPort drew nothing (a 21x21 rect would show); the RPort rect (-50,-50)-(5,5) meets the canvas (origin 4,4) in 2x2 pixels");
}
