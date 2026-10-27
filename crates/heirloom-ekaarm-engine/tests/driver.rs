//! The engine driven through `GuestMachine` only. Corpus-backed tests skip when the install trees are absent.
use heirloom_common::{GuestMachine, Step};
use heirloom_ekaarm_engine::EkaArm;
use heirloom_ui_abi::{RecordingGuestUi, UiCall};
use heirloom_volume::Tree;
use std::path::Path;

const DRIVE: &str = "/data/build/corpus-symbian/drive";

fn fill(tree: &mut Tree, vol: usize, dir: &Path, guest: &[String]) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().into_string().unwrap();
        let mut path = guest.to_vec();
        if e.file_type().unwrap().is_dir() {
            path.push(name);
            fill(tree, vol, &e.path(), &path);
        } else {
            let d = tree.mkdirs(vol, path.iter().map(|s| s.as_str())).unwrap();
            tree.put_file(d, &name, std::fs::read(e.path()).unwrap()).unwrap();
        }
    }
}

fn run_app(tree: &str, exe: &str, caption: &str, fault: &str) {
    let root = format!("{DRIVE}/{tree}");
    if !Path::new(&root).is_dir() { println!("SKIP: {root} absent"); return; }
    let mut m: Box<dyn GuestMachine<Ui = RecordingGuestUi>> = Box::new(EkaArm::new(RecordingGuestUi::new(), ordinals()));
    let vol = m.volume().volume("Z").expect("a Z volume exists from new");
    fill(m.volume(), vol, Path::new(&root), &[]);
    let image = std::fs::read(format!("{root}/sys/bin/{exe}")).unwrap();
    m.start_file(&image, &format!("sys\\bin\\{exe}")).unwrap();
    let mut step = Step::BudgetSpent;
    for _ in 0..1000 {
        step = m.step(1_000_000).0;
        if step != Step::BudgetSpent { break; }
    }
    match &step {
        Step::Fault(msg) => assert!(msg.contains(fault), "{msg}"),
        other => panic!("{other:?}"),
    }
    let opens: Vec<_> = m.ui().calls.iter().filter_map(|c| match c { UiCall::OpenWindow(w) => Some(w.title.clone()), _ => None }).collect();
    assert_eq!(opens, [caption]);
    assert!(m.drain_refused().iter().any(|r| r.contains(fault)));
    assert!(m.drain_log().iter().any(|l| l.contains("started")));
    assert_eq!(m.step(10).0, step, "a fault is terminal");
}

/// The ordinal table the host reads from its bundle (test input here).
fn ordinals() -> heirloom_fmt_e32::OrdinalTable {
    heirloom_fmt_e32::OrdinalTable::parse(include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv")).unwrap()
}

#[test]
fn file_browser_through_the_trait() { run_app("nokiafilebrowser", "filebrowser.exe", "FileBrowser", "resource offset 0x8cc0000"); }

#[test]
fn hero_through_the_trait() { run_app("hero-of-sparta", "heroofsparta.exe", "HeroOfSparta", "CRepository::NewL(0x101f8798)"); }

#[test]
fn non_e32_bytes_are_a_named_refusal() {
    let mut m = EkaArm::new(RecordingGuestUi::new(), ordinals());
    let r = m.start_file(b"MZ this is not a Symbian image at all, just bytes", "x.exe").unwrap_err();
    assert!(r.0.contains("not a loadable E32 image"), "{r:?}");
    assert!(matches!(m.step(10).0, Step::Fault(_)), "nothing started");
    assert!(m.set_wall_clock(-1).is_err());
    assert!(m.set_wall_clock(1_700_000_000).is_ok());
    assert!(m.advance_clock_to(5).is_ok() && m.advance_clock_to(4).is_err());
    assert_eq!(m.next_timer_deadline(), None);
}
