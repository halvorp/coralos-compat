//! The generic mapper every engine shares (ruling 17 piece 2): add, diff-push, rebuild on a new shape, retire, and the one
//! stale-intent rule. Every behaviour has its negative control next to it.
use guestui_abi::*;

fn r(l: i32, t: i32, w: i32, h: i32) -> GuestRect { GuestRect { left: l, top: t, width: w, height: h } }
fn win() -> WindowModel { WindowModel { title: "T".into(), rect: r(0, 0, 100, 100), role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(1) } }
fn model(kind: ControlKind, label: &str) -> ControlModel { ControlModel { kind, label: label.into(), rect: r(1, 1, 50, 20), enabled: true, token: CallbackToken(7) } }

#[test]
fn only_the_difference_is_pushed_and_a_new_shape_is_a_new_generation() {
    let (mut ui, mut reg) = (RecordingGuestUi::new(), Projections::<u32>::default());
    let w = ui.open_window(win()).unwrap();
    let bar = |top| model(ControlKind::ScrollBar { vertical: true, total: 10, visible: 1, top }, "");
    reg.sync(&mut ui, Some(w), 7, 3, Some(bar(0))).unwrap();
    assert_eq!(ui.calls.iter().filter(|c| matches!(c, UiCall::AddControl(..))).count(), 1);
    let first = reg.get(7).unwrap().native;
    // same model: nothing at all is sent
    let n = ui.calls.len();
    reg.sync(&mut ui, Some(w), 7, 3, Some(bar(0))).unwrap();
    assert_eq!(ui.calls.len(), n, "an unchanged model is not echoed");
    // value, rect and enabled each push exactly their own call
    let mut m = bar(4);
    m.rect = r(2, 2, 50, 20);
    m.enabled = false;
    reg.sync(&mut ui, Some(w), 7, 3, Some(m)).unwrap();
    let new: Vec<&UiCall> = ui.calls[n..].iter().collect();
    assert!(matches!(new[..], [UiCall::SetControlRect(..), UiCall::SetControlEnabled(_, _, false), UiCall::SetControl(_, _, ControlValue::Scroll { top: 4, .. })]), "{new:?}");
    assert_eq!(reg.get(7).unwrap().native, first, "same part, same generation");
    // a different fixed parameter (orientation) or label is a NEW incarnation: remove + add, the old ref is refused
    reg.sync(&mut ui, Some(w), 7, 3, Some(model(ControlKind::ScrollBar { vertical: false, total: 10, visible: 1, top: 4 }, ""))).unwrap();
    let second = reg.get(7).unwrap().native;
    assert_ne!(second.generation, first.generation);
    assert!(reg.resolve(7, w, first, first.generation).is_err(), "the old incarnation is stale");
    assert_eq!(reg.resolve(7, w, second, second.generation).unwrap().meta, 3);
    // a window that is not on screen yet shows nothing now (negative control), a None want retires
    let mut other = Projections::<u32>::default();
    other.sync(&mut ui, None, 9, 0, Some(model(ControlKind::Button, "x"))).unwrap();
    assert!(other.is_empty());
    reg.sync(&mut ui, Some(w), 7, 3, None).unwrap();
    assert!(reg.is_empty() && ui.controls.is_empty());
}

#[test]
fn the_stale_intent_rule_names_each_reason() {
    let (mut ui, mut reg) = (RecordingGuestUi::new(), Projections::<()>::default());
    let w = ui.open_window(win()).unwrap();
    reg.sync(&mut ui, Some(w), 7, (), Some(model(ControlKind::Button, "b"))).unwrap();
    let c = reg.get(7).unwrap().native;
    assert!(reg.resolve(7, w, c, c.generation).is_ok(), "positive control");
    let why = |e: Result<_, String>| -> String { e.map(|_: &Projected<()>| ()).unwrap_err() };
    assert!(why(reg.resolve(7, w, c, c.generation + 1)).contains("generation"));
    assert!(why(reg.resolve(8, w, c, c.generation)).contains("not a live control"));
    let other = UiObjectRef { id: c.id + 50, generation: c.generation };
    assert!(why(reg.resolve(7, w, other, other.generation)).contains("stale control ref"));
    let w2 = UiObjectRef { id: w.id + 9, generation: 1 };
    assert!(why(reg.resolve(7, w2, c, c.generation)).contains("does not belong to window"));
    // the free function is the same rule for engines that keep their own bookkeeping
    assert!(check_ref(7, w, c, c.generation, Some((w, c))).is_ok());
    assert!(check_ref(7, w, c, c.generation, None).is_err());
}

#[test]
fn restore_and_adopt_keep_the_engine_and_the_host_in_step() {
    let (mut ui, mut reg) = (RecordingGuestUi::new(), Projections::<()>::default());
    let w = ui.open_window(win()).unwrap();
    let field = |t: &str| model(ControlKind::Field { text: t.into(), max_len: 20, editable: true, integer: false }, "");
    reg.sync(&mut ui, Some(w), 7, (), Some(field("a"))).unwrap();
    // the user typed "ab": adopting it means the next sync of the same text pushes nothing
    reg.adopt(7, &ControlValue::Text("ab".into()));
    let n = ui.calls.len();
    reg.sync(&mut ui, Some(w), 7, (), Some(field("ab"))).unwrap();
    assert_eq!(ui.calls.len(), n);
    // restore re-sends the stored state (the guest's truth) over an optimistic host
    reg.restore(&mut ui, 7).unwrap();
    assert!(matches!(ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Text(t))) if t == "ab"));
    // a kind with no value has nothing to restore
    reg.sync(&mut ui, Some(w), 8, (), Some(model(ControlKind::Button, "b"))).unwrap();
    let n = ui.calls.len();
    reg.restore(&mut ui, 8).unwrap();
    assert_eq!(ui.calls.len(), n);
}
