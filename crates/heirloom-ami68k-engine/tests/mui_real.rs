//! REAL AROS Zune programs (unmodified nightly binaries, not committed: /data/build/corpus-bin, each test SKIPS loudly when absent):
//! their MUI windows as native controls, and a native intent reaching the program exactly (its own notifications fire, its own
//! guest code runs). The programs are driven only through the seam: `GuestUiEvent`s in, `RecordingGuestUi` calls out.
use heirloom_ami68k_engine::{Amiga, Delivered, Step};
use heirloom_ui_abi::{ControlKind, ControlValue, GuestUiEvent, Intent, RecordingGuestUi, UiCall};

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
fn start(rel: &str) -> Option<Amiga<RecordingGuestUi>> { start_with(rel, Default::default()) }

/// [`start`] with the host's control metrics set before the program runs (its window is laid out for them).
fn start_with(rel: &str, metrics: heirloom_ami68k_engine::mui::layout::Metrics) -> Option<Amiga<RecordingGuestUi>> {
    let file = std::fs::read(format!("{ROOT}/aros-m68k/sys/{rel}")).ok().or_else(|| { eprintln!("SKIPPED: {ROOT}/aros-m68k/sys/{rel} not present (see docs/CORPUS.md)"); None })?;
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    load(&mut a, "SYS:", std::path::Path::new(&format!("{ROOT}/aros-m68k/mini")));
    a.fs.vol.assign("LIBS", "SYS:Libs");
    a.stop_on_unimplemented = true;
    a.objects.mui_metrics = metrics;
    a.start_hunk_as(&file, rel.rsplit('/').next().unwrap()).unwrap();
    let (s, _) = a.step(20_000_000);
    assert_eq!(s, Step::Blocked, "{rel}: {:?} {:?}", a.refused, a.log.iter().rev().take(6).collect::<Vec<_>>());
    Some(a)
}

/// Deliver `intent` to the live control matching `pred`, then let the program process it.
fn act(a: &mut Amiga<RecordingGuestUi>, pred: impl Fn(&heirloom_ui_abi::ControlModel) -> bool, intent: Intent) -> Delivered {
    let (w, c, m) = a.ui.find_control(pred).expect("the control is live");
    let d = a.deliver(GuestUiEvent::intent(w, c, m.token, intent)).unwrap();
    let (s, _) = a.step(20_000_000);
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    d
}

fn display(a: &Amiga<RecordingGuestUi>) -> String {
    a.ui.controls.values().find_map(|(_, m)| match &m.kind { ControlKind::Label { text, framed: true } => Some(text.clone()), _ => None }).unwrap()
}
fn button(label: &'static str) -> impl Fn(&heirloom_ui_abi::ControlModel) -> bool { move |m| matches!(m.kind, ControlKind::Button) && m.label == label }

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

fn labels(a: &Amiga<RecordingGuestUi>) -> Vec<String> {
    a.ui.controls.values().filter_map(|(_, m)| match &m.kind { ControlKind::Label { text, .. } => Some(text.clone()), _ => None }).collect()
}

#[test]
fn a_zune_slider_demo_is_a_native_slider_a_native_value_lands_in_the_object_and_a_native_close_quits_the_program() {
    let Some(mut a) = start("Demos/Zune/mui_slider") else { return };
    let win = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.clone()), _ => None }).unwrap();
    assert_eq!(win.title, "Input modes");
    let (w, c, m) = a.ui.find_control(|m| matches!(m.kind, ControlKind::Slider { .. })).expect("the Slider.mui object is a native slider");
    assert_eq!(m.kind, ControlKind::Slider { min: 0, max: 100, value: 50 }, "MUIA_Numeric_Value 50 in Numeric's default range 0..100");
    let slider = a.objects.map.iter().find(|(_, o)| o.kind == heirloom_ami68k_engine::mui::classes::Kind::Slider).map(|(&k, _)| k).unwrap();
    let before = a.ui.calls.len();
    assert_eq!(a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SetValue(ControlValue::Level(75)))).unwrap(), Delivered::Handled);
    // Queued: MUI processes input inside the program's NewInput, so nothing has changed until the program runs.
    assert_eq!(heirloom_ami68k_engine::mui::get_attr(&mut a, slider, heirloom_ami68k_engine::mui::MUIA_Numeric_Value).unwrap(), 50);
    assert_eq!(a.step(1_000_000).0, Step::Blocked);
    assert_eq!(heirloom_ami68k_engine::mui::get_attr(&mut a, slider, heirloom_ami68k_engine::mui::MUIA_Numeric_Value).unwrap(), 75, "GetAttr sees the user's value");
    assert!(a.ui.calls[before..].iter().all(|c| !matches!(c, UiCall::SetControl(..))), "the host already shows 75: no echo");
    // NEGATIVE CONTROL: an intent aimed at a retired generation of the control is refused by name and changes nothing.
    let stale = heirloom_ui_abi::UiObjectRef { id: c.id, generation: c.generation + 1 };
    let e = a.deliver(GuestUiEvent::Intent { win: w, control: stale, token: m.token, generation: stale.generation, intent: Intent::SetValue(ControlValue::Level(5)) }).unwrap_err();
    assert!(e.0.contains("stale control ref"), "{}", e.0);
    assert_eq!(a.step(1_000_000).0, Step::Blocked);
    assert_eq!(heirloom_ami68k_engine::mui::get_attr(&mut a, slider, heirloom_ami68k_engine::mui::MUIA_Numeric_Value).unwrap(), 75);
    // The native close gadget: the program's own MUIM_Notify (CloseRequest TRUE -> ReturnID Quit) fires and it exits cleanly.
    assert_eq!(a.deliver(GuestUiEvent::CloseRequested { win: w, token: heirloom_ui_abi::CallbackToken(0) }).unwrap(), Delivered::Handled);
    assert_eq!(a.step(20_000_000).0, Step::Exited, "{:?}", a.refused);
    assert!(a.refused.is_empty() && a.objects.map.is_empty(), "{:?}", a.refused);
    assert!(matches!(a.ui.calls.last(), Some(UiCall::CloseWindow(_))));
}

#[test]
fn a_zune_pages_demo_turns_its_pages_when_native_buttons_are_pressed() {
    let Some(mut a) = start("Demos/Zune/mui_pages") else { return };
    let page = |a: &Amiga<RecordingGuestUi>| labels(a).into_iter().find(|l| l.starts_with("Rect"));
    let titles: Vec<String> = a.ui.controls.values().filter(|(_, m)| matches!(m.kind, ControlKind::Button)).map(|(_, m)| m.label.clone()).collect();
    assert_eq!(titles, ["prev", "next", "1", "2", "3"], "RelVerify Rectangles are buttons, captioned by MUIA_Rectangle_BarTitle");
    assert_eq!(page(&a).as_deref(), Some("Rect1"), "MUIA_Group_PageMode shows page 0 only");
    assert_eq!(labels(&a).iter().filter(|l| l.starts_with("Rect")).count(), 1, "the other pages have no native control");
    let groups = a.ui.controls.values().filter(|(_, m)| matches!(m.kind, ControlKind::Group { .. })).count();
    assert!(groups >= 2, "GroupFrame and InputListFrame groups are native groups: {groups}");
    // A full click on "next": Pressed TRUE bumps MUIA_Timer, whose notification (EveryTime) sets MUIA_Group_ActivePage to Next.
    act(&mut a, button("next"), Intent::Activate);
    assert_eq!(page(&a).as_deref(), Some("Rect2"), "the program's own notification turned the page");
    act(&mut a, button("next"), Intent::Activate);
    assert_eq!(page(&a).as_deref(), Some("Rect3"));
    act(&mut a, button("next"), Intent::Activate);
    assert_eq!(page(&a).as_deref(), Some("Rect3"), "MUIV_Group_ActivePage_Next stops at the last page");
    // "1" notifies on Pressed FALSE, i.e. on RELEASE: a press alone does nothing, a cancelled click does nothing, a click does.
    act(&mut a, button("1"), Intent::Press);
    assert_eq!(page(&a).as_deref(), Some("Rect3"));
    act(&mut a, button("1"), Intent::Release { inside: false });
    assert_eq!(page(&a).as_deref(), Some("Rect3"), "a release outside is nnset(Pressed): the notification does not run");
    act(&mut a, button("1"), Intent::Press);
    act(&mut a, button("1"), Intent::Release { inside: true });
    assert_eq!(page(&a).as_deref(), Some("Rect1"));
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}

#[test]
fn a_zune_screen_grabber_fills_its_native_list_through_its_own_display_hook_and_takes_a_native_row_and_value() {
    let Some(mut a) = start("Tools/ScreenGrabber") else { return };
    // The List.mui has a MUIA_List_DisplayHook: the cell text exists only because the engine called the program's hook as a guest call.
    let (w, c, m) = a.ui.find_control(|m| matches!(m.kind, ControlKind::List { .. })).expect("List.mui is a native list");
    let ControlKind::List { rows, selected, .. } = &m.kind else { unreachable!() };
    assert_eq!((rows.as_slice(), *selected), ([vec!["Workbench".to_string()]].as_slice(), None), "the screen list, one row from the program's display hook");
    let kinds: std::collections::BTreeSet<String> = a.ui.controls.values().map(|(_, m)| format!("{:?}", m.kind).split([' ', '{', '(']).next().unwrap().to_string()).collect();
    for k in ["Button", "Checkbox", "Field", "Gauge", "Group", "Label", "List", "Slider"] { assert!(kinds.contains(k), "{k} in {kinds:?}"); }
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    // A native row choice becomes MUIA_List_Active and runs the program's notifications and hooks to the end.
    let list = a.objects.map.iter().find(|(_, o)| o.kind == heirloom_ami68k_engine::mui::classes::Kind::List).map(|(&k, _)| k).unwrap();
    assert_eq!(a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SelectRow(0))).unwrap(), Delivered::Handled);
    assert_eq!(a.step(20_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert_eq!(heirloom_ami68k_engine::mui::get_attr(&mut a, list, heirloom_ami68k_engine::mui::MUIA_List_Active).unwrap(), 0);
    // NEGATIVE CONTROL: a row index the list does not have leaves it with no active row (MUI keeps the active row inside the list).
    assert_eq!(a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SelectRow(7))).unwrap(), Delivered::Handled);
    assert_eq!(a.step(20_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert_eq!(heirloom_ami68k_engine::mui::get_attr(&mut a, list, heirloom_ami68k_engine::mui::MUIA_List_Active).unwrap(), MUIV_OFF);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}
const MUIV_OFF: u32 = u32::MAX; // MUIV_List_Active_Off

#[test]
fn the_zune_notify_demo_runs_the_notify_class_through_set_get_and_a_self_notification() {
    // mui_notify1 checks the Notify class itself and prints "(should be N)" next to every value: set/get of MUIA_UserData and a MUIM_Notify
    // whose follow-up is MUIM_Set on the object itself. Its last line expects 14 (a set to the SAME UserData value not notifying), but
    // Zune's Notify class (AROS workbench/libs/muimaster/classes/notify.c, OM_SET -> check_notify for every tag, TrigVal EveryTime) notifies
    // on every set, so the binary built against it prints 19 there: the engine follows the Zune code, not the demo's comment. (Not yet
    // confirmed against the Copperline oracle: tests/oracle/README.md.)
    let Ok(file) = std::fs::read(format!("{ROOT}/aros-m68k/sys/Demos/Zune/mui_notify1")) else { eprintln!("SKIPPED: mui_notify1 not present (see docs/CORPUS.md)"); return };
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    load(&mut a, "SYS:", std::path::Path::new(&format!("{ROOT}/aros-m68k/mini")));
    a.fs.vol.assign("LIBS", "SYS:Libs");
    let r = a.run_hunk(&file, 20_000_000).unwrap();
    assert!(r.stats.halted && a.refused.is_empty(), "{:?}", a.refused);
    let out = String::from_utf8_lossy(&a.console.stdout).into_owned();
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("new Notify object = "), "{out}");
    assert_eq!(&lines[1..], ["object UserData = 21 (should be 21)", "MUIM_Notify OK", "id = 12 (should be 12)", "id = 19 (should be 19) after notification", "id = 19 (should be 14) after notification"], "{out}");
    assert!(a.objects.map.is_empty(), "MUI_DisposeObject freed the object");
    // NEGATIVE CONTROL: the same program with the AROS Libs absent cannot even open muimaster.library, and prints none of it.
    let mut bare = Amiga::new(RecordingGuestUi::new()).unwrap();
    let _ = bare.run_hunk(&file, 20_000_000);
    assert!(!String::from_utf8_lossy(&bare.console.stdout).contains("should be"), "{:?}", bare.console.stdout);
}

// ---- the layout of real programs: nothing overlaps, everything fits the window, labels are the program's own strings ----

/// Every pair of leaf controls (groups frame their children, so they are exempt) that intersect, as a diagnostic list.
fn overlaps(ctls: &[(String, heirloom_ui_abi::GuestRect)]) -> Vec<String> {
    let hit = |a: &heirloom_ui_abi::GuestRect, b: &heirloom_ui_abi::GuestRect| a.left < b.left + b.width && b.left < a.left + a.width && a.top < b.top + b.height && b.top < a.top + a.height;
    let mut out = Vec::new();
    for (i, (na, ra)) in ctls.iter().enumerate() {
        for (nb, rb) in &ctls[i + 1..] { if hit(ra, rb) { out.push(format!("{na} {ra:?} x {nb} {rb:?}")); } }
    }
    out
}

fn leaves(a: &Amiga<RecordingGuestUi>) -> Vec<(String, heirloom_ui_abi::GuestRect)> {
    a.ui.controls.values().filter(|(_, m)| !matches!(m.kind, ControlKind::Group { .. })).map(|(_, m)| (format!("{:?}/{}", m.kind, m.label), m.rect)).collect()
}

fn window(a: &Amiga<RecordingGuestUi>) -> heirloom_ui_abi::WindowModel { a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.clone()), _ => None }).unwrap() }

fn assert_laid_out(p: &str, a: &Amiga<RecordingGuestUi>) {
    let win = window(a);
    assert!(!win.title.is_empty(), "{p}: an untitled MUI window shows the program's name");
    let ov = overlaps(&leaves(a));
    assert!(ov.is_empty(), "{p}: controls overlap: {ov:?}");
    for (_, m) in a.ui.controls.values() {
        let r = m.rect;
        assert!(r.left >= 0 && r.top >= 0 && r.left + r.width <= win.rect.width && r.top + r.height <= win.rect.height, "{p}: {:?} {:?} is outside the window {:?}", m.kind, r, win.rect);
    }
}

#[test]
fn the_overlap_check_sees_overlaps() {
    let r = |l, t, w, h| heirloom_ui_abi::GuestRect { left: l, top: t, width: w, height: h };
    assert!(overlaps(&[("a".into(), r(0, 0, 10, 10)), ("b".into(), r(10, 0, 10, 10))]).is_empty(), "touching is not overlapping");
    assert_eq!(overlaps(&[("a".into(), r(0, 0, 10, 10)), ("b".into(), r(9, 9, 10, 10))]).len(), 1);
}

#[test]
fn zune_snoopy_is_laid_out_without_overlap_titled_by_its_program_and_labelled_with_its_own_strings() {
    let Some(a) = start("System/Snoopy") else { return };
    assert_laid_out("Snoopy", &a);
    assert_eq!(window(&a).title, "Snoopy", "no MUIA_Window_Title: the program's name");
    let labels: Vec<String> = a.ui.controls.values().map(|(_, m)| m.label.clone()).collect();
    for l in ["FindPort", "OpenLibrary", "LoadSeg"] { assert!(labels.iter().any(|x| x == l), "{l} in {labels:?}"); }
    // NEGATIVE CONTROL: locale.library's GetLocaleStr names (weekdays, months) are never a Snoopy label; the catalog ids the program asks
    // GetCatalogStr for fall back to ITS default strings.
    for bad in ["Sunday", "Monday", "Thursday", "January", "July", "Sat"] { assert!(!labels.iter().any(|x| x == bad) && !a.ui.controls.values().any(|(_, m)| matches!(&m.kind, ControlKind::Group { title, .. } if title == bad)), "{bad} is a locale string, not Snoopy's"); }
    assert_eq!(a.ui.controls.values().filter(|(_, m)| matches!(m.kind, ControlKind::Checkbox { .. })).count(), 29, "one checkbox per monitored function");
}

#[test]
fn zune_calculator_and_screengrabber_are_laid_out_without_overlap() {
    for p in ["Tools/Calculator", "Tools/ScreenGrabber"] {
        let Some(a) = start(p) else { continue };
        assert_laid_out(p, &a);
        let labels: Vec<String> = a.ui.controls.values().map(|(_, m)| m.label.clone()).collect();
        assert!(!labels.iter().any(|l| l == "Sunday" || l == "Monday"), "{p}: {labels:?}");
    }
}

/// The CoralOS kit's controls are bigger than Zune's (toggle 56x28, 28 px fields and buttons, 12 px insets): MUI's own layout grows
/// every cell and the window for them, so the kit's parts never overlap and never leave the window.
#[test]
fn a_host_metrics_profile_grows_the_layout_so_larger_native_controls_still_never_overlap() {
    let kit = heirloom_ami68k_engine::mui::layout::Metrics { toggle: (56, 28), control_h: 28, pad_w: 24, label_right: Some(8), ..Default::default() };
    for p in ["System/Snoopy", "Tools/Calculator", "Tools/ScreenGrabber"] {
        let (Some(zune), Some(a)) = (start(p), start_with(p, kit)) else { continue };
        assert_laid_out(p, &a);
        assert!(window(&a).rect.width > window(&zune).rect.width && window(&a).rect.height > window(&zune).rect.height, "{p}: the window grew for the kit's controls");
        for (_, m) in a.ui.controls.values() {
            match m.kind {
                ControlKind::Checkbox { .. } => assert!(m.rect.height >= 28 && m.rect.width >= 56 + if m.label.is_empty() { 0 } else { 8 + 8 }, "{p}: the toggle, the gap and the label fit the rect: {:?} {:?}", m.rect, m.label),
                ControlKind::Button | ControlKind::Field { .. } => assert!(m.rect.height >= 28, "{p}: {:?} {:?}", m.kind, m.rect),
                _ => {}
            }
        }
        // NEGATIVE CONTROL: the same tree laid out for Zune's sizes is NOT big enough for kit parts (a 15 px box under a 56 px toggle).
        let tight = zune.ui.controls.values().filter(|(_, m)| matches!(m.kind, ControlKind::Checkbox { .. })).count();
        assert!(tight == 0 || zune.ui.controls.values().any(|(_, m)| matches!(m.kind, ControlKind::Checkbox { .. }) && m.rect.width < 56), "{p}");
    }
}

#[test]
fn a_host_group_profile_keeps_every_group_member_inside_the_panel_and_labels_with_their_controls() {
    let host = heirloom_ami68k_engine::mui::layout::Metrics { toggle: (56, 28), control_h: 28, pad_w: 24, label_right: Some(8), group_head: 28, group_inset: 12, label_gap: 12, spacing: (12, 8), win_pad: 16, ..Default::default() };
    let Some(a) = start_with("System/Snoopy", host) else { return };
    assert_laid_out("System/Snoopy", &a);
    let controls: Vec<_> = a.ui.controls.values().map(|(_, m)| m).collect();
    let groups: Vec<_> = controls.iter().filter(|m| matches!(m.kind, ControlKind::Group { .. })).collect();
    assert!(groups.len() >= 3, "Snoopy's framed groups");
    for g in &groups {
        let (r, _) = (g.rect, ());
        for m in controls.iter().filter(|m| !matches!(m.kind, ControlKind::Group { .. }) && m.rect.left >= r.left && m.rect.left + m.rect.width <= r.left + r.width && m.rect.top >= r.top && m.rect.top + m.rect.height <= r.top + r.height) {
            assert!(m.rect.top >= r.top + 28 + 12 && m.rect.left >= r.left + 12 && m.rect.left + m.rect.width <= r.left + r.width - 12 && m.rect.top + m.rect.height <= r.top + r.height - 12, "{:?} {:?} sits in the panel of {:?}: {:?}", m.kind, m.rect, g.kind, r);
        }
    }
    // the sliders and the match field arrive WITH the labels the guest gave them
    let named = controls.iter().filter(|m| matches!(m.kind, ControlKind::Slider { .. } | ControlKind::Field { .. })).count();
    assert!(named >= 5 && controls.iter().filter(|m| matches!(m.kind, ControlKind::Slider { .. } | ControlKind::Field { .. })).all(|m| !m.label.is_empty()));
}
