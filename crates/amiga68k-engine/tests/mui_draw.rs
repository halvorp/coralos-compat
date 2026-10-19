//! A MUI class that draws itself (tests/guest/c/muidraw.c, a custom class over Area.mui written like Zune's own): MUI sends it
//! Setup, AskMinMax, Show and Draw in Zune's order when its window opens, Hide and Cleanup when it closes; it reads its place in the
//! window from the object's own memory (the plain offsets Zune's `_left(obj)` ... macros are) and draws through a real RastPort, which the
//! engine records as a replayed PLANE in the object's rectangle (window coordinates: the box the guest read): no native control, no copied pixels.
use amiga68k_engine::{Amiga, Step};
use guestui_abi::{DrawOp, GuestRect, RecordingGuestUi, UiCall};
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/muidraw.bin");
const RES: u32 = 0x1F000;

fn run(mode: u32) -> (Amiga<RecordingGuestUi>, Step) {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.mem.write_u32(RES + 4 * 63, mode).unwrap();
    a.start_hunk(&build(&[Hunk::code(BIN.to_vec(), vec![])], None)).unwrap();
    let (s, _) = a.step(5_000_000);
    (a, s)
}
fn r(a: &Amiga<RecordingGuestUi>, i: u32) -> u32 { a.mem.read_u32(RES + 4 * i).unwrap() }
fn planes(a: &Amiga<RecordingGuestUi>) -> Vec<guestui_abi::Plane> { a.ui.calls.iter().filter_map(|c| match c { UiCall::PresentPlane(_, p) => Some(p.clone()), _ => None }).collect() }

#[test]
fn a_self_drawing_class_gets_zunes_methods_a_real_box_and_rastport_and_becomes_a_replayed_plane() {
    let (a, s) = run(0);
    assert_eq!(s, Step::Blocked, "{:?} {:?}", a.refused, a.log.iter().rev().take(5).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!((0..5).all(|i| r(&a, i) != 0) && r(&a, 17) == 1, "class, object, application built; the window opened inside SetAttrs");
    // The methods, once each, in the order the window sends them; nothing was hidden or cleaned up yet.
    assert_eq!([5, 6, 7].map(|i| r(&a, i)), [1, 1, 1], "Setup, AskMinMax, Show");
    assert!(r(&a, 8) >= 1, "Draw");
    assert_eq!([r(&a, 9), r(&a, 10)], [0, 0], "Hide / Cleanup are for closing");
    let order: Vec<&String> = a.log.iter().filter(|l| l.starts_with("[muimaster] ") && (l.contains("Setup(") || l.contains("AskMinMax(") || l.contains("Open(") || l.contains("Show(") || l.contains("Draw("))).collect();
    let names: Vec<&str> = order.iter().map(|l| l.trim_start_matches("[muimaster] ").split('(').next().unwrap()).collect();
    assert_eq!(names, ["Setup", "AskMinMax", "Open", "Show", "Draw"], "Zune's order: {order:?}");
    // The window is as big as the class's AskMinMax default (80x40) plus MUI's window padding and the native frame; the guest read its box
    // out of its own object memory: window coordinates (borders included).
    let win = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.clone()), _ => None }).unwrap();
    assert_eq!((win.title.as_str(), win.rect.width, win.rect.height), ("Plane", 80 + 8 + 8, 40 + 6 + 19), "{win:?}");
    assert_eq!([11, 12, 13, 14].map(|i| r(&a, i)), [4 + 4, 3 + 15, 80, 40], "_left, _top, _width, _height as the guest sees them");
    assert_eq!(r(&a, 15), 1, "mri_Screen is set at Setup");
    assert_ne!(r(&a, 16), 0, "mri_RastPort is set at Show");
    // No native control: the object has no role of its own. Its drawing is a plane, replayed from ops in the object's rectangle.
    assert!(a.ui.controls.is_empty(), "{:?}", a.ui.controls);
    let p = planes(&a);
    assert_eq!(p.len(), 1, "{p:?}");
    let ops = &p[0].ops;
    assert!(ops.iter().any(|o| matches!(o, DrawOp::Fill { rect, .. } if *rect == GuestRect { left: 8, top: 18, width: 80, height: 40 })), "the RectFill over the whole box: {ops:?}");
    assert!(ops.iter().any(|o| matches!(o, DrawOp::Line { .. })), "the diagonal: {ops:?}");
    assert!(ops.iter().any(|o| matches!(o, DrawOp::Text { string, .. } if string == "Plane")), "the text run: {ops:?}");
    assert!(ops.iter().all(|o| match o {
        DrawOp::Fill { rect, .. } | DrawOp::Bevel { rect, .. } => rect.left >= 8 && rect.top >= 18 && rect.left + rect.width <= 88 && rect.top + rect.height <= 58,
        _ => true,
    }), "nothing outside the object's rectangle: {ops:?}");
}

#[test]
fn closing_the_window_hides_and_cleans_up_the_object_and_disposing_the_application_frees_everything() {
    let (a, s) = run(1);
    assert_eq!(s, Step::Exited, "{:?} {:?}", a.refused, a.log.iter().rev().take(5).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!([r(&a, 5), r(&a, 7), r(&a, 9), r(&a, 10)], [1, 1, 1, 1], "Setup/Show once, then Hide and Cleanup when the window closed");
    assert_eq!(r(&a, 18), 1, "the program got past the close and the dispose");
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::CloseWindow(_))), "the native window closed");
    assert!(a.objects.map.is_empty() && a.objects.drawn.is_empty() && a.objects.surfaces.is_empty() && a.windows.is_empty(), "no object, record, surface or struct Window left");
}

#[test]
fn a_window_that_never_opens_sends_its_objects_nothing_and_draws_nothing() {
    // NEGATIVE CONTROL: the same tree, never opened: no method reached the class, no window, no plane.
    let (a, s) = run(2);
    assert_eq!(s, Step::Exited, "{:?}", a.refused);
    assert_eq!([5, 6, 7, 8, 9, 10].map(|i| r(&a, i)), [0; 6]);
    assert!(a.ui.calls.iter().all(|c| !matches!(c, UiCall::OpenWindow(_) | UiCall::PresentPlane(..))), "{:?}", a.ui.calls);
}

#[test]
fn a_redraw_and_a_resize_draw_the_object_again_in_its_new_box_and_a_missing_redraw_leaves_it_alone() {
    let (mut a, s) = run(0);
    assert_eq!(s, Step::Blocked);
    let before = (r(&a, 8), planes(&a).len());
    // The host resizes the window 30 px wider: the object stretches (its AskMinMax allows MUI_MAXMAX), so layout gives it a new box and
    // MUI sends Draw again. The guest reads the new width from its own memory and the plane is replaced for the new rectangle.
    let w = a.ui.calls.iter().find_map(|c| match c { UiCall::PresentPlane(w, _) => Some(*w), _ => None }).expect("the plane's window");
    let rect = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.rect), _ => None }).unwrap();
    a.deliver(guestui_abi::GuestUiEvent::Resized { win: w, rect: GuestRect { width: rect.width + 30, ..rect } }).unwrap();
    assert_eq!(a.step(5_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert_eq!((r(&a, 8), r(&a, 13)), (before.0 + 1, 110), "one more Draw, and _width is now 80 + 30");
    let p = planes(&a);
    assert!(p.len() > before.1 && p.last().unwrap().ops.iter().any(|o| matches!(o, DrawOp::Fill { rect, .. } if rect.width == 110)), "{:?}", p.last());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    // MUI_Redraw from the program (mode 3) draws once more; NEGATIVE CONTROL: mode 0 never calls it, and its Draw count stays at the one the open made.
    let (b, s) = run(3);
    assert_eq!(s, Step::Blocked, "{:?}", b.refused);
    assert_eq!((r(&b, 20), r(&b, 8)), (1, 2), "MUI_Redraw returned, and the object was drawn a second time");
    let (c, _) = run(0);
    assert_eq!(r(&c, 8), 1, "without MUI_Redraw the open's Draw is the only one");
}
