//! The Dialog Manager on the v2 seam: a hand-assembled program (`fixture::dialog_script`) opens a DLOG/DITL dialog, sits in
//! `ModalDialog`, reads and rewrites an editable text item and disposes the dialog. The host acts through `push_intent` and
//! key events; the guest must see item hits and text exactly as on a Mac. `macppc-engine/tests/dialog.rs` runs the SAME script
//! through the PowerPC glue and must produce the same observation.
mod common;
use common::{put, pstr, Prog, GLOBALS};
use guestui_abi::{CallbackToken, ControlValue, GuestUiEvent, Intent, Modifiers, RecordingGuestUi, UiObjectRef};
use mac68k_engine::fixture::{cells, dialog_fork, dialog_observation, dialog_script, DIALOG_EXPECTED};
use mac68k_engine::translate::Obj;
use mac68k_engine::{Mac, Step};

pub fn machine() -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(dialog_fork()).unwrap());
    put(&mut m, cells::NEW_TEXT, &pstr("Changed"));
    m
}

/// (dialog window ptr, its native ref, a closure-free lookup of item `n`'s native control ref).
fn dialog(m: &Mac<RecordingGuestUi>) -> (u32, UiObjectRef) {
    let w = *m.tr.dlg.dialogs.keys().next().expect("a dialog is open");
    (w, m.windows[&w].native.expect("shown"))
}
fn item_ref(m: &Mac<RecordingGuestUi>, win: u32, n: usize) -> (UiObjectRef, u32) {
    let h = m.tr.dlg.dialogs[&win].list[n - 1].handle;
    let p = m.tr.projected(Obj::Control(h)).or_else(|| m.tr.projected(Obj::Text(h))).unwrap_or_else(|| panic!("item {n} is not projected"));
    (p.native, h)
}

fn run_script() -> Mac<RecordingGuestUi> {
    let mut m = machine();
    let mut p = Prog::new();
    p.script(&m, &dialog_script()).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "the program waits in ModalDialog: {:#?}", m.log);
    assert!(m.tr.dlg.modal.is_some());
    let (win, nwin) = dialog(&m);
    let (edit, edit_h) = item_ref(&m, win, 5);
    let (check, _) = item_ref(&m, win, 3);
    m.ui.push_intent(nwin, edit, Intent::CommitText("World".into())).unwrap();
    m.ui.push_intent(nwin, check, Intent::SetValue(ControlValue::Checked(true))).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "the second ModalDialog waits: {:#?}", m.log);
    assert_eq!(edit_h, m.tr.dlg.dialogs[&win].list[4].handle);
    m.ui.events.push_back(GuestUiEvent::Key { win: nwin, code: 36, text: "\r".into(), modifiers: Modifiers::default(), down: true });
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    m
}

#[test]
fn a_modal_dialog_runs_as_68k_and_the_guest_sees_item_hits_and_text() {
    let m = run_script();
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let got = dialog_observation(&m);
    assert_eq!(got, DIALOG_EXPECTED, "\n{}", got.join("\n"));
}

#[test]
fn dialog_negative_controls() {
    let mut m = machine();
    let mut p = Prog::new();
    p.script(&m, &dialog_script()[..2]).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked);
    let (win, nwin) = dialog(&m);
    let (stat, _) = item_ref(&m, win, 4);
    let (ok, _) = item_ref(&m, win, 1);
    // text cannot be committed into static text; a button has no text; a stale generation is refused
    m.ui.push_intent(nwin, stat, Intent::CommitText("x".into())).unwrap();
    m.ui.push_intent(nwin, ok, Intent::CommitText("x".into())).unwrap();
    m.ui.events.push_back(GuestUiEvent::Intent { win: nwin, control: ok, token: CallbackToken(u64::from(m.tr.dlg.dialogs[&win].list[0].handle)), generation: 9, intent: Intent::Activate });
    m.pump_ui();
    assert_eq!(m.refused.len(), 3, "{:?}", m.refused);
    assert!(m.refused[0].contains("StaticText") && m.refused[1].contains("pushButProc") && m.refused[2].contains("generation 9"), "{:?}", m.refused);
    assert!(m.tr.dlg.hits.is_empty(), "no refused intent became an item hit");
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "the dialog is still waiting");
    // a disabled item's hit is ignored by name (item 6 is disabled); a key that is not a character is refused
    m.dialog_hit(win, 6);
    assert!(m.tr.dlg.hits.is_empty() && m.log.iter().any(|l| l.contains("item 6") && l.contains("disabled")));
    m.ui.events.push_back(GuestUiEvent::Key { win: nwin, code: 123, text: String::new(), modifiers: Modifiers::default(), down: true });
    m.pump_ui();
    assert!(m.refused.last().unwrap().contains("no character"), "{:?}", m.refused);
    // positive control: the real OK click ends the wait with itemHit 1
    m.ui.push_intent(nwin, ok, Intent::Activate).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited);
    assert_eq!(m.mem.read_u16(cells::HIT1).unwrap(), 1);
}
