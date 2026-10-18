//! The Amiga side of the translation layer (ruling 17 / docs/retro-compat/17-TRANSLATION-ARCHITECTURE.md, First slice items 3 and 4):
//! (1) a window's drawing reaches the host as ONE display-list plane per step, with pen ROLES from the DrawInfo pen table and holes
//! (`Pixels`) for everything it cannot name; (2) plain pointer and key input in a window's content becomes the IntuiMessage Intuition
//! would send. Every behaviour has a negative control.
mod gfxcommon;
use amiga68k_engine::Delivered;
use gfxcommon::*;
use guestui_abi::*;

const EXEC: u32 = 0x1000;
const GET_MSG: i32 = -372;

fn open(g: &mut G, idcmp: u32, wflags: u32) -> (u32, UiObjectRef) {
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 200), (6, 100)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 10, idcmp).unwrap();
    g.a.mem.write_u32(nw + 14, wflags | 0x800).unwrap(); // WFLG_BORDERLESS: canvas = window
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    let win = g.result(0);
    (win, g.a.windows[&win])
}

fn planes(g: &G) -> Vec<Plane> { g.a.ui.calls.iter().filter_map(|c| if let UiCall::PresentPlane(_, p) = c { Some(p.clone()) } else { None }).collect() }
fn content_frames(g: &G) -> usize { g.a.ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. })).count() }

#[test]
fn drawing_is_one_plane_per_step_with_roles_from_the_pen_table_and_holes_for_what_has_no_role() {
    let mut g = G::new_plane();
    g.shine_pen = Some(2); // distinct SHINE (pen 2) and SHADOW (pen 4): the Gute Form flat edge has one pen for both
    let (win, native) = open(&mut g, 0, 0);
    let rp = g.r32(win + 50);
    let (env, font) = (g.s("Env"), retro_raster::font::Font::gute_form());
    // dri_Pens (retro-theme): TEXT 1, SHADOW 4, FILL 3, FILLTEXT 2, BACKGROUND 0; SHINE is rewritten to pen 2 by `shine_pen`.
    g.run(&[
        gfx(SET_APEN, &[a(1, rp), d(0, 3)]), gfx(RECT_FILL, &[a(1, rp), d(0, 100), d(1, 5), d(2, 140), d(3, 20)]),
        gfx(SET_APEN, &[a(1, rp), d(0, 1)]), gfx(MOVE, &[a(1, rp), d(0, 12), d(1, 60)]), gfx(TEXT, &[a(1, rp), a(0, env), d(0, 3)]),
        gfx(SET_APEN, &[a(1, rp), d(0, 2)]), gfx(MOVE, &[a(1, rp), d(0, 10), d(1, 40)]), gfx(DRAW, &[a(1, rp), d(0, 10), d(1, 20)]), gfx(DRAW, &[a(1, rp), d(0, 49), d(1, 20)]),
        gfx(SET_APEN, &[a(1, rp), d(0, 4)]), gfx(MOVE, &[a(1, rp), d(0, 50), d(1, 20)]), gfx(DRAW, &[a(1, rp), d(0, 50), d(1, 40)]), gfx(DRAW, &[a(1, rp), d(0, 10), d(1, 40)]),
        gfx(SET_APEN, &[a(1, rp), d(0, 100)]), gfx(RECT_FILL, &[a(1, rp), d(0, 150), d(1, 70), d(2, 159), d(3, 79)]),
    ]);
    let p = planes(&g);
    assert_eq!(p.len(), 1, "one present_plane for the whole drawing step; log {:?}", g.a.log);
    assert_eq!(content_frames(&g), 0, "the plane route replaces present_content");
    assert_eq!(p[0].rect, GuestRect { left: 0, top: 0, width: 200, height: 100 }, "the first frame is the whole canvas, in window coordinates");
    assert!(p[0].validate().is_ok());
    let ops = &p[0].ops;
    assert!(ops.contains(&DrawOp::Fill { rect: GuestRect { left: 100, top: 5, width: 41, height: 16 }, pen_role: PenRole::Fill }), "RectFill in the FILLPEN: {ops:?}");
    let top = 60 - font.baseline as i32;
    assert!(ops.contains(&DrawOp::Text { at: GuestPoint { x: 12, y: top }, string: "Env".into(), font_role: FontRole::Ui, px: font.y_size, pen_role: PenRole::Text, bg_role: Some(PenRole::Background) }),
        "a TEXTPEN run in JAM2 over the BACKGROUNDPEN, the proportional face as Ui: {ops:?}");
    assert!(ops.contains(&DrawOp::Bevel { rect: GuestRect { left: 10, top: 20, width: 41, height: 21 }, raised: true }), "SHINE then SHADOW edges of one box are one bevel: {ops:?}");
    assert!(!ops.iter().any(|o| matches!(o, DrawOp::Line { pen_role: PenRole::Shine | PenRole::Shadow, .. })), "the edge lines were merged: {ops:?}");
    // NEGATIVE CONTROL (RETRO-ROLES-REQUIRE-A-CONTRACT): pen 100 is in no DrawInfo slot, so its fill has NO role; it is pixels.
    let holes: Vec<_> = ops.iter().filter(|o| matches!(o, DrawOp::Pixels { .. })).collect();
    assert!(holes.iter().any(|o| matches!(o, DrawOp::Pixels { rect, .. } if *rect == GuestRect { left: 150, top: 70, width: 10, height: 10 })), "{ops:?}");
    assert!(!ops.iter().any(|o| matches!(o, DrawOp::Fill { rect, .. } if rect.left == 150)), "a pen without a role is never guessed into one");
    // The census counts the Pixels hole as raw; the window has no native control.
    assert_eq!(g.a.ui.route_census(native), RouteCensus { native: 0, replayed: 0, raw: 1 });
    // A step that draws nothing publishes nothing; one that draws a little publishes only its damage, with only the ops that touch it.
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 3)]), gfx(RECT_FILL, &[a(1, rp), d(0, 100), d(1, 5), d(2, 109), d(3, 9)])]);
    let p = planes(&g);
    assert_eq!(p.len(), 2);
    assert_eq!(p[1].rect, GuestRect { left: 100, top: 5, width: 10, height: 5 });
    assert!(p[1].ops.iter().all(|o| !matches!(o, DrawOp::Bevel { .. } | DrawOp::Pixels { .. })), "{:?}", p[1].ops);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

#[test]
fn a_flat_theme_edge_is_the_shadow_colour_and_never_a_bevel() {
    // The shipped Gute Form table has SHINEPEN == SHADOWPEN == 4: one hairline. Both edges are the border colour (Shadow), and a box
    // whose two edges are the SAME pen is a flat outline, not a raised bevel the guest never drew.
    let mut g = G::new_plane();
    let (win, _) = open(&mut g, 0, 0);
    let rp = g.r32(win + 50);
    g.run(&[
        gfx(SET_APEN, &[a(1, rp), d(0, 4)]),
        gfx(MOVE, &[a(1, rp), d(0, 10), d(1, 40)]), gfx(DRAW, &[a(1, rp), d(0, 10), d(1, 20)]), gfx(DRAW, &[a(1, rp), d(0, 49), d(1, 20)]),
        gfx(MOVE, &[a(1, rp), d(0, 50), d(1, 20)]), gfx(DRAW, &[a(1, rp), d(0, 50), d(1, 40)]), gfx(DRAW, &[a(1, rp), d(0, 10), d(1, 40)]),
    ]);
    let ops = &planes(&g)[0].ops;
    assert!(!ops.iter().any(|o| matches!(o, DrawOp::Bevel { .. } | DrawOp::Line { pen_role: PenRole::Shine, .. })), "{ops:?}");
    assert!(ops.iter().filter(|o| matches!(o, DrawOp::Line { pen_role: PenRole::Shadow, .. })).count() >= 4, "{ops:?}");
}

#[test]
fn a_write_mask_makes_a_hole_not_a_guess() {
    let mut g = G::new_plane();
    let (win, _) = open(&mut g, 0, 0);
    let rp = g.r32(win + 50);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 1)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 9), d(3, 9)])]);
    assert!(planes(&g)[0].ops.iter().any(|o| matches!(o, DrawOp::Fill { pen_role: PenRole::Text, .. })), "control: the plain fill is named");
    g.a.mem.write_u8(rp + 24, 0x0F).unwrap(); // RastPort.Mask: only some planes
    g.run(&[gfx(RECT_FILL, &[a(1, rp), d(0, 20), d(1, 20), d(2, 29), d(3, 29)])]);
    let p = planes(&g);
    assert!(!p[1].ops.iter().any(|o| matches!(o, DrawOp::Fill { .. })), "a write mask changes what the fill means: {:?}", p[1].ops);
    assert!(p[1].ops.iter().any(|o| matches!(o, DrawOp::Pixels { .. })), "{:?}", p[1].ops);
}

#[test]
fn a_minterm_blit_into_a_window_reaches_the_plane_as_pixels() {
    let mut g = G::new_plane();
    let (win, _) = open(&mut g, 0, 0);
    let rp = g.r32(win + 50);
    g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 1)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 9), d(3, 9)])]);
    g.run(&[gfx(CLIP_BLIT, &[a(0, rp), d(0, 0), d(1, 0), a(1, rp), d(2, 50), d(3, 50), d(4, 10), d(5, 10), d(6, 0xC0)])]);
    let p = planes(&g);
    assert_eq!(p.len(), 2, "the blit damaged the window, so a plane is published");
    assert!(p[1].ops.iter().any(|o| matches!(o, DrawOp::Pixels { rect, .. } if *rect == GuestRect { left: 50, top: 50, width: 10, height: 10 })),
        "the blitted rectangle is a hole of canvas pixels: {:?}", p[1].ops);
    // NEGATIVE CONTROL: a blit entirely outside the canvas damages nothing and publishes nothing.
    g.run(&[gfx(CLIP_BLIT, &[a(0, rp), d(0, 0), d(1, 0), a(1, rp), d(2, 500), d(3, 500), d(4, 10), d(5, 10), d(6, 0xC0)])]);
    assert_eq!(planes(&g).len(), 2);
}

fn message(g: &mut G, win: u32) -> Option<(u32, u16, u16, i16, i16, u32, u32)> {
    let user = g.r32(win + 86);
    g.run(&[(EXEC, GET_MSG, vec![a(0, user)])]);
    let m = g.result(0);
    (m != 0).then(|| (g.r32(m + 20), g.r16(m + 24), g.r16(m + 26), g.r16(m + 32) as i16, g.r16(m + 34) as i16, g.r32(m + 36), g.r32(m + 40)))
}

#[test]
fn a_press_a_move_and_a_release_in_content_are_mousebuttons_and_mousemove_with_position_qualifier_and_time() {
    let mut g = G::new_plane();
    let (win, native) = open(&mut g, 0x8 | 0x10, 0x200); // MOUSEBUTTONS | MOUSEMOVE, WFLG_REPORTMOUSE
    g.a.clock_us = 2_500_000;
    assert!(matches!(g.a.deliver(GuestUiEvent::primary(native, 33, 44, PointerPhase::Down)).unwrap(), Delivered::Queued(_)));
    assert_eq!((g.r16(win + 14), g.r16(win + 12)), (33, 44), "Window.MouseX / MouseY follow the pointer");
    g.a.clock_us = 2_750_000;
    g.a.deliver(GuestUiEvent::primary(native, 40, 50, PointerPhase::Move)).unwrap();
    g.a.deliver(GuestUiEvent::primary(native, 40, 50, PointerPhase::Up)).unwrap();
    assert_eq!(message(&mut g, win), Some((0x8, 0x68, 0x4000, 33, 44, 2, 500_000)), "SELECTDOWN, left button held, at the press time");
    assert_eq!(message(&mut g, win), Some((0x10, 0, 0x4000, 40, 50, 2, 750_000)), "MOUSEMOVE while the button is held");
    assert_eq!(message(&mut g, win), Some((0x8, 0xE8, 0, 40, 50, 2, 750_000)), "SELECTUP, nothing held");
    assert_eq!(message(&mut g, win), None, "and nothing else");
    // A middle button; the right button belongs to the menu system unless the window traps it (WFLG_RMBTRAP).
    let mid = GuestUiEvent::Pointer { win: native, x: 1, y: 2, button: PointerButton::Middle, phase: PointerPhase::Down, clicks: 1 };
    g.a.deliver(mid).unwrap();
    assert_eq!(message(&mut g, win).map(|m| (m.0, m.1)), Some((0x8, 0x6A)));
    let right = |phase| GuestUiEvent::Pointer { win: native, x: 1, y: 2, button: PointerButton::Secondary, phase, clicks: 1 };
    assert_eq!(g.a.deliver(right(PointerPhase::Down)).unwrap(), Delivered::Handled);
    assert_eq!(message(&mut g, win), None, "no RMBTRAP: Intuition keeps the menu button");
    let fl = g.r32(win + 24);
    g.a.mem.write_u32(win + 24, fl | 0x1_0000).unwrap();
    g.a.deliver(right(PointerPhase::Down)).unwrap();
    assert_eq!(message(&mut g, win).map(|m| (m.0, m.1)), Some((0x8, 0x69)), "RMBTRAP: MENUDOWN reaches the program");
    // NEGATIVE CONTROLS: a move without WFLG_REPORTMOUSE; a window that did not ask for the class.
    g.a.mem.write_u32(win + 24, fl & !0x200).unwrap();
    g.a.deliver(GuestUiEvent::primary(native, 5, 5, PointerPhase::Move)).unwrap();
    assert_eq!(message(&mut g, win), None, "no REPORTMOUSE: Intuition sends no MOUSEMOVE");
    g.a.mem.write_u32(win + 82, 0x10).unwrap(); // IDCMPFlags: MOUSEMOVE only
    assert_eq!(g.a.deliver(GuestUiEvent::primary(native, 5, 5, PointerPhase::Down)).unwrap(), Delivered::IgnoredUnsubscribed);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

#[test]
fn a_typed_key_is_vanillakey_alone_and_falls_back_to_rawkey_the_way_intuition_routes_it() {
    let mut g = G::new_plane();
    let (win, native) = open(&mut g, 0x400 | 0x20_0000, 0); // RAWKEY | VANILLAKEY
    let key = |code, text: &str, m, down| GuestUiEvent::Key { win: native, code, text: text.into(), modifiers: Modifiers(m), down };
    g.a.deliver(GuestUiEvent::primary(native, 12, 34, PointerPhase::Move)).unwrap(); // Window.MouseX / MouseY follow the pointer
    g.a.deliver(key(0x20, "a", Modifiers::SHIFT, true)).unwrap();
    g.a.deliver(key(0x20, "", Modifiers::SHIFT, false)).unwrap();
    let full = |g: &mut G| { let m = message(g, win); m.map(|m| (m.0, m.1, m.2, m.3, m.4)) };
    assert_eq!(full(&mut g), Some((0x20_0000, 0x61, 1, 12, 34)), "VANILLAKEY: the character, LSHIFT, stamped with the window's MouseX/MouseY");
    assert_eq!(full(&mut g), Some((0x400, 0xA0, 1, 12, 34)), "the release is RAWKEY (raw code + 0x80): a release types nothing");
    assert_eq!(message(&mut g, win), None, "and the press sent NO RAWKEY after its VANILLAKEY");
    // A key that types no character is RAWKEY even with VANILLAKEY on.
    g.a.deliver(key(0x4F, "", 0, true)).unwrap();
    assert_eq!(message(&mut g, win).map(|m| (m.0, m.1)), Some((0x400, 0x4F)));
    // NEGATIVE CONTROLS: a window without VANILLAKEY gets the same press as RAWKEY; one with neither ignores it.
    g.a.mem.write_u32(win + 82, 0x400).unwrap();
    g.a.deliver(key(0x20, "a", 0, true)).unwrap();
    assert_eq!(message(&mut g, win).map(|m| (m.0, m.1)), Some((0x400, 0x20)));
    g.a.mem.write_u32(win + 82, 0x10).unwrap();
    assert_eq!(g.a.deliver(key(0x20, "a", 0, true)).unwrap(), Delivered::IgnoredUnsubscribed);
    // A host key code that is not an Amiga raw code is refused by name BEFORE anything is queued, even when a character was typed.
    g.a.mem.write_u32(win + 82, 0x400 | 0x20_0000).unwrap();
    let e = g.a.deliver(key(0x1_0041, "", 0, true)).unwrap_err();
    assert!(e.0.contains("no Amiga raw key code"), "{e:?}");
    assert_eq!(message(&mut g, win), None, "a refused key queued nothing");
}

/// The REAL amimon (unmodified GadTools program) through the v2 seam: its eight controls are native roles, its window needs no content plane
/// (and no canvas), the route census says so, and the intents the GadTools table has no row for are refused by name.
#[test]
fn amimon_runs_on_the_v2_seam_native_controls_a_plane_for_the_rest_and_named_refusals() {
    use amiga68k_engine::{Amiga, Step};
    let file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/amimon.020")).unwrap();
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.stop_on_unimplemented = true;
    a.start_hunk(&file).unwrap();
    loop {
        match a.step(1_000_000).0 {
            Step::Blocked => break,
            Step::BudgetSpent => {}
            other => panic!("{other:?}: {:?}", a.refused),
        }
    }
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let (&win, &native) = a.windows.iter().next().expect("its window");
    let controls: Vec<(UiObjectRef, CallbackToken, ControlKind)> = a.ui.controls.iter().filter(|(_, (w, _))| *w == native)
        .map(|(&id, (_, m))| (UiObjectRef { id, generation: 1 }, m.token, m.kind.clone())).collect();
    assert_eq!(controls.len(), 8);
    let census = a.ui.route_census(native);
    assert_eq!(census.native, 8, "{census:?}");
    assert_eq!((census.replayed, census.raw), (0, 0), "amimon draws nothing of its own: every pixel is a kit part, no plane at all: {census:?}");
    assert_eq!(a.ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. })).count(), 0);
    // A label is read-only: no row, so every intent is refused by name and nothing is queued.
    let (label, token, kind) = controls.iter().find(|(_, _, k)| matches!(k, ControlKind::Label { .. })).expect("a label");
    assert!(matches!(kind, ControlKind::Label { .. }));
    let e = a.deliver(GuestUiEvent::intent(native, *label, *token, Intent::Activate)).unwrap_err();
    assert!(e.0.contains("means nothing for the control"), "{e:?}");
    // A reused or forged generation is refused before any table is consulted.
    let stale = GuestUiEvent::Intent { win: native, control: *label, token: *token, generation: 2, intent: Intent::Activate };
    assert!(a.deliver(stale).unwrap_err().0.contains("stale"));
    assert!(a.engine_msgs.is_empty(), "{win:#x}: none of that reached the guest");
}
