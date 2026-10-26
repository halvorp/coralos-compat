//! The Choice / List / Gauge roles: what a host holds, what it refuses by name, and that the route census counts them native.
use heirloom_ui_abi::*;

fn model(kind: ControlKind) -> ControlModel {
    ControlModel { kind, label: String::new(), rect: GuestRect { left: 0, top: 0, width: 80, height: 20 }, enabled: true, token: CallbackToken(1) }
}
fn window(ui: &mut RecordingGuestUi) -> UiObjectRef {
    ui.open_window(WindowModel { title: "t".into(), rect: GuestRect { left: 0, top: 0, width: 200, height: 100 }, role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(0) }).unwrap()
}
fn s(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

#[test]
fn a_choice_holds_its_selection_and_refuses_an_index_past_its_labels() {
    let mut ui = RecordingGuestUi::new();
    let w = window(&mut ui);
    let c = ui.add_control(w, model(ControlKind::Choice { style: ChoiceStyle::Cycle, labels: s(&["a", "b", "c"]), selected: 0 })).unwrap();
    ui.set_control(w, c, ControlValue::Choice(2)).unwrap();
    assert!(matches!(&ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Choice(2)))));
    assert_eq!(ui.set_control(w, c, ControlValue::Choice(3)), Err(UiRefusal::InvalidGeometry("choice index past its labels")));
    // CONTROL: the shape rule is on add, too.
    assert!(ui.add_control(w, model(ControlKind::Choice { style: ChoiceStyle::Tabs, labels: s(&["a"]), selected: 1 })).is_err());
    ui.push_intent(w, c, Intent::SetValue(ControlValue::Choice(1))).unwrap();
    assert!(matches!(ui.events.back(), Some(GuestUiEvent::Intent { intent: Intent::SetValue(ControlValue::Choice(1)), .. })));
}

#[test]
fn a_list_replaces_its_rows_whole_and_a_gauge_stays_inside_its_range() {
    let mut ui = RecordingGuestUi::new();
    let w = window(&mut ui);
    let l = ui.add_control(w, model(ControlKind::List { columns: s(&["Name", "Size"]), rows: vec![s(&["x", "1"])], selected: None, top: 0 })).unwrap();
    ui.set_control(w, l, ControlValue::Rows { rows: vec![s(&["x", "1"]), s(&["y", "2"])], selected: Some(1), top: 0 }).unwrap();
    assert!(ui.set_control(w, l, ControlValue::Rows { rows: vec![s(&["only one cell"])], selected: None, top: 0 }).is_err(), "a row needs one cell per column");
    assert!(ui.set_control(w, l, ControlValue::Rows { rows: vec![s(&["x", "1"])], selected: Some(1), top: 0 }).is_err(), "the selection must be a row");
    let g = ui.add_control(w, model(ControlKind::Gauge { value: 3, max: 10 })).unwrap();
    ui.set_control(w, g, ControlValue::Level(10)).unwrap();
    assert!(ui.set_control(w, g, ControlValue::Level(11)).is_err());
    assert!(ui.add_control(w, model(ControlKind::Gauge { value: 0, max: 0 })).is_err(), "max is at least 1");
    assert_eq!(ui.route_census(w), RouteCensus { native: 2, replayed: 0, raw: 0 });
}

#[test]
fn a_radio_can_say_none_chosen_and_nothing_else_can() {
    let mut ui = RecordingGuestUi::new();
    let w = window(&mut ui);
    // a radio whose buttons are all off: a Mac radio that is off, a GadTools MX nobody has picked in
    let r = ui.add_control(w, model(ControlKind::Choice { style: ChoiceStyle::Radio, labels: s(&["Fast"]), selected: CHOICE_NONE })).unwrap();
    ui.set_control(w, r, ControlValue::Choice(0)).unwrap();
    ui.set_control(w, r, ControlValue::Choice(CHOICE_NONE)).unwrap();
    assert!(matches!(&ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Choice(CHOICE_NONE)))));
    // NEGATIVE CONTROLS: a cycle, a pop-up and a tab strip always show one label, and an index is still bounded by the labels
    for style in [ChoiceStyle::Cycle, ChoiceStyle::Popup, ChoiceStyle::Tabs] {
        assert!(ui.add_control(w, model(ControlKind::Choice { style, labels: s(&["a", "b"]), selected: CHOICE_NONE })).is_err(), "{style:?} cannot be none-selected");
        let c = ui.add_control(w, model(ControlKind::Choice { style, labels: s(&["a", "b"]), selected: 0 })).unwrap();
        assert_eq!(ui.set_control(w, c, ControlValue::Choice(CHOICE_NONE)), Err(UiRefusal::InvalidGeometry("a choice with nothing selected is a radio's")));
    }
    assert_eq!(ui.set_control(w, r, ControlValue::Choice(1)), Err(UiRefusal::InvalidGeometry("choice index past its labels")));
    assert_eq!(ChoiceStyle::Radio.allows_none(), true);
}
