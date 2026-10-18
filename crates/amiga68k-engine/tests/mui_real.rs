//! REAL AROS Zune programs (unmodified nightly binaries, not committed: /data/build/corpus-bin, each test SKIPS loudly when absent):
//! their MUI windows as native controls, and a native intent reaching the program exactly (its own notifications fire, its own
//! guest code runs). The programs are driven only through the seam: `GuestUiEvent`s in, `RecordingGuestUi` calls out.
use amiga68k_engine::{Amiga, Delivered, Step};
use guestui_abi::{ControlKind, ControlValue, GuestUiEvent, Intent, RecordingGuestUi, UiCall};

const ROOT: &str = "/data/build/corpus-bin";

fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path()); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

/// Start the AROS program `rel` on the trimmed AROS volume (docs/CORPUS.md "the mini volume") and run it until it blocks.
fn start(rel: &str) -> Option<Amiga<RecordingGuestUi>> {
    let file = std::fs::read(format!("{ROOT}/aros-m68k/sys/{rel}")).ok().or_else(|| { eprintln!("SKIPPED: {ROOT}/aros-m68k/sys/{rel} not present (see docs/CORPUS.md)"); None })?;
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    load(&mut a, "SYS:", std::path::Path::new(&format!("{ROOT}/aros-m68k/mini")));
    a.fs.vol.assign("LIBS", "SYS:Libs");
    a.stop_on_unimplemented = true;
    a.start_hunk(&file).unwrap();
    let (s, _) = a.step(20_000_000);
    assert_eq!(s, Step::Blocked, "{rel}: {:?} {:?}", a.refused, a.log.iter().rev().take(6).collect::<Vec<_>>());
    Some(a)
}

/// Deliver `intent` to the live control matching `pred`, then let the program process it.
fn act(a: &mut Amiga<RecordingGuestUi>, pred: impl Fn(&guestui_abi::ControlModel) -> bool, intent: Intent) -> Delivered {
    let (w, c, m) = a.ui.find_control(pred).expect("the control is live");
    let d = a.deliver(GuestUiEvent::intent(w, c, m.token, intent)).unwrap();
    let (s, _) = a.step(20_000_000);
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    d
}

fn display(a: &Amiga<RecordingGuestUi>) -> String {
    a.ui.controls.values().find_map(|(_, m)| match &m.kind { ControlKind::Label { text, framed: true } => Some(text.clone()), _ => None }).unwrap()
}
fn button(label: &'static str) -> impl Fn(&guestui_abi::ControlModel) -> bool { move |m| matches!(m.kind, ControlKind::Button) && m.label == label }

#[test]
fn a_zune_calculator_opens_as_native_buttons_and_a_native_click_runs_its_own_class_code() {
    let Some(mut a) = start("Tools/Calculator") else { return };
    let labels: Vec<String> = a.ui.controls.values().filter(|(_, m)| matches!(m.kind, ControlKind::Button)).map(|(_, m)| m.label.clone()).collect();
    assert_eq!(labels.len(), 20, "{labels:?}");
    for l in ["0", "1", "2", "3", "+", "="] { assert!(labels.iter().any(|x| x == l), "button {l:?} in {labels:?}"); }
    let win = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.clone()), _ => None }).unwrap();
    assert_eq!(win.title, "Calculator");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!(display(&a), "0");
    // 1 + 2 = : every click is a complete press/release; the program's hook (a class of its own) updates the display label.
    for k in ["1", "+", "2", "="] { assert_eq!(act(&mut a, button(k), Intent::Activate), Delivered::Handled); }
    assert_eq!(display(&a), "3", "1 + 2 on the program's own arithmetic");
    // The display went through the seam as a value change on the SAME native label, never a rebuilt control.
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, _, ControlValue::Text(t)) if t == "3")));
    // NEGATIVE CONTROL: a release outside the button only un-selects it; the program's notification (Pressed FALSE) does not run.
    let before = a.ui.calls.len();
    for step in [Intent::Press, Intent::Release { inside: false }] { act(&mut a, button("9"), step); }
    assert_eq!(display(&a), "3", "a cancelled click is not a key press");
    assert!(a.ui.calls[before..].iter().all(|c| !matches!(c, UiCall::SetControl(_, _, ControlValue::Text(_)))));
    for step in [Intent::Press, Intent::Release { inside: true }] { act(&mut a, button("9"), step); }
    assert_eq!(display(&a), "9", "a completed click is");
    // A stale/meaningless intent is refused by NAME at once and nothing changes.
    let (w, c, m) = a.ui.find_control(button("1")).unwrap();
    let e = a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SelectRow(2))).unwrap_err();
    assert!(e.0.contains("means nothing for the control"), "{}", e.0);
}
