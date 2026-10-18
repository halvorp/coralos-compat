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
    // typing in the edit field returns ITS item from the first ModalDialog; the check box click returns item 3 from the second
    m.ui.push_intent(nwin, edit, Intent::CommitText("World".into())).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "the second ModalDialog waits: {:#?}", m.log);
    assert_eq!(edit_h, m.tr.dlg.dialogs[&win].list[4].handle);
    m.ui.push_intent(nwin, check, Intent::SetValue(ControlValue::Checked(true))).unwrap();
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

// ---- ModalDialog behaves like the original OS (review findings) ----

/// A machine parked in the first `ModalDialog` of `dialog_script`.
fn parked() -> (Mac<RecordingGuestUi>, u32, UiObjectRef) {
    let mut m = machine();
    let mut p = Prog::new();
    p.script(&m, &dialog_script()[..2]).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked);
    let (win, nwin) = dialog(&m);
    (m, win, nwin)
}
fn key(nwin: UiObjectRef, text: &str, modifiers: Modifiers) -> GuestUiEvent { GuestUiEvent::Key { win: nwin, code: 30, text: text.into(), modifiers, down: true } }

#[test]
fn a_key_typed_into_the_edit_field_returns_the_edit_item_and_never_reaches_the_event_queue() {
    let (mut m, _, nwin) = parked();
    m.ui.events.push_back(key(nwin, "x", Modifiers::default()));
    m.pump_ui();
    assert!(m.ev.queue.iter().all(|e| e.what != 3 && e.what != 5), "no keyDown leaks to the program: {:?}", m.ev.queue);
    assert_eq!(m.step(1_000_000).0, Step::Exited);
    assert_eq!(m.mem.read_u16(cells::HIT1).unwrap(), 5, "ModalDialog returned the edit item, as DialogSelect does after TEKey");
    // NEGATIVE control: a key with no edit field in the dialog is consumed too, and returns nothing
    let (mut m, _, nwin) = parked();
    m.mem.write_u16(m.tr.dlg.dialogs.keys().next().unwrap() + 164, 0xFFFF).unwrap(); // editField = none
    m.ui.events.push_back(key(nwin, "x", Modifiers::default()));
    m.pump_ui();
    assert!(m.ev.queue.iter().all(|e| e.what != 3 && e.what != 5) && m.tr.dlg.hits.is_empty(), "{:?}", m.ev.queue);
    assert!(m.log.iter().any(|l| l.contains("no edit field")), "{:#?}", m.log);
}

#[test]
fn typed_keys_edit_the_text_and_a_command_key_is_consumed_by_name() {
    let (mut m, _, nwin) = parked();
    m.ui.events.push_back(key(nwin, "a", Modifiers::default()));
    m.ui.events.push_back(key(nwin, "\u{8}", Modifiers::default()));
    m.ui.events.push_back(key(nwin, "q", Modifiers(Modifiers::META)));
    m.pump_ui();
    assert!(m.ev.queue.iter().all(|e| e.what != 3 && e.what != 5), "a command-Q typed into a dialog is not the program's: {:?}", m.ev.queue);
    assert!(m.log.iter().any(|l| l.contains("command-key")), "{:#?}", m.log);
    assert_eq!(m.tr.dlg.hits.len(), 2, "the two edits are hits of item 5; the command key is none");
    let h = m.tr.dlg.dialogs.values().next().unwrap().list[4].handle;
    let b = m.mm.block(h).unwrap();
    assert_eq!(m.mem.bytes(m.mem.span(b.ptr, b.size).unwrap()).unwrap(), b"Hello", "typed 'a' then backspace: back to the start");
}

#[test]
fn a_filter_proc_is_named_not_silently_ignored() {
    let mut m = machine();
    let mut p = Prog::new();
    p.script(&m, &dialog_script()[..1]);
    p.l(0x1234).l(cells::HIT1).trap(0xA991).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked);
    assert!(m.refused.iter().any(|r| r.contains("ModalDialog") && r.contains("filterProc 0x1234")), "{:?}", m.refused);
    // NEGATIVE control: a NIL filterProc says nothing (every other dialog test asserts refused is empty)
    let (m, ..) = parked();
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn the_default_item_is_guest_data_and_never_panics() {
    for bad in [i16::MIN, -1, 0, 7, i16::MAX] {
        let (mut m, win, nwin) = parked();
        m.mem.write_u16(win + 168, bad as u16).unwrap(); // aDefItem
        m.ui.events.push_back(key(nwin, "\r", Modifiers::default()));
        m.pump_ui();
        assert!(m.tr.dlg.hits.is_empty(), "aDefItem {bad} names no enabled item: nothing is hit");
        assert!(m.log.iter().any(|l| l.contains("the hit is ignored")), "aDefItem {bad}: {:#?}", m.log);
    }
    // positive control: the stock aDefItem 1 is OK
    let (mut m, _, nwin) = parked();
    m.ui.events.push_back(key(nwin, "\r", Modifiers::default()));
    assert_eq!(m.step(1_000_000).0, Step::Exited);
    assert_eq!(m.mem.read_u16(cells::HIT1).unwrap(), 1);
}

/// DLOG 130 with an OK button (item 1) and a `resCtrl` item 2 naming CNTL 130, a vertical scroll bar 0..100.
fn scroll_dialog_fork() -> Vec<u8> {
    let ps = |s: &str| { let mut v = vec![s.len() as u8]; v.extend_from_slice(s.as_bytes()); v };
    let item = |rect: [i16; 4], ty: u8, data: &[u8]| {
        let mut v = vec![0u8; 4];
        for r in rect { v.extend_from_slice(&r.to_be_bytes()); }
        v.push(ty);
        v.push(data.len() as u8);
        v.extend_from_slice(data);
        if data.len() & 1 == 1 { v.push(0); }
        v
    };
    let mut ditl = 1i16.to_be_bytes().to_vec();
    for it in [item([80, 200, 100, 260], 4, &ps("OK")), item([10, 220, 70, 236], 7, &130i16.to_be_bytes())] { ditl.extend_from_slice(&it); }
    let mut dlog = Vec::new();
    for v in [60i16, 40, 180, 300, 1, 0x0100, 0x0000] { dlog.extend_from_slice(&v.to_be_bytes()); }
    dlog.extend_from_slice(&0u32.to_be_bytes());
    dlog.extend_from_slice(&130i16.to_be_bytes());
    dlog.extend_from_slice(&ps("Scroll"));
    let mut cntl = Vec::new();
    for v in [10i16, 220, 70, 236, 0, 0x0100, 100, 0, 16] { cntl.extend_from_slice(&v.to_be_bytes()); } // rect, value, visible, max, min, procID 16
    cntl.extend_from_slice(&0u32.to_be_bytes());
    cntl.extend_from_slice(&ps(""));
    retro_resfork::writer::ForkBuilder::new().add(b"DLOG", 130, None, &dlog).add(b"DITL", 130, None, &ditl).add(b"CNTL", 130, None, &cntl).build()
}

#[test]
fn a_scroll_bar_in_a_dialog_moves_its_value_and_reports_its_item() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(scroll_dialog_fork()).unwrap());
    let mut p = Prog::new();
    p.clr_l().w(130).l(0).l(0xFFFF_FFFF).trap(0xA97C).pop_l(cells::DIALOG).l(0).l(cells::HIT1).trap(0xA991).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "{:#?}", m.log);
    let (win, nwin) = dialog(&m);
    let (bar, h) = item_ref(&m, win, 2);
    m.ui.push_intent(nwin, bar, Intent::ScrollTo(40)).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert_eq!(m.mem.read_u16(cells::HIT1).unwrap(), 2, "the item is reported");
    assert_eq!(m.control_facts(h).unwrap().value, 40, "and DialogSelect's TrackControl left the thumb where it was dragged");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}
