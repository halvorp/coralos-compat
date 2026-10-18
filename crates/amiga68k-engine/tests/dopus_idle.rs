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
    // No refusal at all: DOpus's CloseLibrary(NULL) is a no-op and its ReplyFreeMsg of an IntuiMessage replies (the message names the WindowPort).
    assert!(a.refused.is_empty(), "{:#?}", a.refused);

    // The lister's file list: RAM: holds Env, EnvArc and T; each is drawn as a row of 48 characters (432 px), name first. Control: no row was
    // ever drawn zero characters wide (what a window with GZZWidth 0 gives).
    for name in ["Env    ", "EnvArc ", "T      "] { assert!(a.log.iter().any(|l| l.starts_with(&format!("[graphics] Text \"{name}")) && l.ends_with("(+432)")), "no list row for {name:?}"); }
    assert!(!a.log.iter().any(|l| l.starts_with("[graphics] Text \"\" at 5,")), "a file-list row was drawn zero characters wide");

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

    // EXPECTATIONS.md item 11: update.module is tried first (absent, both spellings), then the ten modules at version 73, in order.
    let modules: Vec<String> = a.log.iter().filter_map(|l| l.strip_prefix("[exec] OpenLibrary name=\"")).filter(|l| l.contains(".module\"")).map(|l| l.replace("\" ver=", " ").split(" ->").next().unwrap().to_string()).collect();
    let expect = ["PROGDIR:Modules/update.module 0", "DOpus5:Modules/update.module 0"].iter().map(|s| s.to_string())
        .chain(["cleanup", "diskinfo", "filetype", "fixicons", "ftp", "join", "misc", "pathformat", "themes", "xadopus"].iter().map(|m| format!("DOpus5:Modules/{m}.module 73"))).collect::<Vec<_>>();
    assert_eq!(modules, expect);
    // Items 8 and 10: FindPort before AddPort, "WORKBENCH" looked for, then DOPUS.1, then REXX.
    let ports_seq: Vec<&str> = a.log.iter().filter(|l| l.starts_with("[exec] FindPort") || l.starts_with("[exec] AddPort")).map(|l| l.as_str()).collect();
    assert_eq!(ports_seq.len(), 6, "{ports_seq:?}");
    assert!(ports_seq[0].contains("\"Directory Opus\" -> NULL") && ports_seq[1].contains("\"WORKBENCH\"") && ports_seq[2].starts_with("[exec] AddPort")
        && ports_seq[3].contains("\"DOPUS.1\" -> NULL") && ports_seq[4].starts_with("[exec] AddPort") && ports_seq[5].contains("\"REXX\" -> NULL"), "{ports_seq:?}");

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
    // DOPUS_DUMP=<dir> writes every window canvas as a PPM there, to look at next to tests/oracle/dopus5/screens.
    if let Ok(dir) = std::env::var("DOPUS_DUMP") {
        for (id, (w, h, px)) in &a.ui.canvases {
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
            std::fs::write(format!("{dir}/window{id}-{w}x{h}.ppm"), out).unwrap();
        }
    }
    let main_px = &canvas(640, 241).2;
    let lister_px = &canvas(435, 564).2;
    assert!(distinct_pens(main_px) >= 3, "main window canvas is blank ({} colours)", distinct_pens(main_px));
    assert!(distinct_pens(lister_px) >= 3, "lister canvas is blank ({} colours)", distinct_pens(lister_px));
    // Negative control for the check itself: a canvas nobody drew on is one colour.
    assert_eq!(distinct_pens(&vec![0u32; 100]), 1);
}

#[test]
#[ignore]
fn explore_idle_tasks() {
    let Some(a) = run(std::env::var("SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(5)) else { return };
    for (addr, t) in &a.sched.tasks { eprintln!("TASK {addr:#x} {} {:?} {:?}", t.name, t.state, t.waiting); }
    eprintln!("clock {} timers {:?}", a.clock_us, a.timers);
    for w in a.windows.keys() { eprintln!("WIN {w:#x} userport {:#x} idcmp {:#x}", a.mem.read_u32(w + 86).unwrap(), a.mem.read_u32(w + 82).unwrap()); }
    eprintln!("sem owners {:x?} nest {:x?}", a.sched.sem_owner, a.semaphores.iter().filter(|(_, &n)| n > 0).collect::<Vec<_>>());
}

/// A region fixture from the AROS oracle's screenshot (tests/oracle/dopus5/regions): rows of class characters.
fn region(name: &str) -> Vec<Vec<char>> {
    let t = std::fs::read_to_string(format!("{}/tests/oracle/dopus5/regions/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    t.lines().filter(|l| !l.starts_with('#') || l.starts_with("#W") || l.starts_with("##")).filter(|l| !l.starts_with("# ")).map(|l| l.chars().collect()).collect()
}

/// The class of one pixel of ours, in the fixture's alphabet (0x6688BB, black, any light pen; everything else '.').
fn class(p: u32) -> char {
    match p & 0xFF_FFFF { 0 => '#', 0x6688BB => 'B', c if (c >> 16) + ((c >> 8) & 0xFF) + (c & 0xFF) >= 700 => 'W', _ => '.' }
}

/// Compare a w x h patch of `canvas` at (cx, cy) with an oracle region ('?' = not compared); returns the mismatching positions.
fn mismatches(canvas: &(i32, i32, Vec<u32>), (cx, cy): (i32, i32), want: &[Vec<char>]) -> Vec<(usize, usize, char, char)> {
    let mut bad = vec![];
    for (j, row) in want.iter().enumerate() {
        for (i, &w) in row.iter().enumerate() {
            if w == '?' { continue; }
            let got = class(canvas.2[((cy as usize + j) * canvas.0 as usize) + cx as usize + i]);
            if got != w { bad.push((i, j, got, w)); }
        }
    }
    bad
}

#[test]
fn the_lister_matches_the_oracle_where_the_oracle_is_ground_truth() {
    let Some(a) = run(5) else { return };
    let lister = a.ui.canvases.values().find(|(w, h, _)| (*w, *h) == (435, 564)).expect("the lister canvas");
    // Regions compared (class structure; palettes differ: AROS's light pen is 0xEEEEEE, the engine's 0xFFFFFF):
    //  1. the "SRCE" button at the lister's top right: exact text bitmap, black frame and the 0x6688BB pen;
    //  2. the path field's 3D frame at the lister's bottom (outer white/black lines, the inner bevel), interior not compared.
    // NOT compared: the toolbar artwork (the cell frames are, region 3), the file list area (the oracle's list is empty: AROS stub libraries), the scroll bars and arrows (they sit in the window
    // border, outside the engine's canvas) and the disk icons (different volumes, different default icon artwork).
    let srce = region("lister-srce-48x10.txt");
    assert_eq!(srce.len(), 10, "{srce:?}");
    let bad = mismatches(lister, (387, 0), &srce);
    assert!(bad.is_empty(), "SRCE region differs from the oracle at {bad:?}");
    let frame = region("lister-pathfield-frame-24x12.txt");
    assert_eq!(frame.len(), 12);
    let bad = mismatches(lister, (0, 552), &frame);
    assert!(bad.is_empty(), "path field frame differs from the oracle at {bad:?}");
    // 3. the toolbar: the black separator above it and the 3D frames of its first two 26x26 cells (interiors, the brush artwork, are not compared).
    let cells = region("lister-toolbar-cells-52x27.txt");
    assert_eq!(cells.len(), 27);
    let bad = mismatches(lister, (0, 9), &cells);
    assert!(bad.is_empty(), "toolbar cell frames differ from the oracle at {bad:?}");
    assert!(!mismatches(lister, (1, 9), &cells).is_empty(), "control: the toolbar comparison shifted by one pixel must fail");
    // Negative control: shifting the SRCE comparison by one pixel must fail (the check is not vacuous).
    assert!(!mismatches(lister, (388, 0), &srce).is_empty());
}

#[test]
fn the_lister_registers_its_gadgets_as_native_controls() {
    use guestui_abi::{ArrowDir, ControlKind, GuestRect};
    let Some(a) = run(5) else { return };
    // The lister window (443x583 at 209,51) is the native window whose controls carry its border scrollbars and arrows, its drag strip
    // and the path field; before the Gute Form spike the engine painted buttongclass / propgclass / strgclass in the AROS look instead.
    let lister = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) if (m.rect.width, m.rect.height) == (443, 583) => Some(m.token), _ => None }).expect("the lister window");
    let native = *a.windows.get(&(lister.0 as u32)).expect("open lister");
    let mine: Vec<_> = a.ui.controls.values().filter(|(w, _)| *w == native).map(|(_, m)| m).collect();
    let rect = |l, t, w, h| GuestRect { left: l, top: t, width: w, height: h };
    let has = |k: &ControlKind, r: GuestRect| mine.iter().any(|m| &m.kind == k && m.rect == r);
    assert!(has(&ControlKind::ScrollBar { vertical: true, total: 3, visible: 63, top: 0 }, rect(439, 17, 4, 532)), "{mine:#?}");
    assert!(has(&ControlKind::ScrollBar { vertical: false, total: 280, visible: 432, top: 0 }, rect(4, 579, 405, 4)), "{mine:#?}");
    for (dir, r) in [(ArrowDir::Up, rect(439, 551, 4, 14)), (ArrowDir::Down, rect(439, 565, 4, 14)), (ArrowDir::Left, rect(411, 579, 14, 4)), (ArrowDir::Right, rect(425, 579, 14, 4))] {
        assert!(has(&ControlKind::ArrowButton { dir }, r), "arrow {dir:?}: {mine:#?}");
    }
    assert!(has(&ControlKind::Hotspot, rect(0, 15, 4, 564)), "the left-border drag strip is an invisible hit area");
    assert!(mine.iter().any(|m| matches!(&m.kind, ControlKind::Field { text, .. } if text == "RAM:") && m.rect == rect(8, 569, 427, 8)), "the path field (dopusstrgclass's super GM_RENDER): {mine:#?}");
    assert_eq!(mine.len(), 8);
    assert!(a.refused.is_empty(), "{:#?}", a.refused);
    // Negative control: the other windows (progress, backdrop) have no such controls.
    assert!(a.ui.controls.values().filter(|(w, _)| *w != native).all(|(_, m)| !matches!(m.kind, ControlKind::ScrollBar { .. })));
}
