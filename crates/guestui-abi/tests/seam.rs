//! The seam's contract (ruling 17, pieces 1, 2 and 9): control generations, role-level intents, display-list planes, the
//! route census. Every behaviour has its negative control next to it.
use guestui_abi::*;

fn r(l: i32, t: i32, w: i32, h: i32) -> GuestRect { GuestRect { left: l, top: t, width: w, height: h } }
fn win() -> WindowModel {
    WindowModel { title: "T".into(), rect: r(0, 0, 100, 100), role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(1) }
}
fn ctl(kind: ControlKind, token: u64) -> ControlModel {
    ControlModel { kind, label: "x".into(), rect: r(1, 1, 5, 5), enabled: true, token: CallbackToken(token) }
}

#[test]
fn a_removed_control_is_stale_for_every_mutation_and_for_intents() {
    let mut ui = RecordingGuestUi::new();
    let w = ui.open_window(win()).unwrap();
    let c = ui.add_control(w, ctl(ControlKind::Checkbox { checked: false }, 7)).unwrap();
    // Control: while live, everything works.
    ui.set_control(w, c, ControlValue::Checked(true)).unwrap();
    ui.set_control_rect(w, c, r(2, 2, 6, 6)).unwrap();
    ui.set_control_enabled(w, c, false).unwrap();
    ui.push_intent(w, c, Intent::SetValue(ControlValue::Checked(false))).unwrap();
    ui.remove_control(w, c).unwrap();
    // After removal: each is refused BY NAME (StaleGeneration), changes nothing and records nothing.
    let n = ui.calls.len();
    let stale = Err(UiRefusal::StaleGeneration(c));
    assert_eq!(ui.set_control(w, c, ControlValue::Checked(true)), stale);
    assert_eq!(ui.set_control_rect(w, c, r(0, 0, 1, 1)), stale);
    assert_eq!(ui.set_control_enabled(w, c, true), stale);
    assert_eq!(ui.remove_control(w, c), stale);
    assert_eq!(ui.push_intent(w, c, Intent::Activate), stale);
    assert_eq!(ui.calls.len(), n);
    // A ref never issued is Unknown, not Stale: the two refusals mean different things.
    let never = UiObjectRef { id: 999, generation: 1 };
    assert_eq!(ui.set_control(w, never, ControlValue::Pressed), Err(UiRefusal::UnknownObject(never)));
}

#[test]
fn a_reused_token_gets_a_new_generation_and_the_old_ref_stays_stale() {
    let mut ui = RecordingGuestUi::new();
    let w = ui.open_window(win()).unwrap();
    let first = ui.add_control(w, ctl(ControlKind::Button, 9)).unwrap();
    let other = ui.add_control(w, ctl(ControlKind::Button, 10)).unwrap();
    assert_eq!((first.generation, other.generation), (1, 1), "different tokens start at generation 1");
    ui.remove_control(w, first).unwrap();
    let second = ui.add_control(w, ctl(ControlKind::Button, 9)).unwrap();
    assert_eq!(second.generation, 2, "the same token, a new incarnation");
    assert_ne!(first, second);
    // The old ref, and the old ref re-stamped with the new generation, are both refused; the new ref works.
    assert_eq!(ui.set_control(w, first, ControlValue::Pressed), Err(UiRefusal::StaleGeneration(first)));
    let forged = UiObjectRef { id: first.id, generation: second.generation };
    assert_eq!(ui.set_control(w, forged, ControlValue::Pressed), Err(UiRefusal::StaleGeneration(forged)));
    ui.set_control(w, second, ControlValue::Pressed).unwrap();
    // The wrong generation for a LIVE id is stale too.
    let wrong = UiObjectRef { id: second.id, generation: 1 };
    assert_eq!(ui.set_control(w, wrong, ControlValue::Pressed), Err(UiRefusal::StaleGeneration(wrong)));
}

#[test]
fn closing_a_window_retires_its_controls_and_planes() {
    let mut ui = RecordingGuestUi::new();
    let w = ui.open_window(win()).unwrap();
    let c = ui.add_control(w, ctl(ControlKind::Button, 3)).unwrap();
    ui.present_plane(w, Plane { id: 1, rect: r(0, 0, 10, 10), ops: vec![DrawOp::Fill { rect: r(0, 0, 10, 10), pen_role: PenRole::Surface }] }).unwrap();
    ui.close_window(w).unwrap();
    assert_eq!(ui.set_control(w, c, ControlValue::Pressed), Err(UiRefusal::UnknownObject(w)), "the window is gone");
    let w2 = ui.open_window(WindowModel { token: CallbackToken(2), ..win() }).unwrap();
    assert_eq!(ui.set_control(w2, c, ControlValue::Pressed), Err(UiRefusal::StaleGeneration(c)), "its control was retired with it");
    assert!(ui.planes.is_empty() && ui.controls.is_empty());
    assert_eq!(ui.route_census(w), RouteCensus::default(), "a closed window holds nothing");
}

#[test]
fn intents_carry_the_token_and_generation_the_control_was_added_with() {
    let mut ui = RecordingGuestUi::new();
    let w = ui.open_window(win()).unwrap();
    let c = ui.add_control(w, ctl(ControlKind::Button, 5)).unwrap();
    for i in [Intent::Activate, Intent::Press, Intent::Release { inside: false }, Intent::SelectRow(3), Intent::ScrollTo(9),
              Intent::CommitText("hi".into()), Intent::Drag(ControlValue::Level(2)), Intent::SetValue(ControlValue::Checked(true))] {
        ui.push_intent(w, c, i.clone()).unwrap();
        assert_eq!(ui.poll_event(), Some(GuestUiEvent::Intent { win: w, control: c, token: CallbackToken(5), generation: 1, intent: i }));
    }
    // Negative control: a control of ANOTHER window is not addressable through this one.
    let w2 = ui.open_window(WindowModel { token: CallbackToken(2), ..win() }).unwrap();
    assert!(ui.push_intent(w2, c, Intent::Activate).is_err());
    assert!(ui.poll_event().is_none());
}

#[test]
fn pointer_and_key_events_are_plain_window_coordinates() {
    let w = UiObjectRef { id: 1, generation: 1 };
    let down = GuestUiEvent::primary(w, 4, 5, PointerPhase::Down);
    assert_eq!(down, GuestUiEvent::Pointer { win: w, x: 4, y: 5, button: PointerButton::Primary, phase: PointerPhase::Down, clicks: 1 });
    // Negative control: motion holds no button and counts no click.
    assert_eq!(GuestUiEvent::primary(w, 4, 5, PointerPhase::Move), GuestUiEvent::Pointer { win: w, x: 4, y: 5, button: PointerButton::None, phase: PointerPhase::Move, clicks: 0 });
    let m = Modifiers(Modifiers::SHIFT | Modifiers::META);
    assert!(m.has(Modifiers::SHIFT) && m.has(Modifiers::META) && !m.has(Modifiers::CTRL));
    let k = GuestUiEvent::Key { win: w, code: 38, text: "a".into(), modifiers: m, down: true };
    assert_ne!(k, GuestUiEvent::Key { win: w, code: 38, text: "a".into(), modifiers: m, down: false });
}

fn text(s: &str, pen: PenRole) -> DrawOp {
    DrawOp::Text { at: GuestPoint { x: 2, y: 2 }, string: s.into(), font_role: FontRole::Ui, px: 14, pen_role: pen, bg_role: None }
}

#[test]
fn planes_are_recorded_replaced_per_id_and_validated() {
    let mut ui = RecordingGuestUi::new();
    let w = ui.open_window(win()).unwrap();
    let ops = vec![
        DrawOp::Fill { rect: r(0, 0, 50, 20), pen_role: PenRole::Surface }, text("Name", PenRole::Text),
        DrawOp::Line { a: GuestPoint { x: 0, y: 19 }, b: GuestPoint { x: 49, y: 19 }, pen_role: PenRole::Shadow },
        DrawOp::Bevel { rect: r(0, 0, 50, 20), raised: true }, DrawOp::Icon { rect: r(2, 2, 16, 16), role: IconRole::Drawer },
    ];
    let p = Plane { id: 4, rect: r(0, 0, 50, 20), ops };
    ui.present_plane(w, p.clone()).unwrap();
    assert_eq!(ui.planes[&(w.id, 4)], p);
    assert_eq!(ui.calls.last(), Some(&UiCall::PresentPlane(w, p)));
    let newer = Plane { id: 4, rect: r(0, 0, 50, 20), ops: vec![text("Size", PenRole::TextMuted)] };
    ui.present_plane(w, newer.clone()).unwrap();
    assert_eq!(ui.planes.len(), 1, "a later frame replaces the earlier one for the same id");
    assert_eq!(ui.planes[&(w.id, 4)], newer);
    // Negative controls, each refused by name, none recorded: empty rect, bad pixel count, zero-size text, unknown window.
    let n = ui.calls.len();
    let bad_rect = Plane { id: 5, rect: r(0, 0, 0, 5), ops: vec![] };
    let bad_pixels = Plane { id: 5, rect: r(0, 0, 4, 4), ops: vec![DrawOp::Pixels { rect: r(0, 0, 2, 2), rgb: vec![0; 3] }] };
    let bad_text = Plane { id: 5, rect: r(0, 0, 4, 4), ops: vec![DrawOp::Text { at: GuestPoint { x: 0, y: 0 }, string: "x".into(), font_role: FontRole::Mono, px: 0, pen_role: PenRole::Text, bg_role: Some(PenRole::Background) }] };
    for bad in [bad_rect, bad_pixels, bad_text] { assert!(matches!(ui.present_plane(w, bad), Err(UiRefusal::InvalidGeometry(_)))); }
    assert!(ui.present_plane(UiObjectRef { id: 77, generation: 1 }, newer).is_err());
    assert_eq!(ui.calls.len(), n);
}

/// A host with no plane replay refuses by name (the trait default), so an engine can fall back to `present_content`.
#[test]
fn the_default_present_plane_is_a_named_refusal() {
    struct Bare;
    impl GuestUi for Bare {
        fn open_window(&mut self, _: WindowModel) -> Result<UiObjectRef, UiRefusal> { unreachable!() }
        fn close_window(&mut self, _: UiObjectRef) -> Result<(), UiRefusal> { unreachable!() }
        fn set_window_title(&mut self, _: UiObjectRef, _: String) -> Result<(), UiRefusal> { unreachable!() }
        fn set_menu_strip(&mut self, _: UiObjectRef, _: MenuModel) -> Result<(), UiRefusal> { unreachable!() }
        fn clear_menu_strip(&mut self, _: UiObjectRef) -> Result<(), UiRefusal> { unreachable!() }
        fn add_control(&mut self, _: UiObjectRef, _: ControlModel) -> Result<UiObjectRef, UiRefusal> { unreachable!() }
        fn remove_control(&mut self, _: UiObjectRef, _: UiObjectRef) -> Result<(), UiRefusal> { unreachable!() }
        fn set_control(&mut self, _: UiObjectRef, _: UiObjectRef, _: ControlValue) -> Result<(), UiRefusal> { unreachable!() }
        fn poll_event(&mut self) -> Option<GuestUiEvent> { None }
    }
    let e = Bare.present_plane(UiObjectRef { id: 1, generation: 1 }, Plane { id: 1, rect: r(0, 0, 1, 1), ops: vec![] });
    assert!(matches!(e, Err(UiRefusal::Unsupported(w)) if w.contains("planes")));
}

#[test]
fn the_route_census_counts_what_the_host_holds() {
    let mut ui = RecordingGuestUi::new();
    let w = ui.open_window(win()).unwrap();
    let w2 = ui.open_window(WindowModel { token: CallbackToken(2), ..win() }).unwrap();
    assert_eq!(ui.route_census(w), RouteCensus { native: 0, replayed: 0, raw: 0 });
    let a = ui.add_control(w, ctl(ControlKind::Button, 1)).unwrap();
    ui.add_control(w, ctl(ControlKind::Hotspot, 2)).unwrap();
    ui.add_control(w2, ctl(ControlKind::Button, 3)).unwrap();
    ui.present_plane(w, Plane { id: 1, rect: r(0, 0, 9, 9), ops: vec![text("a", PenRole::Text)] }).unwrap();
    ui.present_plane(w, Plane { id: 2, rect: r(0, 0, 2, 1), ops: vec![DrawOp::Pixels { rect: r(0, 0, 2, 1), rgb: vec![1, 2] }] }).unwrap();
    assert_eq!(ui.route_census(w), RouteCensus { native: 2, replayed: 1, raw: 1 });
    ui.present_content(w, r(0, 0, 2, 1), 2, 1, &[1, 2]).unwrap();
    assert_eq!(ui.route_census(w), RouteCensus { native: 2, replayed: 1, raw: 2 }, "a guest-pixel canvas is a raw unit");
    // Negative controls: the other window's census is its own; removing a control and re-sending a plane without pixels move the counts.
    assert_eq!(ui.route_census(w2), RouteCensus { native: 1, replayed: 0, raw: 0 });
    ui.remove_control(w, a).unwrap();
    ui.present_plane(w, Plane { id: 2, rect: r(0, 0, 2, 1), ops: vec![text("b", PenRole::Text)] }).unwrap();
    assert_eq!(ui.route_census(w), RouteCensus { native: 1, replayed: 2, raw: 1 });
}
