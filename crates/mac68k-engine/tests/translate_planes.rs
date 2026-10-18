//! QuickDraw content planes and plain input (ruling 17, pieces 5 and 8): the display list a window's port records next to its
//! authoritative bitmap, and the pointer/keyboard events a plane receives.
mod common;
use common::{put, pstr, set_rect, Prog, Rig, WIN_LEFT, WIN_TOP};
use guestui_abi::{DrawOp, GuestRect, GuestUiEvent, Modifiers, PenRole, PointerButton, PointerPhase, RecordingGuestUi, UiCall, UiObjectRef};
use mac68k_engine::fixture::{oracle_app_macbinary, ORACLE_H, ORACLE_W};
use mac68k_engine::lowmem::{MB_STATE, MOUSE};
use mac68k_engine::{Delivered, Mac};

const R: u32 = 0x3100;
const S: u32 = 0x3200;

fn native(g: &Rig) -> UiObjectRef { g.m.windows[&g.win].native.unwrap() }
fn plane(m: &Mac<RecordingGuestUi>, win: UiObjectRef) -> Vec<DrawOp> { m.ui.planes[&(win.id, 0)].ops.clone() }

/// Replay a display list onto a white bitmap the way a host with the kit would for the roles that name a pixel exactly
/// (Text = black, Surface = white, Selection = invert, Pixels = themselves); anything the list leaves to the host's own
/// rendering (half-tones, lines, text) makes its pixels UNKNOWN (`None`).
fn replay(ops: &[DrawOp], w: i32, h: i32) -> Vec<Option<bool>> {
    let mut v = vec![Some(false); (w * h) as usize];
    let mut each = |r: &GuestRect, f: &mut dyn FnMut(&mut Option<bool>)| {
        for y in r.top.max(0)..(r.top + r.height).min(h) { for x in r.left.max(0)..(r.left + r.width).min(w) { f(&mut v[(y * w + x) as usize]); } }
    };
    for op in ops {
        match op {
            DrawOp::Fill { rect, pen_role: PenRole::Text } => each(rect, &mut |p| *p = Some(true)),
            DrawOp::Fill { rect, pen_role: PenRole::Surface } => each(rect, &mut |p| *p = Some(false)),
            DrawOp::Fill { rect, pen_role: PenRole::Selection } => each(rect, &mut |p| *p = p.map(|b| !b)),
            DrawOp::Fill { rect, .. } => each(rect, &mut |p| *p = None),
            DrawOp::Pixels { rect, rgb } => { let mut i = 0; each(rect, &mut |p| { *p = Some(rgb[i] == 0); i += 1; }) }
            DrawOp::Line { a, b, .. } => each(&GuestRect { left: a.x.min(b.x), top: a.y.min(b.y), width: (a.x - b.x).abs() + 1, height: (a.y - b.y).abs() + 1 }, &mut |p| *p = None),
            DrawOp::Text { at, string, px, bg_role, .. } => {
                let r = GuestRect { left: at.x, top: at.y, width: 8 * string.len() as i32, height: i32::from(*px) };
                if bg_role.is_some() { each(&r, &mut |p| *p = None) } else { each(&r, &mut |p| if *p != Some(true) { *p = None }) }
            }
            other => panic!("unexpected op {other:?}"),
        }
    }
    v
}

#[test]
fn the_oracle_apps_window_is_a_display_list_that_agrees_with_its_bitmap_wherever_it_names_pixels() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.launch(&oracle_app_macbinary()).unwrap();
    let r = m.run_loop(50_000_000).unwrap();
    assert!(r.stats.blocked && m.refused.is_empty(), "{:?} {:?}", r.stats, m.refused);
    let (win, canvas) = (m.windows.values().next().unwrap().native.unwrap(), m.qd.canvases.values().next().unwrap());
    assert!(!m.ui.calls.iter().any(|c| matches!(c, UiCall::PresentContent { .. })), "the plane route replaces the canvas route");
    let ops = plane(&m, win);
    // the four DrawString slots are text runs with their font role and point size, not glyph pixels
    let texts: Vec<(&str, u16)> = ops.iter().filter_map(|o| if let DrawOp::Text { string, px, .. } = o { Some((string.as_str(), *px)) } else { None }).collect();
    assert_eq!(texts.len(), 4, "{texts:?}");
    assert_eq!((texts[0], texts[3].1), (("Hello", 12), 9));
    // CopyBits (row 4) has no role: it is recorded as the pixels it drew, so the census says one raw plane
    assert!(ops.iter().any(DrawOp::is_raw));
    let c = m.ui.route_census(win);
    assert_eq!((c.native, c.replayed, c.raw), (0, 0, 1), "{c:?}");
    // The list agrees with the bitmap on every pixel it names, and names most of them
    let (w, h) = (i32::from(ORACLE_W), i32::from(ORACLE_H));
    let rep = replay(&ops, w, h);
    let known = rep.iter().filter(|p| p.is_some()).count();
    let wrong: Vec<usize> = rep.iter().enumerate().filter(|(i, p)| p.is_some_and(|b| b != (canvas.pixels()[*i] == 1))).map(|(i, _)| i).collect();
    assert!(wrong.is_empty(), "{} pixels disagree, first at ({}, {})", wrong.len(), wrong[0] as i32 % w, wrong[0] as i32 / w);
    assert!(known * 100 / rep.len() >= 60, "only {known} of {} pixels are named by the list", rep.len());
    // NEGATIVE control: a corrupted list is caught by the same comparison
    let mut bad = ops.clone();
    let i = bad.iter().position(|o| matches!(o, DrawOp::Fill { pen_role: PenRole::Text, .. })).unwrap();
    if let DrawOp::Fill { pen_role, .. } = &mut bad[i] { *pen_role = PenRole::Surface; }
    let rep = replay(&bad, w, h);
    assert!(rep.iter().enumerate().any(|(i, p)| p.is_some_and(|b| b != (canvas.pixels()[i] == 1))), "the comparison can fail");
}

#[test]
fn a_drawn_button_shaped_rect_is_not_a_role() {
    // A rounded frame with "OK" inside looks exactly like a push button. Nothing in the Toolbox said so: it stays a plane.
    let mut g = Rig::new();
    set_rect(&mut g.m, R, 10, 10, 34, 90);
    put(&mut g.m, S, &pstr("OK"));
    let rr = mac68k_engine::mixed::trap_opcode("FrameRoundRect").unwrap();
    g.run(Prog::new().l(R).w(12).w(12).trap(rr).move_to(40, 28).draw_string(S).exit());
    g.no_refusals();
    assert!(!g.m.ui.calls.iter().any(|c| matches!(c, UiCall::AddControl(..))), "no control was created from a drawing");
    let c = g.m.ui.route_census(native(&g));
    assert_eq!((c.native, c.replayed, c.raw), (0, 1, 0), "one replayed plane, nothing native, nothing raw");
    let ops = plane(&g.m, native(&g));
    assert!(ops.iter().any(|o| matches!(o, DrawOp::Text { string, .. } if string == "OK")));
    // positive control: the same look made by the Control Manager IS a control
    let mut g = Rig::new();
    put(&mut g.m, S, &pstr("OK"));
    set_rect(&mut g.m, R, 10, 10, 34, 90);
    let win = g.win;
    g.run(Prog::new().clr_l().l(win).l(R).l(S).b(1).w(0).w(0).w(1).w(0).l(0).trap(0xA954).pop_l(0x3000).exit());
    assert_eq!(g.m.ui.route_census(native(&g)).native, 1);
}

#[test]
fn the_list_stays_bounded_without_a_cap_and_highlights_cancel() {
    let mut g = Rig::new();
    set_rect(&mut g.m, R, 0, 0, 120, 200);
    set_rect(&mut g.m, R + 8, 20, 20, 36, 60);
    let mut p = Prog::new();
    for _ in 0..300 { p.erase_rect(R).paint_rect(R + 8); } // a redraw loop: erase everything, draw
    g.run(&p.exit());
    assert_eq!(plane(&g.m, native(&g)).len(), 2, "every redraw's EraseRect covered the previous frame");
    // InvertRect over plain surface is a highlight; inverting again removes it
    let mut g = Rig::new();
    set_rect(&mut g.m, R, 10, 10, 30, 50);
    g.run(Prog::new().invert_rect(R).exit());
    assert!(matches!(plane(&g.m, native(&g)).as_slice(), [DrawOp::Fill { pen_role: PenRole::Selection, .. }]));
    g.run(Prog::new().invert_rect(R).exit());
    assert!(plane(&g.m, native(&g)).is_empty(), "inverted twice: nothing");
    assert_eq!(g.count_black(), 0, "and the bitmap agrees");
    // inverting something that is not plain surface is not a highlight: the pixels, exactly
    let mut g = Rig::new();
    set_rect(&mut g.m, R, 10, 10, 30, 50);
    g.run(Prog::new().paint_rect(R).invert_rect(R).exit());
    assert!(!plane(&g.m, native(&g)).iter().any(|o| matches!(o, DrawOp::Fill { pen_role: PenRole::Selection, .. })));
    assert!(plane(&g.m, native(&g)).iter().any(DrawOp::is_raw));
}

#[test]
fn a_host_without_plane_replay_gets_the_canvas_route() {
    struct NoPlane(RecordingGuestUi);
    impl guestui_abi::GuestUi for NoPlane {
        fn open_window(&mut self, m: guestui_abi::WindowModel) -> Result<UiObjectRef, guestui_abi::UiRefusal> { self.0.open_window(m) }
        fn close_window(&mut self, w: UiObjectRef) -> Result<(), guestui_abi::UiRefusal> { self.0.close_window(w) }
        fn set_window_title(&mut self, w: UiObjectRef, t: String) -> Result<(), guestui_abi::UiRefusal> { self.0.set_window_title(w, t) }
        fn set_menu_strip(&mut self, w: UiObjectRef, m: guestui_abi::MenuModel) -> Result<(), guestui_abi::UiRefusal> { self.0.set_menu_strip(w, m) }
        fn clear_menu_strip(&mut self, w: UiObjectRef) -> Result<(), guestui_abi::UiRefusal> { self.0.clear_menu_strip(w) }
        fn add_control(&mut self, w: UiObjectRef, c: guestui_abi::ControlModel) -> Result<UiObjectRef, guestui_abi::UiRefusal> { self.0.add_control(w, c) }
        fn remove_control(&mut self, w: UiObjectRef, c: UiObjectRef) -> Result<(), guestui_abi::UiRefusal> { self.0.remove_control(w, c) }
        fn set_control(&mut self, w: UiObjectRef, c: UiObjectRef, v: guestui_abi::ControlValue) -> Result<(), guestui_abi::UiRefusal> { self.0.set_control(w, c, v) }
        fn present_content(&mut self, w: UiObjectRef, d: GuestRect, cw: i32, ch: i32, px: &[u32]) -> Result<(), guestui_abi::UiRefusal> { self.0.present_content(w, d, cw, ch, px) }
        fn poll_event(&mut self) -> Option<GuestUiEvent> { None }
    }
    let mut m = Mac::new(NoPlane(RecordingGuestUi::new())).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, common::GLOBALS).unwrap();
    m.qd_globals = common::GLOBALS;
    let spec = mac68k_engine::toolbox::WindowSpec { storage: 0, rect: mac68k_engine::pascal::Rect { top: 100, left: 50, bottom: 200, right: 150 }, title: "n".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: false, ref_con: 0 };
    m.make_window(spec).unwrap();
    m.mem.write_bytes(R, &[0, 0, 0, 0, 0, 10, 0, 10]).unwrap();
    m.start(&Prog::new().paint_rect(R).exit().0, 0).unwrap();
    m.run_loop(100_000).unwrap();
    assert!(m.qd.plane.unsupported, "refused by name once, then the canvas route");
    assert!(m.ui.0.calls.iter().any(|c| matches!(c, UiCall::PresentContent { .. })), "the canvas was presented instead");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

// ---------------------------------------------------------------- plain input

fn queued(g: &Rig) -> Vec<(u16, u32, i16, i16, u16)> { g.m.ev.queue.iter().map(|e| (e.what, e.message, e.v, e.h, e.modifiers)).collect() }

#[test]
fn pointer_and_keys_become_the_events_a_mac_program_handles() {
    let mut g = Rig::new();
    let win = native(&g);
    let mouse = |g: &Rig, o: u32| g.m.mem.read_u16(MOUSE + o).unwrap() as i16;
    let down = |g: &mut Rig, x, y, b| g.m.deliver(GuestUiEvent::Pointer { win, x, y, button: b, phase: PointerPhase::Down, clicks: 1 });
    // window coordinates + the content origin = global
    assert_eq!(down(&mut g, 30, 40, PointerButton::Primary), Ok(Delivered::Queued));
    assert_eq!(queued(&g), vec![(1, 0, 40 + WIN_TOP, 30 + WIN_LEFT, 0)]);
    assert_eq!(g.m.mem.read_u8(MB_STATE).unwrap(), 0, "button down");
    g.m.deliver(GuestUiEvent::primary(win, 31, 41, PointerPhase::Up)).unwrap();
    assert_eq!(queued(&g)[1], (2, 0, 41 + WIN_TOP, 31 + WIN_LEFT, 0));
    assert_eq!(g.m.mem.read_u8(MB_STATE).unwrap(), 0x80, "button up");
    // motion makes no event but moves the polled Mouse
    g.m.ev.queue.clear();
    assert_eq!(g.m.deliver(GuestUiEvent::primary(win, 5, 6, PointerPhase::Move)), Ok(Delivered::Ignored));
    assert!(g.m.ev.queue.is_empty());
    assert_eq!((mouse(&g, 0), mouse(&g, 2)), (6 + WIN_TOP, 5 + WIN_LEFT));
    // the secondary button is a control-click; the middle button has no classic meaning
    down(&mut g, 1, 1, PointerButton::Secondary).unwrap();
    assert_eq!(queued(&g)[0].4, 0x1000, "controlKey");
    assert!(down(&mut g, 1, 1, PointerButton::Middle).unwrap_err().0.contains("no classic Mac event"));
    assert_eq!(g.m.ev.queue.len(), 1, "the refused one queued nothing");
    // a pointer for a window the guest does not own is not for it
    let other = UiObjectRef { id: 99, generation: 1 };
    assert_eq!(g.m.deliver(GuestUiEvent::primary(other, 1, 1, PointerPhase::Down)), Ok(Delivered::NotForAGuestWindow));

    // keys: keyDown with the character, autoKey for the same key held, keyUp nothing, modifiers mapped
    g.m.ev.queue.clear();
    let key = |g: &mut Rig, code, text: &str, m: u8, d| g.m.deliver(GuestUiEvent::Key { win, code, text: text.into(), modifiers: Modifiers(m), down: d });
    assert_eq!(key(&mut g, 38, "a", Modifiers::SHIFT | Modifiers::META, true), Ok(Delivered::Queued));
    key(&mut g, 38, "a", 0, true).unwrap();
    assert_eq!(key(&mut g, 38, "a", 0, false), Ok(Delivered::Ignored));
    key(&mut g, 38, "a", 0, true).unwrap();
    assert_eq!(queued(&g).iter().map(|e| (e.0, e.1, e.4)).collect::<Vec<_>>(), vec![(3, 0x61, 0x0300), (5, 0x61, 0), (3, 0x61, 0)]);
    key(&mut g, 36, "\n", 0, true).unwrap();
    assert_eq!(queued(&g).last().unwrap().1, 0x0D, "Return is CR");
    // named refusals: a key without a character, a character outside ASCII
    assert!(key(&mut g, 123, "", 0, true).unwrap_err().0.contains("no character"));
    assert!(key(&mut g, 200, "é", 0, true).unwrap_err().0.contains("outside ASCII"));
    assert_eq!(g.m.ev.queue.len(), 4);
}

#[test]
fn inverting_twice_around_other_drawing_is_not_an_undo_of_the_list() {
    // InvertRect, then text drawn inside, then InvertRect again: the bitmap has changed in between, so the second invert
    // must not just delete the first highlight (the list would then still show text the bitmap no longer has).
    let mut g = Rig::new();
    set_rect(&mut g.m, R, 10, 10, 30, 50);
    put(&mut g.m, S, &pstr("OK"));
    g.run(Prog::new().invert_rect(R).move_to(14, 24).draw_string(S).invert_rect(R).exit());
    g.no_refusals();
    let ops = plane(&g.m, native(&g));
    let canvas = &g.m.qd.canvases[&g.win];
    let (w, h) = (i32::from(canvas.rect().x1), i32::from(canvas.rect().y1));
    let rep = replay(&ops, w, h);
    let wrong = rep.iter().enumerate().filter(|(i, p)| p.is_some_and(|b| b != (canvas.pixels()[*i] == 1))).count();
    assert_eq!(wrong, 0, "the list disagrees with the bitmap on {wrong} pixels: {ops:?}");
    assert!(!ops.iter().any(|o| matches!(o, DrawOp::Text { .. })), "the text was inverted away: {ops:?}");
    // POSITIVE control: nothing drawn in between keeps the cheap undo (an empty list)
    let mut g = Rig::new();
    set_rect(&mut g.m, R, 10, 10, 30, 50);
    g.run(Prog::new().invert_rect(R).invert_rect(R).exit());
    assert!(plane(&g.m, native(&g)).is_empty());
}
