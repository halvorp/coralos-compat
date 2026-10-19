//! REAL AROS Prefs programs whose windows are made by classes that live on DISK (`Classes/Zune/*.mcc`): the program creates its own
//! class over `PrefsEditor.mcc`, its window over `SystemPrefsWindow.mcc` (over `PrefsWindow.mcc`). The engine opens each as a disk
//! library, runs its Open and `MCC_Query(0)` in the guest, and the program's window appears as native controls. Programs and classes
//! are unmodified AROS m68k binaries, not committed (docs/CORPUS.md); each test SKIPS loudly when absent.
mod aros;
use amiga68k_engine::{Amiga, Delivered, Step};
use guestui_abi::{ControlKind, ControlValue, GuestUiEvent, Intent, RecordingGuestUi, UiCall};

fn kinds(a: &Amiga<RecordingGuestUi>) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for (_, c) in a.ui.controls.values() { *m.entry(format!("{:?}", c.kind).split([' ', '{', '(']).next().unwrap().to_string()).or_default() += 1; }
    m
}
fn title(a: &Amiga<RecordingGuestUi>) -> Option<String> { a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.title.clone()), _ => None }) }
fn disk_classes(a: &Amiga<RecordingGuestUi>) -> Vec<String> { a.objects.disk_classes.keys().cloned().collect() }

#[test]
fn the_appearance_prefs_window_is_made_by_three_classes_loaded_from_disk_and_shows_as_native_controls() {
    let Some((mut a, s)) = aros::start("Prefs/Appearance", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?} {:?}", a.refused, a.log.iter().rev().take(6).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!(disk_classes(&a), ["PrefsEditor.mcc", "PrefsWindow.mcc", "SystemPrefsWindow.mcc"], "each opened once, from SYS:Classes/Zune");
    assert!(a.objects.disk_classes.values().all(|d| d.lib != 0 && d.users >= 1 && a.mem.read_u32(d.mcc + 28).unwrap() == d.lib), "mcc_Module = the library, as Zune sets it");
    assert_eq!(title(&a).as_deref(), Some("Appearance Preferences"));
    let k = kinds(&a);
    assert_eq!((k.get("Checkbox"), k.get("Choice"), k.get("Button")), (Some(&11), Some(&1), Some(&5)), "{k:?}");
    // A native click on a check box becomes MUIA_Selected and runs the program's OWN notification chain (guest code of the classes
    // loaded from disk): here it greys other controls, which reaches the host as SetControlEnabled.
    let (w, c, m) = a.ui.find_control(|m| matches!(m.kind, ControlKind::Checkbox { checked: false })).expect("an unchecked box");
    let obj = m.token.0 as u32;
    let before = a.ui.calls.len();
    let enabled_changes = |a: &Amiga<RecordingGuestUi>| a.ui.calls[before..].iter().filter(|c| matches!(c, UiCall::SetControlEnabled(..))).count();
    assert_eq!(a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SetValue(ControlValue::Checked(true)))).unwrap(), Delivered::Handled);
    assert_eq!(a.step(20_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert_eq!(amiga68k_engine::mui::get_attr(&mut a, obj, amiga68k_engine::mui::MUIA_Selected).unwrap(), 1, "the box holds the user's value");
    assert!(enabled_changes(&a) >= 1, "the program's notification ran: {:?}", a.ui.calls[before..].iter().collect::<Vec<_>>());
    // NEGATIVE CONTROL: a second intent that changes nothing the program is told about (the same value again) runs no notification.
    let after = a.ui.calls.len();
    a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SetValue(ControlValue::Checked(true)))).unwrap();
    assert_eq!(a.step(20_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert_eq!(a.ui.calls[after..].iter().filter(|c| matches!(c, UiCall::SetControlEnabled(..))).count(), 0, "no change, no notification run");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}

#[test]
fn the_font_prefs_window_has_its_fields_and_buttons_and_a_missing_class_file_is_a_named_refusal_not_a_stand_in() {
    let Some((a, s)) = aros::start("Prefs/Font", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    let k = kinds(&a);
    assert_eq!((k.get("Field"), k.get("Label"), k.get("Button")), (Some(&3), Some(&3), Some(&8)), "{k:?}");
    // NEGATIVE CONTROL: the same program with PrefsEditor.mcc missing from the volume: the class is not implemented, by name, and no window opens.
    let (b, _) = aros::start("Prefs/Font", &["SYS:Classes/Zune/PrefsEditor.mcc"]).unwrap();
    assert!(b.refused.iter().any(|r| r.contains("MUI_CreateCustomClass: the superclass \"PrefsEditor.mcc\" is not implemented")), "{:?}", b.refused);
    assert!(title(&b).is_none() && b.ui.controls.is_empty());
    assert!(b.objects.disk_classes.is_empty());
}

#[test]
fn a_disk_class_instance_is_disposed_with_its_window_and_releases_the_class() {
    let Some((mut a, _)) = aros::start("Prefs/Pointer", &[]) else { return };
    assert_eq!(title(&a).as_deref(), Some("Pointer Preferences"), "{:?}", a.refused);
    assert!(a.objects.disk_classes.values().all(|d| d.users >= 1), "while the window lives, each class is in use: {:?}", a.objects.disk_classes);
    let (w, _, _) = a.ui.find_control(|_| true).unwrap();
    assert_eq!(a.deliver(GuestUiEvent::CloseRequested { win: w, token: guestui_abi::CallbackToken(0) }).unwrap(), Delivered::Handled);
    assert_eq!(a.step(40_000_000).0, Step::Exited, "{:?}", a.refused);
    assert!(a.refused.is_empty() && a.objects.map.is_empty(), "{:?} {} objects left", a.refused, a.objects.map.len());
    // Zune's family dispose order: the Application disposes the window and the window its editor through THEIR dispatchers (guest code,
    // classes from disk), which reach the native half with DoSuperMethod; the destructor of each custom child ran.
    let reached = |c: &str| a.log.iter().filter(|l| l.starts_with(&format!("[boopsi] {c} OM_DISPOSE on"))).count();
    assert_eq!((reached("Window.mui"), reached("Group.mui")), (1, 1), "{:?}", a.log.iter().filter(|l| l.contains("OM_DISPOSE")).collect::<Vec<_>>());
    let users: Vec<(&str, u32)> = a.objects.disk_classes.iter().map(|(k, d)| (k.as_str(), d.users)).collect();
    // Every use the program held was given back; PrefsWindow.mcc stays held by the loaded SystemPrefsWindow.mcc class built over it.
    assert_eq!(users, [("PrefsEditor.mcc", 0), ("PrefsWindow.mcc", 1), ("SystemPrefsWindow.mcc", 0)]);
}
