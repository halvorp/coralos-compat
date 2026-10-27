//! BaseConstructL opens the application window, titled with the caption its resource files give, through the GuestUi seam;
//! the run then stops at a named refusal. Needs the owner's corpus (skipped when absent): the images in
//! `/data/build/corpus-symbian/e32` and the extracted install trees in `.../drive`.
use heirloom_ekaarm_engine::drive::{Drive, DriveError};
use heirloom_ekaarm_engine::{loader, Outcome, Process};
use heirloom_fmt_e32::OrdinalTable;
use heirloom_fmt_rsc::{Reader, RscFile};
use heirloom_ui_abi::{RecordingGuestUi, UiCall, WindowRole};

use heirloom_ekaarm_engine::hostdir;

const ORD_TSV: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
const E32: &str = "/data/build/corpus-symbian/e32";
const DRIVE: &str = "/data/build/corpus-symbian/drive";

struct App {
    exe: &'static str,
    tree: &'static str,
    /// The localisable resource file and the id its registration gives (read here independently of the engine).
    loc_rsc: &'static str,
    loc_id: u32,
    caption: &'static str,
    refusal: &'static str,
}

const FILE_BROWSER: App = App { exe: "FileBrowser.exe", tree: "nokiafilebrowser", loc_rsc: "resource/apps/filebrowser.rsc", loc_id: 0x3ec9_005d, caption: "FileBrowser", refusal: "_ZN8CFileMan4NewLER3RFsP16MFileManObserver" };
const HERO: App = App { exe: "HeroOfSparta.exe", tree: "hero-of-sparta", loc_rsc: "resource/apps/heroofsparta.rsc", loc_id: 0x08f6_f004, caption: "HeroOfSparta", refusal: "_ZN5RLineC1Ev" };

fn load(app: &App) -> Option<Process> {
    let path = format!("{E32}/{}", app.exe);
    let bytes = std::fs::read(&path).ok().or_else(|| { println!("SKIP: {path} absent"); None })?;
    Some(loader::load(&bytes, &OrdinalTable::parse(ORD_TSV).unwrap()).unwrap())
}

fn drive_of(app: &App) -> Option<Drive> {
    let dir = format!("{DRIVE}/{}", app.tree);
    if !std::path::Path::new(&dir).is_dir() {
        println!("SKIP: {dir} absent");
        return None;
    }
    Some(hostdir::load_dir(std::path::Path::new(&dir)).unwrap().0)
}

/// What the resource file itself says the caption is: LOCALISABLE_APP_INFO's `caption_and_icon.caption` (appinfo.rh:176-202).
fn caption_per_resource(app: &App) -> String {
    let bytes = std::fs::read(format!("{DRIVE}/{}/{}", app.tree, app.loc_rsc)).unwrap();
    let info = RscFile::parse(&bytes).unwrap().resource(app.loc_id).unwrap();
    let mut r = Reader::new(&info);
    (r.long().unwrap(), r.long().unwrap(), r.ltext().unwrap(), r.long().unwrap(), r.long().unwrap());
    r.ltext().unwrap()
}

fn windows(p: &Process) -> Vec<String> {
    p.ui_as::<RecordingGuestUi>().unwrap().calls.iter().map(|c| match c {
        UiCall::OpenWindow(m) => { assert_eq!(m.role, WindowRole::Document); format!("{}@{}x{}", m.title, m.rect.width, m.rect.height) }
        other => panic!("only OpenWindow is expected, got {other:?}"),
    }).collect()
}

fn opens_its_window_and_stops_at_a_named_refusal(app: App) {
    let (Some(mut p), Some(drive)) = (load(&app), drive_of(&app)) else { return };
    p.set_drive(drive);
    p.set_ui(RecordingGuestUi::new());
    let outcome = p.run(50_000_000);
    let want = caption_per_resource(&app);
    println!("{}: caption \"{want}\" ({} instructions)", app.exe, p.insns);
    assert!(!want.is_empty());
    assert_eq!(want, app.caption);
    assert_eq!(windows(&p), vec![format!("{want}@360x640")], "exactly one OpenWindow, titled with the resource's caption");
    match outcome {
        Outcome::Refused(m) => {
            println!("{}: {m}", app.exe);
            assert!(m.starts_with("NEXT-NEEDED") && m.contains(app.refusal), "{m}");
        }
        o => panic!("expected a named refusal: {o:?}"),
    }
}

#[test]
fn file_browser_opens_its_window() {
    opens_its_window_and_stops_at_a_named_refusal(FILE_BROWSER);
}

#[test]
fn hero_of_sparta_opens_its_window() {
    opens_its_window_and_stops_at_a_named_refusal(HERO);
}

/// Run `app` with the given host objects and return the refusal text; no window may have been opened.
fn refused_without_window(app: App, drive: Option<Drive>, ui: bool) -> Option<String> {
    let mut p = load(&app)?;
    if let Some(d) = drive { p.set_drive(d); }
    if ui { p.set_ui(RecordingGuestUi::new()); }
    let outcome = p.run(50_000_000);
    assert!(p.host.window.is_none());
    if ui { assert!(windows(&p).is_empty(), "no window may open"); }
    match outcome {
        Outcome::Refused(m) => Some(m),
        o => panic!("expected a named refusal: {o:?}"),
    }
}

#[test]
fn negative_control_without_a_drive_baseconstructl_refuses_and_guesses_no_title() {
    for app in [FILE_BROWSER, HERO] {
        let Some(m) = refused_without_window(app, None, true) else { return };
        println!("no drive: {m}");
        assert!(m.starts_with("NEXT-NEEDED") && m.contains("BaseConstructL") && m.contains("no drive"), "{m}");
    }
}

#[test]
fn negative_control_a_drive_without_the_resource_files_refuses() {
    let mut empty = Drive::new();
    empty.put_file("\\sys\\bin\\readme.txt", b"no resources here".to_vec()).unwrap();
    let Some(m) = refused_without_window(HERO, Some(empty), true) else { return };
    println!("empty drive: {m}");
    assert!(m.contains("BaseConstructL") && m.contains("registration"), "{m}");
    // the registration is there but the localisable file it names is not
    let Some(mut drive) = drive_of(&HERO) else { return };
    drive.put_file("\\resource\\apps\\heroofsparta.rsc", b"not a resource file".to_vec()).unwrap();
    let Some(m) = refused_without_window(HERO, Some(drive), true) else { return };
    println!("corrupt resource file: {m}");
    assert!(m.contains("BaseConstructL") && m.contains("heroofsparta.rsc"), "{m}");
}

#[test]
fn negative_control_without_a_ui_refuses() {
    let Some(drive) = drive_of(&HERO) else { return };
    let Some(m) = refused_without_window(HERO, Some(drive), false) else { return };
    assert!(m.contains("BaseConstructL") && m.contains("no GuestUi"), "{m}");
}

#[test]
fn the_drive_is_case_insensitive_read_only_and_names_its_letters() {
    let mut d = Drive::new();
    d.put_file("\\Resource\\Apps\\X.RSC", vec![1, 2, 3]).unwrap();
    for p in ["z:\\resource\\apps\\x.rsc", "C:/RESOURCE/APPS/x.rsc", "\\resource\\apps\\x.rsc"] {
        assert_eq!(d.read(p).unwrap(), &[1, 2, 3], "{p}");
    }
    assert_eq!(d.read("e:\\resource\\apps\\x.rsc"), Err(DriveError::NoDrive('e')));
    assert_eq!(d.read("\\resource\\apps\\y.rsc"), Err(DriveError::NotFound));
    assert_eq!(d.read("\\resource"), Err(DriveError::IsDirectory));
    assert_eq!(d.list("\\resource").unwrap(), vec!["Apps".to_string()]);
}
