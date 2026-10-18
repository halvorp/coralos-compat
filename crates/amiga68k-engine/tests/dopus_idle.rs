//! The real Directory Opus 5.102 (unmodified, from /data/build/corpus-bin; skipped when absent) run to its idle point on the
//! engine and checked against what the AROS m68k oracle saw (tests/oracle/dopus5/EXPECTATIONS.md): the processes it starts,
//! the ports it publishes, the windows it opens and that they were really drawn.
use amiga68k_engine::Amiga;
use guestui_abi::{RecordingGuestUi, UiCall};

const DOPUS: &str = "/data/build/corpus-bin/dopus5/DOpus5";

fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = format!("{prefix}{}", e.file_name().to_string_lossy());
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path()); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

/// Boot DirectoryOpus and run it, advancing the virtual clock whenever every task sleeps, until `seconds` of virtual time passed.
fn run(seconds: u64) -> Option<Amiga<RecordingGuestUi>> {
    let file = std::fs::read(format!("{DOPUS}/DirectoryOpus")).ok().or_else(|| { eprintln!("skipped: corpus not present"); None })?;
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.add_device("DOpus5", false);
    load(&mut a, "DOpus5:", std::path::Path::new(DOPUS));
    a.fs.vol.assign_add("LIBS", "DOpus5:Libs");
    a.fs.vol.assign("PROGDIR", "DOpus5:");
    a.start_hunk(&file).unwrap();
    let mut insns = 0u64;
    while a.clock_us < seconds * 1_000_000 && insns < 200_000_000 {
        let (step, st) = a.step(2_000_000);
        insns += st.insns;
        match step {
            amiga68k_engine::Step::Blocked => match a.next_timer_deadline() { Some(t) => a.advance_clock_to(t).unwrap(), None => break },
            amiga68k_engine::Step::BudgetSpent => {}
            other => panic!("DirectoryOpus stopped: {other:?}\nrefused: {:?}", a.refused),
        }
    }
    Some(a)
}

/// Names on ExecBase.PortList (offset 392 of the ExecBase at 0x1000), read the way a guest would walk it.
fn public_ports(a: &Amiga<RecordingGuestUi>) -> Vec<String> {
    let mut out = vec![];
    let mut n = a.mem.read_u32(0x1000 + 392).unwrap();
    while a.mem.read_u32(n).unwrap() != 0 {
        let name = a.mem.read_u32(n + 10).unwrap();
        if name != 0 { out.push(a.mem.read_cstr(name).unwrap()); }
        n = a.mem.read_u32(n).unwrap();
    }
    out
}

fn distinct_pens(px: &[u32]) -> usize { let mut v = px.to_vec(); v.sort_unstable(); v.dedup(); v.len() }

#[test]
fn directory_opus_starts_its_processes_ports_and_windows() {
    let Some(a) = run(5) else { return };
    // Named refusals only, and only these: a strgclass method the boopsi lane has not written, and DOpus's own CloseLibrary(NULL).
    assert!(a.refused.iter().all(|r| r.contains("strgclass method 1") || r.contains("CloseLibrary: 0x0 is not an open library base")), "{:#?}", a.refused);

    // Processes: the order DOpus creates them in (EXPECTATIONS.md items 8-13), from the engine's own log.
    let created: Vec<&str> = a.log.iter().filter_map(|l| l.strip_prefix("[dos] CreateNewProc \"")).filter_map(|l| l.split('"').next()).collect();
    let first: Vec<&str> = created.iter().copied().take(8).collect();
    assert_eq!(first, ["DO_LAUNCHER", "dopus_progressbar", "dopus_rexx", "dopus_buttons", "dopus_lister", "dopus_clock", "dopus_pattern_stuff", "dopus_wbstartup"], "{created:?}");
    // At idle: the long-lived ones are alive; the progress bar and the two start-and-exit helpers have gone (as on the oracle).
    let alive: Vec<String> = a.sched.tasks.values().map(|t| t.name.clone()).collect();
    for want in ["program", "DO_LAUNCHER", "dopus_rexx", "dopus_buttons", "dopus_lister", "dopus_clock"] { assert!(alive.iter().any(|n| n == want), "{want} is not running: {alive:?}"); }
    for gone in ["dopus_progressbar", "dopus_pattern_stuff", "dopus_wbstartup"] { assert!(!alive.iter().any(|n| n == gone), "{gone} should have exited: {alive:?}"); }
    // The main program parks in Wait on its IPC port, as the oracle's main does.
    let main = &a.sched.tasks[&amiga68k_engine::process::PROCESS];
    assert!(matches!(main.waiting, Some(amiga68k_engine::exec_wait::Waiting::Signals(_))) && main.state == amiga68k_engine::sched::RunState::Wait, "main: {:?} {:?}", main.state, main.waiting);

    // Ports.
    let ports = public_ports(&a);
    assert!(ports.iter().any(|p| p == "Directory Opus") && ports.iter().any(|p| p == "DOPUS.1"), "{ports:?}");

    // Windows, in order: the progress window (closed again), the backdrop main window, the lister at the oracle's geometry.
    // (The oracle also lacks a button-bank window: its stub asl.library made dopus_buttons exit at once; ours opens one.)
    let opens: Vec<_> = a.ui.calls.iter().filter_map(|c| if let UiCall::OpenWindow(m) = c { Some(m.clone()) } else { None }).collect();
    let geo = |m: &guestui_abi::WindowModel| (m.rect.left, m.rect.top, m.rect.width, m.rect.height);
    assert_eq!((opens[0].title.as_str(), geo(&opens[0])), ("Directory Opus", (150, 100, 339, 55)), "progress window");
    assert_eq!((opens[1].title.as_str(), geo(&opens[1]), opens[1].closable), ("", (0, 15, 640, 241), false), "backdrop main window");
    let lister = opens.iter().find(|m| geo(m) == (209, 51, 443, 583)).expect("the lister window (EXPECTATIONS.md item 13)");
    assert!(lister.closable && lister.resizable);
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::CloseWindow(_))), "the progress window was closed");
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::SetWindowTitle(_, t) if t.contains("in use"))), "the lister shows its volume line");

    // They were drawn: frames were presented and the canvases hold more than one colour.
    let frames = a.ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. })).count();
    assert!(frames >= 10, "{frames} frames");
    let canvas = |w: i32, h: i32| a.ui.canvases.values().find(|(cw, ch, _)| (*cw, *ch) == (w, h)).unwrap_or_else(|| panic!("no {w}x{h} canvas: {:?}", a.ui.canvases.values().map(|c| (c.0, c.1)).collect::<Vec<_>>()));
    let main_px = &canvas(640, 241).2;
    let lister_px = &canvas(435, 564).2;
    assert!(distinct_pens(main_px) >= 3, "main window canvas is blank ({} colours)", distinct_pens(main_px));
    assert!(distinct_pens(lister_px) >= 3, "lister canvas is blank ({} colours)", distinct_pens(lister_px));
    // Negative control for the check itself: a canvas nobody drew on is one colour.
    assert_eq!(distinct_pens(&vec![0u32; 100]), 1);
}
