//! Slice 4, input: a host key event reaches FileBrowser's list box through the environment's own path (window server event ->
//! `CCoeEnv::RunL` -> the AppUi's `HandleWsEventL` -> its control stack -> the container's `OfferKeyEventL` -> the list box's) and the
//! app waits in its scheduler afterwards. The owner's corpus; skipped when absent.
use heirloom_common::{Delivery, GuestMachine, Step};
use heirloom_ekaarm_engine::cdlextract;
use heirloom_ekaarm_engine::listbox::{LB_I_VIEW, VIEW_I_CURRENT};
use heirloom_ekaarm_engine::EkaArm;
use heirloom_ui_abi::{ControlKind, ControlValue, GuestUiEvent, Modifiers, RecordingGuestUi, UiCall, UiObjectRef};
use heirloom_volume::Tree;
use std::path::Path;

const TREE: &str = "/data/refs/heirloom/symbian-uiresources/layouts/aknlayout2";
const RSG: &str = "/data/build/corpus-symbian/sdk/sdk/CPP_API/epoc32/include/avkon.rsg";
const UDEB: &str = "/data/build/corpus-symbian/sdk/sdk/Emulator/epoc32/release/winscw/udeb";
const DRIVE: &str = "/data/build/corpus-symbian/drive/nokiafilebrowser";
/// The HOST's key codes (GuestUiEvent::Key.code is the host's code, heirloom-ui-abi event.rs:64): CoralOS key-wire bytes,
/// coral-keymap-abi lib.rs (WIRE_DOWN 0x0E, WIRE_UP 0x10) and Enter '\r'. The engine maps them to Symbian codes (keymap.rs).
const HOST_KEY_DOWN: u32 = 0x0E;
const HOST_KEY_UP: u32 = 0x10;
const HOST_KEY_ENTER: u32 = 0x0D;

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

struct App {
    eka: EkaArm<RecordingGuestUi>,
    win: UiObjectRef,
    list: u32,
}

impl App {
    fn start() -> Option<App> {
        let (qhd, rom) = (format!("{UDEB}/10283389.dll"), format!("{UDEB}/z"));
        if ![TREE, DRIVE, &rom].iter().all(|d| Path::new(d).is_dir()) || ![RSG, &qhd].iter().all(|f| Path::new(f).is_file()) {
            println!("SKIP: corpus absent");
            return None;
        }
        let text = cdlextract::extract(Path::new(TREE), "Qhd_touch_akn_app", Some(Path::new(RSG)), Some(Path::new(&qhd))).expect("extract").0;
        let ordinals = heirloom_fmt_e32::OrdinalTable::parse(include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv")).unwrap();
        let mut eka = EkaArm::new(RecordingGuestUi::new(), ordinals);
        eka.set_rom(Box::new(heirloom_ekaarm_engine::hostdir::DirRom::new(Path::new(&rom)).unwrap()));
        eka.set_cdl_data(text.into_bytes());
        let vol = eka.volume().volume("Z").unwrap();
        fill(eka.volume(), vol, Path::new(DRIVE), &[]);
        let image = std::fs::read(format!("{DRIVE}/sys/bin/filebrowser.exe")).unwrap();
        eka.start_file(&image, "sys\\bin\\filebrowser.exe").unwrap();
        let mut app = App { eka, win: UiObjectRef { id: 0, generation: 0 }, list: 0 };
        app.settle();
        let (win, list) = app.eka.ui().calls.iter().find_map(|c| match c {
            UiCall::AddControl(w, m) if matches!(m.kind, ControlKind::List { .. }) => Some((*w, m.token.0 as u32)),
            _ => None,
        }).expect("the list control was added");
        app.win = win;
        app.list = list;
        Some(app)
    }

    /// Step until the machine waits; returns the last slice's stats. The step MUST end Blocked.
    fn settle(&mut self) -> heirloom_common::RunStats {
        let mut total = heirloom_common::RunStats::default();
        for _ in 0..100 {
            let (step, stats) = self.eka.step(1_000_000);
            total.insns += stats.insns;
            match step {
                Step::BudgetSpent => continue,
                Step::Blocked => return total,
                other => panic!("{other:?}\n{:?}", self.eka.drain_refused()),
            }
        }
        panic!("never blocked");
    }

    /// The guest list box's current item index, read from the guest's own `CListBoxView`.
    fn current(&self) -> u32 {
        let p = self.eka.process().unwrap();
        let view = p.guest_word(self.list + LB_I_VIEW).unwrap();
        p.guest_word(view + VIEW_I_CURRENT).unwrap()
    }

    fn key(&mut self, scan: u32, down: bool) -> Delivery {
        self.eka.deliver(GuestUiEvent::Key { win: self.win, code: scan, text: String::new(), modifiers: Modifiers::default(), down }).unwrap()
    }

    /// Every `Rows` value the host was sent for the list, in order: (selected).
    fn selections(&self) -> Vec<Option<u32>> {
        self.eka.ui().calls.iter().filter_map(|c| match c {
            UiCall::SetControl(_, _, ControlValue::Rows { selected, .. }) => Some(*selected),
            _ => None,
        }).collect()
    }
}

#[test]
fn down_arrow_moves_the_list_selection_once_and_the_app_waits_again() {
    let Some(mut app) = App::start() else { return };
    assert_eq!(app.current(), 0, "the app starts on its first drive");
    let before = app.selections();
    assert_eq!(app.key(HOST_KEY_DOWN, true), Delivery::Queued);
    let stats = app.settle();
    assert!(stats.insns > 0, "the key ran guest code");
    assert_eq!(app.current(), 1, "the guest's own list box moved to the second drive");
    let after = app.selections();
    assert_eq!(after.len(), before.len() + 1, "the host was told exactly once");
    assert_eq!(after.last(), Some(&Some(1)));
    // The key release is routed too, changes nothing and tells the host nothing.
    assert_eq!(app.key(HOST_KEY_DOWN, false), Delivery::Queued);
    app.settle();
    assert_eq!(app.current(), 1);
    assert_eq!(app.selections(), after);
    // Idle means idle: with nothing delivered a step runs no instruction and stays Blocked (no polling).
    let (step, stats) = app.eka.step(1_000_000);
    assert_eq!((step, stats.insns), (Step::Blocked, 0));
}

#[test]
fn the_list_stops_at_its_ends_and_up_goes_back() {
    let Some(mut app) = App::start() else { return };
    app.key(HOST_KEY_UP, true);
    app.settle();
    assert_eq!(app.current(), 0, "Up on the first item stays");
    for _ in 0..3 {
        app.key(HOST_KEY_DOWN, true);
        app.settle();
    }
    assert_eq!(app.current(), 1, "Down on the last item stays");
    app.key(HOST_KEY_UP, true);
    app.settle();
    assert_eq!(app.current(), 0);
    assert_eq!(app.selections().last(), Some(&Some(0)));
}

#[test]
fn a_key_for_another_window_is_not_the_guests_and_an_unimplemented_path_is_a_named_terminal_fault() {
    let Some(mut app) = App::start() else { return };
    let stranger = UiObjectRef { id: app.win.id + 99, generation: 1 };
    let ev = GuestUiEvent::Key { win: stranger, code: HOST_KEY_DOWN, text: String::new(), modifiers: Modifiers::default(), down: true };
    assert_eq!(app.eka.deliver(ev).unwrap(), Delivery::NotForGuestWindow);
    assert_eq!(app.eka.step(1_000_000).0, Step::Blocked, "nothing was queued");
    assert_eq!(app.current(), 0);
    // Enter opens a drive: FileBrowser's own code goes on from here, into calls this engine has not implemented yet (named, terminal).
    app.key(HOST_KEY_ENTER, true);
    match app.eka.step(1_000_000).0 {
        Step::Fault(m) => assert!(m.starts_with("NEXT-NEEDED"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(!app.eka.drain_refused().is_empty());
}
