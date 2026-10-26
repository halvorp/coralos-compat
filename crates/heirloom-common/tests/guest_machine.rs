//! The ONE driver surface, exercised ONLY through `GuestMachine`: every program below is started from a file in the volume,
//! driven by the same generic pump, and observed through `ui()`. The Amiga, Mac 68k and Mac PowerPC engines behave as one.
//! Real corpus applications (not committed: /data/build/corpus-bin) SKIP loudly when absent.
use heirloom_ui_abi::{CallbackToken, GuestUiEvent, RecordingGuestUi, UiCall, UiObjectRef};
use heirloom_common::{Delivery, GuestMachine, Step};
use heirloom_volume::ROOT;

type Ui = RecordingGuestUi;
const CORPUS: &str = "/data/build/corpus-bin";
const SLICE: u64 = 1_000_000;

/// The pump a host runs, minus the timers: step until the guest is not simply out of budget.
fn rest(m: &mut dyn GuestMachine<Ui = Ui>) -> Step {
    for _ in 0..200 {
        match m.step(SLICE).0 { Step::BudgetSpent => continue, other => return other }
    }
    panic!("the guest never came to rest");
}

fn windows(m: &dyn GuestMachine<Ui = Ui>) -> Vec<String> {
    m.ui().calls.iter().filter_map(|c| match c { UiCall::OpenWindow(w) => Some(w.title.clone()), _ => None }).collect()
}

/// Stage `file` as `name` on a volume of the machine (a new one called `volume`, else the machine's first) and read it back:
/// how a host gets the bytes it starts from.
fn from_volume(m: &mut dyn GuestMachine<Ui = Ui>, volume: Option<&str>, name: &str, file: &[u8]) -> Vec<u8> {
    let tree = m.volume();
    let vol = match volume { Some(v) => tree.add_volume(v, true), None => tree.children(ROOT)[0] };
    let node = tree.put_file(vol, name, file.to_vec()).expect("a file stages on the volume");
    tree.file_data(node).expect("and reads back").to_vec()
}

fn amiga() -> heirloom_ami68k_engine::Amiga<Ui> { heirloom_ami68k_engine::Amiga::new(RecordingGuestUi::new()).unwrap() }
fn mac() -> heirloom_mac68k_engine::Mac<Ui> { heirloom_mac68k_engine::Mac::new(RecordingGuestUi::new()).unwrap() }
fn ppc() -> heirloom_macppc_engine::MacPpc<Ui> { heirloom_macppc_engine::MacPpc::new(RecordingGuestUi::new()).unwrap() }

/// The native close request for the guest's first window, and what it comes to: the program ends.
fn close_ends_the_program(m: &mut dyn GuestMachine<Ui = Ui>) {
    let win = UiObjectRef { id: 1, generation: 1 };
    let d = m.deliver(GuestUiEvent::CloseRequested { win, token: CallbackToken(0) }).unwrap();
    assert_eq!(d, Delivery::Queued);
    assert_eq!(rest(m), Step::Exited, "{:?}", m.drain_refused());
}

fn amimon() -> Vec<u8> { std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../heirloom-ami68k-engine/tests/corpus/amimon.020")).unwrap() }

#[test]
fn the_amiga_fixture_and_a_real_amiga_program_reach_their_windows_through_the_trait() {
    let mut a = amiga();
    let file = from_volume(&mut a, Some("Work"), "Hello", &heirloom_ami68k_engine::fixture::build_hunk(None));
    a.start_file(&file, "Hello").unwrap();
    assert_eq!(rest(&mut a), Step::Exited, "the fixture opens its window, sets a menu strip and closes");
    assert_eq!(windows(&a), ["Hello"]);
    assert!(a.drain_refused().is_empty());

    // amimon: a real GadTools program; it parks in Wait on its timer with the window open, and a native close ends it.
    let mut a = amiga();
    a.set_stop_on_unimplemented(true);
    let file = from_volume(&mut a, Some("Work"), "amimon", &amimon());
    a.start_file(&file, "amimon").unwrap();
    assert_eq!(rest(&mut a), Step::Blocked);
    assert_eq!(windows(&a).len(), 1);
    assert!(a.next_timer_deadline().is_some(), "it waits on its timer");
    close_ends_the_program(&mut a);
    assert!(a.drain_refused().is_empty());
    assert!(!a.drain_log().is_empty(), "the bridged calls were logged");
    assert!(a.drain_log().is_empty(), "and draining empties the log");
}

#[test]
fn the_mac_68k_fixture_reaches_its_window_from_every_container_and_closes_on_the_close_box() {
    let mut m = mac();
    let file = from_volume(&mut m, None, "Test App", &heirloom_mac68k_engine::fixture::test_app_macbinary());
    m.start_file(&file, "Test App").unwrap();
    assert_eq!(rest(&mut m), Step::Blocked, "{:?}", m.drain_refused());
    assert_eq!(windows(&m), ["Test Window"]);
    assert!(m.drain_refused().is_empty());
    close_ends_the_program(&mut m);

    // a raw resource fork carries no name: the one given to start() names the application
    let mut m = mac();
    m.start_file(&heirloom_mac68k_engine::fixture::test_app_fork(), "Raw Fork").unwrap();
    assert_eq!(rest(&mut m), Step::Blocked);
    assert_eq!(windows(&m), ["Test Window"]);
    assert_eq!(m.fm.tree.children(m.fm.def_dir).iter().map(|&n| m.fm.tree.name(n)).collect::<Vec<_>>(), ["System Folder", "Raw Fork"], "the boot volume's System Folder, and the application's own file");
}

#[test]
fn the_mac_ppc_fixture_reaches_its_window_from_a_macbinary_and_from_a_bare_pef() {
    let pef = heirloom_macppc_engine::fixture::startup_app();
    let mut m = ppc();
    let file = from_volume(&mut m, None, "Startup", &heirloom_macppc_engine::fixture::macbinary_app(&pef));
    m.start_file(&file, "Startup").unwrap();
    assert_eq!(rest(&mut m), Step::Blocked, "{:?}", m.drain_refused());
    assert_eq!(windows(&m), ["PPC Hello"]);
    assert!(m.drain_refused().is_empty());
    close_ends_the_program(&mut m);

    let mut m = ppc();
    m.start_file(&pef, "Startup").unwrap();
    assert_eq!(rest(&mut m), Step::Blocked);
    assert_eq!(windows(&m), ["PPC Hello"]);
}

#[test]
fn a_file_the_platform_cannot_run_is_refused_by_name_and_an_appledouble_header_needs_its_data_fork() {
    assert!(amiga().start_file(b"not a hunk", "x").is_err());
    assert!(!mac().start_file(&[0u8; 200], "x").unwrap_err().0.is_empty());
    let e = ppc().start_file(&heirloom_macppc_engine::fixture::appledouble_app(), "Startup").unwrap_err();
    assert!(e.0.contains("data fork"), "one file cannot carry the PEF of an AppleDouble header: {e:?}");
}

#[test]
fn the_mac_profile_is_chosen_at_construction_and_an_unknown_one_is_refused_by_name() {
    let app = heirloom_mac68k_engine::fixture::test_app_macbinary();
    for (name, cpu, machine) in [("mac-ii", 2u32, 6u32), ("mac-plus", 0, 4)] {
        let mut m = heirloom_mac68k_engine::Mac::with_profile(RecordingGuestUi::new(), name).unwrap();
        assert_eq!((m.profile.gestalt_cpu, m.profile.gestalt_machine), (cpu, machine));
        m.start_file(&app, "Test App").unwrap();
        assert_eq!(rest(&mut m), Step::Blocked, "{name}: {:?}", m.drain_refused());
        assert_eq!(windows(&m), ["Test Window"], "{name}");
    }
    assert_eq!(mac().profile.name, "mac-ii", "the default");
    let e = heirloom_mac68k_engine::Mac::with_profile(RecordingGuestUi::new(), "mac-xyzzy").err().unwrap();
    assert!(e.0.contains("unknown machine profile") && e.0.contains("mac-plus"), "{e:?}");
}

/// A blocked wait wakes at exactly its deadline in virtual microseconds: one microsecond earlier no guest instruction runs.
fn wakes_exactly_at_its_deadline(m: &mut dyn GuestMachine<Ui = Ui>, want_us: Option<u64>) {
    assert_eq!(rest(m), Step::Blocked);
    let due = m.next_timer_deadline().expect("a sleeping guest waits on time");
    if let Some(w) = want_us { assert_eq!(due, w); }
    m.advance_clock_to(due - 1).unwrap();
    let (s, st) = m.step(SLICE);
    assert_eq!((s, st.insns), (Step::Blocked, 0), "a microsecond early: the wait holds");
    assert_eq!(m.next_timer_deadline(), Some(due), "and the deadline did not move");
    m.advance_clock_to(due).unwrap();
    let (s, st) = m.step(SLICE);
    assert!(st.insns > 0 && s == Step::Blocked, "at the deadline the guest ran and waited again: {s:?} {st:?}");
    let next = m.next_timer_deadline().expect("it sleeps again");
    assert!(next > due, "the next wait is later: {next} after {due}");
    m.advance_clock_to(due).unwrap(); // the clock never runs backwards
    assert_eq!(m.step(SLICE), (Step::Blocked, heirloom_common::RunStats::default()), "and going back wakes nothing");
}

#[test]
fn a_sleeping_event_call_is_woken_by_the_virtual_microsecond_clock_on_the_tick_boundary() {
    // WaitNextEvent(sleep = 60 ticks) at tick 0: 60 ticks is exactly one second of the 60 Hz machine.
    let mut m = mac();
    m.start_file(&heirloom_mac68k_engine::fixture::test_app_macbinary(), "Test App").unwrap();
    wakes_exactly_at_its_deadline(&mut m, Some(1_000_000));
    let mut m = ppc();
    m.start_file(&heirloom_macppc_engine::fixture::startup_app(), "Startup").unwrap();
    wakes_exactly_at_its_deadline(&mut m, Some(1_000_000));
    // an Amiga timer.device request, the same contract in the machine's own unit
    let mut a = amiga();
    a.start_file(&amimon(), "amimon").unwrap();
    wakes_exactly_at_its_deadline(&mut a, None);
}

#[test]
fn a_wait_is_not_lost_when_the_host_jumps_the_clock_far_past_it() {
    for mut m in [Box::new(mac_started()) as Box<dyn GuestMachine<Ui = Ui>>, Box::new(ppc_started())] {
        assert_eq!(rest(&mut *m), Step::Blocked);
        let jump = 10 * 60 * 1_000_000; // ten virtual minutes in one call
        m.advance_clock_to(jump).unwrap();
        assert_eq!(rest(&mut *m), Step::Blocked);
        let due = m.next_timer_deadline().unwrap();
        assert!(due > jump && due <= jump + 1_000_000 + 16_667, "re-armed relative to the new time, one sleep ahead: {due}");
    }
}

fn mac_started() -> heirloom_mac68k_engine::Mac<Ui> {
    let mut m = mac();
    m.start_file(&heirloom_mac68k_engine::fixture::test_app_macbinary(), "Test App").unwrap();
    m
}
fn ppc_started() -> heirloom_macppc_engine::MacPpc<Ui> {
    let mut m = ppc();
    m.start_file(&heirloom_macppc_engine::fixture::startup_app(), "Startup").unwrap();
    m
}

// ---- real applications -------------------------------------------------------------------------------------------------

fn corpus(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("{CORPUS}/{rel}")).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {CORPUS}/{rel} not present (docs/CORPUS.md)"); None })
}

#[test]
fn a_real_mac_68k_application_reaches_its_document_window_through_the_trait() {
    let Some(file) = corpus("mac/macwrite/fx46/MacWrite.bin") else { return };
    let mut m = mac();
    m.set_stop_on_unimplemented(true);
    m.start_file(&file, "MacWrite").unwrap();
    assert_eq!(rest(&mut m), Step::Blocked, "{:#?}", m.drain_log());
    assert_eq!(windows(&m), ["Untitled"]);
    assert!(m.ui().calls.iter().any(|c| matches!(c, UiCall::SetMenuStrip(..))));
}

#[test]
fn a_real_mac_powerpc_application_reaches_its_window_through_the_trait() {
    let Some(file) = corpus("mac-retro68/ppc/HelloWorld.bin") else { return };
    let mut m = ppc();
    m.set_stop_on_unimplemented(true);
    m.start_file(&file, "HelloWorld").unwrap();
    // It opens its console window and waits in its event loop: NGetTrapAddress, where it used to stop, answers from the trap
    // registry since Mixed Mode slice 1 (docs/MACPPC-PLUMBING.md), and the answer is in the log.
    assert_eq!(rest(&mut m), Step::Blocked, "{:#?}", m.drain_log());
    assert_eq!(windows(&m), ["Retro68 Console"]);
    assert!(m.drain_log().iter().any(|l| l.contains("GetTrapAddress")), "the availability query ran");
}
