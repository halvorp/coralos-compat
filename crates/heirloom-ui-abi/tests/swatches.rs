//! The Swatches role (a palette): what a host holds, what it refuses by name, and that the route census counts it native.
use heirloom_ui_abi::*;

fn model(kind: ControlKind) -> ControlModel {
    ControlModel { kind, label: String::new(), rect: GuestRect { left: 0, top: 0, width: 80, height: 20 }, enabled: true, token: CallbackToken(1) }
}
fn window(ui: &mut RecordingGuestUi) -> UiObjectRef {
    ui.open_window(WindowModel { title: "t".into(), rect: GuestRect { left: 0, top: 0, width: 200, height: 100 }, role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(0) }).unwrap()
}

#[test]
fn swatches_hold_their_selection_and_refuse_an_index_past_their_colours() {
    let mut ui = RecordingGuestUi::new();
    let w = window(&mut ui);
    let c = ui.add_control(w, model(ControlKind::Swatches { colors: vec![0x000000, 0xFF0000, 0x00FF00], selected: 1 })).unwrap();
    ui.set_control(w, c, ControlValue::Choice(2)).unwrap();
    assert!(matches!(&ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Choice(2)))));
    assert_eq!(ui.set_control(w, c, ControlValue::Choice(3)), Err(UiRefusal::InvalidGeometry("swatch index past its colours")));
    // NEGATIVE CONTROLS: the shape rule is on add too, and a value of another kind is refused by name.
    assert!(ui.add_control(w, model(ControlKind::Swatches { colors: vec![], selected: 0 })).is_err(), "a palette needs a colour");
    assert!(ui.add_control(w, model(ControlKind::Swatches { colors: vec![1, 2], selected: 2 })).is_err(), "the selection is one of the colours");
    assert!(ui.set_control(w, c, ControlValue::Level(1)).is_err());
    ui.push_intent(w, c, Intent::SetValue(ControlValue::Choice(0))).unwrap();
    assert!(matches!(ui.events.back(), Some(GuestUiEvent::Intent { intent: Intent::SetValue(ControlValue::Choice(0)), .. })));
    assert_eq!(ui.route_census(w), RouteCensus { native: 1, replayed: 0, raw: 0 });
}
