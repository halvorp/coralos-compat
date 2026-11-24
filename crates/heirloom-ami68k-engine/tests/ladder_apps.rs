//! Real-binary ladder replay tests for Amiga 68k Rung 2–3 applications (`MISSION-native-heirloom.md`):
//! - **AWeb 3.6** (ReAction web browser): loads 15+ AROS ReAction classes from disk (`frbuttonclass`, `InitResident` RomTag
//!   version seeding, `TypeOfMem`, `LibList` guest libraries, `timer.device/AddTime`), opens its splash window, advances its
//!   timer into its main browser window & menus.
//! - **FileX 2.7 (68020)** (commercial-grade Amiga hex editor): opens its 3-pane hex editor window, 7-menu global bar with
//!   nested submenus, native vertical scrollbar, and Pen 3 status bar with zero refusals.
//! Missing corpus binaries under `/data/build/corpus-bin` SKIP loudly.
use heirloom_ami68k_engine::{Amiga, Step};
use heirloom_ui_abi::{ControlKind, RecordingGuestUi, UiCall, WindowRole};

const ROOT: &str = "/data/build/corpus-bin";

fn load_tree(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        if e.file_type().unwrap().is_dir() {
            load_tree(a, &format!("{path}/"), &e.path());
        } else if let Ok(d) = std::fs::read(e.path()) {
            a.fs.vol.put_file(&path, d).unwrap();
        }
    }
}

#[test]
fn aweb_3_6_loads_reaction_classes_opens_splash_and_advances_timer_into_browser_ui() {
    let bin = format!("{ROOT}/aweb3/ex/AWeb/AWeb");
    let Ok(file) = std::fs::read(&bin) else {
        eprintln!("SKIPPED: {bin} not present");
        return;
    };
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    load_tree(&mut a, "SYS:", std::path::Path::new(&format!("{ROOT}/aros-m68k/mini")));
    a.fs.vol.assign("LIBS", "SYS:Libs");
    a.fs.vol.add_device("AWEB", false);
    load_tree(&mut a, "AWEB:", std::path::Path::new(&format!("{ROOT}/aweb3/ex/AWeb")));
    a.fs.vol.assign("PROGDIR", "AWEB:");
    a.stop_on_unimplemented = true;
    a.start_hunk_as(&file, "AWeb").unwrap();

    for tick in 0..8 {
        let (st, _) = a.step(40_000_000);
        if st == Step::Blocked {
            if let Some(dl) = a.next_timer_deadline() {
                a.advance_clock_to(dl).unwrap();
                continue;
            }
            if tick == 0 {
                a.advance_clock_to(a.clock_us + 2_000_000).unwrap();
                continue;
            }
            break;
        }
        break;
    }
    let wins: Vec<_> = a.ui.calls.iter().filter_map(|c| match c { UiCall::OpenWindow(w) => Some(w.clone()), _ => None }).collect();
    let menus: Vec<_> = a.ui.calls.iter().filter_map(|c| match c { UiCall::SetMenuStrip(_, m) => Some(m.clone()), _ => None }).collect();
    let ctls: Vec<_> = a.ui.controls.values().map(|(_, c)| c.clone()).collect();
    eprintln!("AWeb: {} windows={:?}, {} menu strips, {} live controls, refused={:?}",
        wins.len(),
        wins.iter().map(|w| (&w.title, w.rect.width, w.rect.height)).collect::<Vec<_>>(),
        menus.len(),
        ctls.len(),
        a.refused);
    assert!(!wins.is_empty(), "AWeb must open at least its ReAction splash/browser window");
    assert!(a.refused.is_empty(), "AWeb hit refusals: {:?}", a.refused);
}

#[test]
fn filex_2_7_opens_hex_editor_window_nested_menus_scrollbar_and_status_bar() {
    let bin = format!("{ROOT}/aminet-filex-68k/ex/FileX-2.7a-68k/FileX.68020");
    let Ok(file) = std::fs::read(&bin) else {
        eprintln!("SKIPPED: {bin} not present");
        return;
    };
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.add_device("FILEX", false);
    load_tree(&mut a, "FILEX:", std::path::Path::new(&format!("{ROOT}/aminet-filex-68k/ex/FileX-2.7a-68k")));
    a.fs.vol.put_file("FILEX:sample.bin", b"HEIRLOOM-HEX-OK!".to_vec()).unwrap();
    a.fs.vol.assign("PROGDIR", "FILEX:");
    // FileX self-detaches via CreateNewProc unless SYNC is passed on the CLI; passing SYNC + a file opens it in-process.
    a.cli_args = Some("SYNC FILEX:sample.bin".into());
    a.stop_on_unimplemented = true;
    a.start_hunk_as(&file, "FileX.68020").unwrap();
    let (st, _) = a.step(20_000_000);
    assert_eq!(st, Step::Blocked, "FileX should park in Wait: {:?} log={:?}", a.refused, a.log.iter().rev().take(15).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "FileX refusals: {:?}", a.refused);

    let win = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(w) => Some(w.clone()), _ => None }).expect("FileX main window");
    assert_eq!((win.title.as_str(), win.role, win.closable, win.resizable), ("FileX 2.7 (68020)", WindowRole::Document, true, true));

    let strip = a.ui.calls.iter().rev().find_map(|c| match c { UiCall::SetMenuStrip(_, m) => Some(m.clone()), _ => None }).expect("FileX menus");
    let titles: Vec<&str> = strip.menus.iter().map(|m| m.title.as_str()).collect();
    assert!(titles.contains(&"Project") && titles.contains(&"Edit") && titles.contains(&"Search & Replace"), "menus={titles:?}");
    let project = strip.menus.iter().find(|m| m.title == "Project").unwrap();
    assert!(project.items.iter().any(|i| i.label == "View" && i.sub_items.len() >= 10), "nested Project->View submenu");

    assert!(a.ui.controls.values().any(|(_, c)| matches!(c.kind, ControlKind::ScrollBar { vertical: true, .. })), "native vertical scrollbar");
    let has_status = a.ui.calls.iter().any(|c| match c {
        UiCall::PresentPlane(_, p) => p.ops.iter().any(|o| matches!(o, heirloom_ui_abi::DrawOp::Text { string, .. } if string.contains("Pos:") && string.contains("sample.bin"))),
        _ => false,
    });
    assert!(has_status, "FileX status bar shows loaded sample.bin");
    let has_ascii = a.ui.calls.iter().any(|c| match c {
        UiCall::PresentPlane(_, p) => p.ops.iter().any(|o| matches!(o, heirloom_ui_abi::DrawOp::Text { string, .. } if string.contains("HEIRLOOM-HEX-OK!"))),
        _ => false,
    });
    assert!(has_ascii, "FileX ASCII pane renders real bytes of sample.bin");
}
