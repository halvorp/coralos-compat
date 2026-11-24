//! Real-binary ladder replay tests for Mac 68k Rung 2–3 applications (`MISSION-native-heirloom.md`):
//! - **WriteNow 2.2** (commercial word processor): opens "Untitled-1" + "Ruler", normalizes multi-hyphen Mac dividers (`"--"`)
//!   into standard `"-"` separators across its 9-menu bar, updates dynamic menu items ("Print...", "Can't Undo", "Find/Replace..."),
//!   and parks cleanly in its event loop with zero refusals.
//! - **MacPaint 2.1F** (classic paint app): opens its personalization dialog (native `Button`/`Field`/`Label` controls), accepts
//!   user name input + `OK` activation, and enters its paint event loop with 4 `WindowRole::Utility` palettes (`Tool`, `Pattern`,
//!   `Spot`, `FatBits`) and zero refusals.
//! - **ResEdit 2.1.3** (complex multi-window resource editor): decompresses guest `CODE` resources via `'dcmp'`, publishes its
//!   `File`/`Edit`/`Resource`/`Window` menu bar with hierarchical submenus, and parks cleanly with zero refusals.
use heirloom_mac68k_engine::Mac;
use heirloom_ui_abi::{ControlKind, GuestUiEvent, Intent, RecordingGuestUi, UiCall, WindowRole};

fn run_mac(rel: &str, machine: &str, idle_ticks: u32) -> Option<Mac<RecordingGuestUi>> {
    let path = format!("/data/build/corpus-bin/{rel}");
    let Ok(file) = std::fs::read(&path) else {
        eprintln!("SKIPPED: {path} not present");
        return None;
    };
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.ui.drain_audio = true;
    m.set_machine(machine).unwrap();
    m.launch(&file).unwrap();
    let mut left = 40_000_000u64;
    let mut r = m.run_loop(left).unwrap();
    for _ in 0..idle_ticks {
        left = left.saturating_sub(r.stats.insns);
        if !r.stats.blocked || left == 0 { break; }
        m.advance_ticks(1);
        r = m.run_loop(left).unwrap();
    }
    Some(m)
}

#[test]
fn writenow_2_2_normalizes_multi_hyphen_menu_separators_and_opens_document_and_ruler() {
    let Some(m) = run_mac("mac/writenow/fx/z22_1/WriteNow 2.2.bin", "mac-plus", 300) else { return };
    assert!(m.refused.is_empty(), "WriteNow refusals: {:?}", m.refused);
    let titles: Vec<_> = m.ui.calls.iter().filter_map(|c| match c { UiCall::OpenWindow(w) => Some((w.title.clone(), w.role)), _ => None }).collect();
    assert!(titles.iter().any(|(t, r)| t == "Untitled-1" && *r == WindowRole::Document), "windows={titles:?}");
    assert!(titles.iter().any(|(t, _)| t == "Ruler"), "windows={titles:?}");

    let strip = m.ui.calls.iter().rev().find_map(|c| match c { UiCall::SetMenuStrip(_, s) => Some(s.clone()), _ => None }).expect("WriteNow menu strip");
    let win_menu = strip.menus.iter().find(|mm| mm.title == "Window").expect("Window menu");
    assert!(
        !win_menu.items.iter().any(|it| it.label == "--"),
        "Mac multi-hyphen dividers (\"--\") must normalize to \"-\" separators: {:?}",
        win_menu.items.iter().map(|i| (&i.label, i.enabled)).collect::<Vec<_>>()
    );
    assert!(win_menu.items.iter().filter(|it| it.label == "-" && !it.enabled).count() >= 3, "expected >= 3 normalized separators in Window menu");
    let file_menu = strip.menus.iter().find(|mm| mm.title == "File").expect("File menu");
    assert!(file_menu.items.iter().any(|i| i.label == "Print..."), "dynamic SetItem label updated");
}

#[test]
fn macpaint_2_1f_completes_personalization_dialog_and_enters_paint_event_loop() {
    let Some(mut m) = run_mac("mac/macpaint/u21/MacPaint 2.1F.rsrc", "mac-ii", 40) else { return };
    assert!(m.refused.is_empty(), "MacPaint startup refusals: {:?}", m.refused);
    // Enter user name in the first native text field and step so MacPaint enables its OK button, then activate OK.
    let field = m.ui.find_control(|c| matches!(c.kind, ControlKind::Field { editable: true, .. }));
    if let Some((fw, fc, fm)) = field {
        m.deliver(GuestUiEvent::intent(fw, fc, fm.token, Intent::CommitText("Coral User".into()))).unwrap();
        for _ in 0..15 {
            let r = m.run_loop(5_000_000).unwrap();
            if !r.stats.blocked { break; }
            m.advance_ticks(1);
        }
    }
    if let Some((bw, bc, bm)) = m.ui.find_control(|c| matches!(c.kind, ControlKind::Button) && c.label == "OK" && c.enabled) {
        m.deliver(GuestUiEvent::intent(bw, bc, bm.token, Intent::Activate)).unwrap();
        let mut left = 20_000_000u64;
        for _ in 0..80 {
            let r = m.run_loop(left).unwrap();
            left = left.saturating_sub(r.stats.insns);
            if !r.stats.blocked || left == 0 { break; }
            m.advance_ticks(1);
        }
    }
    assert!(m.unimplemented_at.is_empty(), "MacPaint hit unimplemented trap: {:?}", m.unimplemented_at);
    assert!(m.refused.is_empty(), "MacPaint refusals after OK: {:?}", m.refused);
    assert_eq!(m.windows.values().filter(|w| w.proc_id == 544).count(), 4, "4 WDEF 34 Utility palettes live");
}

#[test]
fn resedit_2_1_3_decompresses_code_and_publishes_hierarchical_menus_with_zero_refusals() {
    let Some(m) = run_mac("mac/resedit/u213/resedit_2.1.3.sea_/ResEdit.rsrc", "mac-ii", 60) else { return };
    assert!(m.refused.is_empty(), "ResEdit refusals: {:?}", m.refused);
    let strip = m.ui.calls.iter().rev().find_map(|c| match c { UiCall::SetMenuStrip(_, s) => Some(s.clone()), _ => None }).expect("ResEdit menu strip");
    let names: Vec<&str> = strip.menus.iter().map(|mm| mm.title.as_str()).collect();
    assert!(names.contains(&"File") && names.contains(&"Edit") && names.contains(&"Resource"), "menus={names:?}");
    let file_menu = strip.menus.iter().find(|mm| mm.title == "File").unwrap();
    assert!(file_menu.items.iter().any(|i| !i.sub_items.is_empty()), "ResEdit File menu carries hierarchical submenus");
}
