//! The real Directory Opus 5.102 (unmodified, from /data/build/corpus-bin; skipped when absent) run to its idle point on the
//! engine and checked against what the AROS m68k oracle saw (tests/oracle/dopus5/EXPECTATIONS.md): the processes it starts,
//! the ports it publishes, the windows it opens and that they were really drawn.
use heirloom_ami68k_engine::Amiga;
use heirloom_ui_abi::IconRole;
use heirloom_ui_abi::{ControlKind, DrawOp, GuestPoint, GuestRect, PenRole, Plane, RecordingGuestUi, UiCall};

const DOPUS: &str = "/data/build/corpus-bin/dopus5/DOpus5";

fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = format!("{prefix}{}", e.file_name().to_string_lossy());
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path()); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

/// Boot DirectoryOpus and run it, advancing the virtual clock whenever every task sleeps, until `seconds` of virtual time passed.
fn run(seconds: u64) -> Option<Amiga<RecordingGuestUi>> { run_cfg(seconds, true, |_| {}) }

/// [`run`] with the plane route (the default of the engine): the windows are presented as display-list planes, not canvases.
fn run_plane(seconds: u64) -> Option<Amiga<RecordingGuestUi>> { run_route(seconds, true, true, |_| {}) }

/// [`run`] with the font choice explicit: with `topaz` the screen's font is topaz 8, as on the AROS oracle that EXPECTATIONS.md and the
/// region fixtures come from (AROS is the BEHAVIOUR reference, not the pixel target): window sizes follow the font DOpus reads from the
/// screen, so the oracle's geometry holds only under it. Without it the engine is as shipped: the Gute Form face and pens.
fn run_with(seconds: u64, topaz: bool) -> Option<Amiga<RecordingGuestUi>> { run_cfg(seconds, topaz, |_| {}) }

/// [`run`] with the host's `RecordingGuestUi` prepared first (e.g. with icon artwork).
fn run_prep(seconds: u64, prep: impl FnOnce(&mut RecordingGuestUi)) -> Option<Amiga<RecordingGuestUi>> { run_cfg(seconds, true, prep) }

/// The canvas route: the pixel-level checks below read `present_content` canvases.
fn run_cfg(seconds: u64, topaz: bool, prep: impl FnOnce(&mut RecordingGuestUi)) -> Option<Amiga<RecordingGuestUi>> { run_route(seconds, topaz, false, prep) }

fn run_route(seconds: u64, topaz: bool, plane: bool, prep: impl FnOnce(&mut RecordingGuestUi)) -> Option<Amiga<RecordingGuestUi>> {
    let file = std::fs::read(format!("{DOPUS}/DirectoryOpus")).ok().or_else(|| {
        // A gate that must prove the claim sets HEIRLOOM_REQUIRE_CORPUS: an absent corpus is then a failure, not a silent pass.
        assert!(std::env::var_os("HEIRLOOM_REQUIRE_CORPUS").is_none(), "HEIRLOOM_REQUIRE_CORPUS is set but {DOPUS} is absent");
        eprintln!("skipped: corpus not present"); None
    })?;
    let mut ui = RecordingGuestUi::new();
    prep(&mut ui);
    let mut a = Amiga::new(ui).unwrap();
    a.gfx.present.set_plane_route(plane);
    a.fs.vol.add_device("DOpus5", false);
    load(&mut a, "DOpus5:", std::path::Path::new(DOPUS));
    a.fs.vol.assign_add("LIBS", "DOpus5:Libs");
    a.fs.vol.assign("PROGDIR", "DOpus5:");
    a.start_hunk(&file).unwrap();
    if topaz { use_topaz(&mut a); }
    let mut insns = 0u64;
    while a.clock_us < seconds * 1_000_000 && insns < 200_000_000 {
        let (step, st) = a.step(2_000_000);
        insns += st.insns;
        match step {
            heirloom_ami68k_engine::Step::Blocked => match a.next_timer_deadline() { Some(t) => a.advance_clock_to(t).unwrap(), None => break },
            heirloom_ami68k_engine::Step::BudgetSpent => {}
            other => panic!("DirectoryOpus stopped: {other:?}\nrefused: {:?}", a.refused),
        }
    }
    Some(a)
}

/// The screen's font as the AROS oracle had it: GfxBase.DefaultFont, DrawInfo.dri_Font and Screen.Font (TextAttr "topaz.font" 8) = topaz 8.
fn use_topaz(a: &mut Amiga<RecordingGuestUi>) {
    a.mem.write_u32(0x3000 + 154, 0x8000).unwrap();
    a.mem.write_u32(0x8E00 + 8, 0x8000).unwrap();
    a.mem.write_u32(0x6400 + 84 + 52, 0x8000).unwrap(); // Screen.RastPort.rp_Font
    for (i, b) in b"topaz.font\0".iter().enumerate() { a.mem.write_u8(0x6570 + i as u32, *b).unwrap(); }
    a.mem.write_u16(0x6560 + 4, 8).unwrap();
    // Gute Form's SHINEPEN == SHADOWPEN (one flat hairline); the oracle fixtures tell light edges from dark, so they run with a distinct SHINEPEN (pen 2).
    a.mem.write_u16(0x8E40 + 2 * 3, 2).unwrap();
}

/// DOPUS_DUMP=<dir> writes every window canvas as a PPM there (`<tag>-window<id>-<w>x<h>.ppm`), to look at next to tests/oracle/dopus5/screens.
fn dump_canvases(a: &Amiga<RecordingGuestUi>, tag: &str) {
    let Ok(dir) = std::env::var("DOPUS_DUMP") else { return };
    for (id, (w, h, px)) in &a.ui.canvases {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
        std::fs::write(format!("{dir}/{tag}-window{id}-{w}x{h}.ppm"), out).unwrap();
    }
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
    let main = &a.sched.tasks[&heirloom_ami68k_engine::process::PROCESS];
    assert!(matches!(main.waiting, Some(heirloom_ami68k_engine::exec_wait::Waiting::Signals(_))) && main.state == heirloom_ami68k_engine::sched::RunState::Wait, "main: {:?} {:?}", main.state, main.waiting);

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
    let geo = |m: &heirloom_ui_abi::WindowModel| (m.rect.left, m.rect.top, m.rect.width, m.rect.height);
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
    dump_canvases(&a, "topaz-oracle-geometry");
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

/// The class of one pixel of ours, in the fixture's alphabet, by ROLE: '#' the Gute Form text or border colour (the oracle's black was both the text pen and SHADOWPEN, which Gute Form tells apart), 'B' the accent (the
/// oracle's 0x6688BB fill), 'W' the shine (the oracle's light pen); everything else '.'. The fixtures hold GEOMETRY; the colours are ours.
fn class(p: u32) -> char {
    use heirloom_theme::GUTE_FORM as T;
    match p & 0xFF_FFFF { c if c == T.text.packed() || c == T.border.packed() => '#', c if c == T.accent_press.packed() => 'B', c if c == T.shine.packed() => 'W', _ => '.' }
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
    // Regions compared (class structure by role; the palettes are not compared: AROS's are not the target):
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

/// Host icon artwork reaches DOpus: its three volume icons are GetDefDiskObject(WBDISK) defaults, so the host is asked for Disk art.
/// (DOpus puts them in gadgets; the engine does not paint GADGIMAGE gadget Images into the canvas, so the canvas dump is unchanged.)
#[test]
fn directory_opus_disk_icons_come_from_the_host_when_it_has_artwork() {
    let art: Vec<u32> = (0..32 * 32u32).map(|i| { let (x, y) = (i % 32, i / 32); if (x + y) % 8 < 2 { 0xFF00_0000 } else if x > 4 && x < 28 && y > 6 { 0xFF44_88DD } else { 0 } }).collect();
    let Some(a) = run_prep(5, |ui| { for r in [IconRole::Disk, IconRole::Drawer, IconRole::Device, IconRole::Tool, IconRole::Project, IconRole::Trash, IconRole::Kick] { ui.icon_art.insert(r, (32, 32, art.clone())); } }) else { return };
    let icon_calls: Vec<&String> = a.log.iter().filter(|l| l.starts_with("[icon]")).collect();
    eprintln!("DOpus icon calls: {icon_calls:#?}\nhost requests: {:?}", a.ui.icon_requests);
    assert!(a.ui.icon_requests.iter().any(|(r, ..)| *r == IconRole::Disk), "DOpus's volume icons were asked of the host: {:?}", a.ui.icon_requests);
    assert!(icon_calls.iter().any(|l| l.contains("[host artwork: Disk]")), "{icon_calls:?}");
    if let Ok(dir) = std::env::var("DOPUS_DUMP") {
        for (id, (w, h, px)) in &a.ui.canvases {
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in px { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
            std::fs::write(format!("{dir}/hostart-window{id}-{w}x{h}.ppm"), out).unwrap();
        }
    }
}

#[test]
fn the_lister_registers_its_gadgets_as_native_controls() {
    use heirloom_ui_abi::{ArrowDir, ControlKind, GuestRect};
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

#[test]
fn as_shipped_directory_opus_draws_in_gute_form_colours_and_the_gute_form_face() {
    use heirloom_theme::{amiga::AROS_SYSTEM_PALETTE, GUTE_FORM as T};
    let (Some(shipped), Some(aros_font)) = (run_with(5, false), run(5)) else { return };
    assert!(shipped.refused.is_empty(), "{:#?}", shipped.refused);
    dump_canvases(&shipped, "guteform");
    // Structure, not geometry: the same windows open in the same order with the same titles and flags (their SIZES follow the font DOpus reads).
    let opens = |a: &Amiga<RecordingGuestUi>| -> Vec<(String, bool, bool)> { a.ui.calls.iter().filter_map(|c| if let UiCall::OpenWindow(m) = c { Some((m.title.clone(), m.closable, m.resizable)) } else { None }).collect() };
    assert_eq!(opens(&shipped), opens(&aros_font), "the window set is the font-independent structure");
    // Colours: the lister canvas is dominated by the window surface and carries the text, shine and accent roles; no AROS system grey survives.
    let lister = shipped.ui.canvases.values().max_by_key(|(w, h, _)| w * h).expect("a canvas");
    let count = |c: u32| lister.2.iter().filter(|&&p| p & 0xFF_FFFF == c).count();
    assert!(count(T.surface.packed()) > lister.2.len() / 4, "the surface is the ground: {} of {}", count(T.surface.packed()), lister.2.len());
    assert!(count(T.text.packed()) > 0 && count(T.shine.packed()) > 0, "text and shine are drawn");
    // The AROS system pens do not survive in the lister's chrome. (A few pixels of 0xAAAAAA / 0x6688BB remain: DOpus's own icon artwork carries
    // literal colours; that is the icon lane's, hence a bound of 0.1% and not zero.)
    for aros in [AROS_SYSTEM_PALETTE[0], AROS_SYSTEM_PALETTE[3], AROS_SYSTEM_PALETTE[4], AROS_SYSTEM_PALETTE[5]] { assert!(count(aros) < lister.2.len() / 1000, "AROS colour {aros:#x} is {} px of the lister", count(aros)); }
    // Negative control for the check itself: a canvas in the AROS grey exceeds the bound.
    let grey_canvas = vec![AROS_SYSTEM_PALETTE[0]; 1000];
    assert!(grey_canvas.iter().filter(|&&p| p == AROS_SYSTEM_PALETTE[0]).count() >= grey_canvas.len() / 1000);
    // The face: the file list rows are drawn in the proportional Gute Form face (Text advance = the face's own TextLength), not at topaz's 8 px pitch.
    let f = heirloom_raster::font::Font::gute_form();
    let rows: Vec<&String> = shipped.log.iter().filter(|l| l.starts_with("[graphics] Text \"")).collect();
    assert!(!rows.is_empty());
    let proportional = rows.iter().any(|l| {
        let text = l.strip_prefix("[graphics] Text \"").and_then(|t| t.split('"').next()).unwrap_or("");
        l.ends_with(&format!("(+{})", f.text_length(text.as_bytes()))) && f.text_length(text.as_bytes()) != 8 * text.len() as i32
    });
    assert!(proportional, "some drawn text advanced by the Gute Form TextLength: {:?}", &rows[..rows.len().min(5)]);
}

/// The bounding box of an op (window coordinates), `None` for a text run (its width is the host's font's).
fn op_box(o: &DrawOp) -> Option<GuestRect> {
    let line = |a: &GuestPoint, b: &GuestPoint| GuestRect { left: a.x.min(b.x), top: a.y.min(b.y), width: (a.x - b.x).abs() + 1, height: (a.y - b.y).abs() + 1 };
    match o {
        DrawOp::Fill { rect, .. } | DrawOp::Bevel { rect, .. } | DrawOp::Icon { rect, .. } | DrawOp::Pixels { rect, .. } => Some(*rect),
        DrawOp::Line { a, b, .. } => Some(line(a, b)),
        DrawOp::Text { .. } => None,
    }
}
/// Do the two rectangles share any pixel (full containment is not required: a PARTLY covered pixel op is still under the control)?
fn overlaps(o: &GuestRect, c: &GuestRect) -> bool { o.left < c.left + c.width && c.left < o.left + o.width && o.top < c.top + c.height && c.top < o.top + o.height }
fn inside(o: &GuestRect, c: &GuestRect) -> bool { o.left >= c.left && o.top >= c.top && o.left + o.width <= c.left + c.width && o.top + o.height <= c.top + c.height }

/// First slice item 3 (17-TRANSLATION-ARCHITECTURE.md): the real DOpus lister is a PLANE. Its rows are text runs with pen ROLES, its
/// native scrollbars / path field are controls with no drawing under them, and no list role is invented from the rows.
#[test]
fn the_lister_is_a_plane_of_row_text_runs_with_pen_roles_and_no_ops_under_its_native_controls() {
    let Some(a) = run_plane(5) else { return };
    assert!(a.refused.is_empty(), "{:#?}", a.refused);
    let lister = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) if (m.rect.width, m.rect.height) == (443, 583) => Some(m.token), _ => None }).expect("the lister window");
    let native = *a.windows.get(&(lister.0 as u32)).expect("open lister");
    assert_eq!(a.ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. })).count(), 0, "the plane route replaces present_content");
    let planes: Vec<&Plane> = a.ui.calls.iter().filter_map(|c| match c { UiCall::PresentPlane(w, p) if *w == native => Some(p), _ => None }).collect();
    assert!(!planes.is_empty() && planes.iter().all(|p| p.validate().is_ok()), "the lister was presented as valid planes");
    let ops: Vec<&DrawOp> = planes.iter().flat_map(|p| p.ops.iter()).collect();

    // The rows: DOpus draws "Env    ", "EnvArc ", "T      " (name column, then date and protection) with Text(); each row is ONE text run
    // with a pen ROLE (no pen number exists in the seam) over the BACKGROUNDPEN, drawn in JAM2.
    for name in ["Env    ", "EnvArc ", "T      "] {
        let run = ops.iter().find_map(|o| match o { DrawOp::Text { string, pen_role, bg_role, .. } if string.starts_with(name) => Some((*pen_role, *bg_role)), _ => None });
        let Some((_, bg)) = run else { panic!("no text run for row {name:?}; texts: {:?}", ops.iter().filter_map(|o| if let DrawOp::Text { string, .. } = o { Some(string.trim_end()) } else { None }).filter(|s| !s.is_empty()).take(12).collect::<Vec<_>>()) };
        assert_eq!(bg, Some(PenRole::Background), "row {name:?} is drawn over the window background");
    }
    let named_texts = ops.iter().filter(|o| matches!(o, DrawOp::Text { .. })).count();
    assert!(named_texts >= 3, "{named_texts}");

    // Nothing is published under a native control: the host draws the scrollbars, arrows, drag strip and path field itself.
    let controls: Vec<GuestRect> = a.ui.controls.values().filter(|(w, _)| *w == native).map(|(_, m)| m.rect).collect();
    assert_eq!(controls.len(), 8, "{controls:?}");
    let under = |ops: &[&DrawOp]| ops.iter().filter_map(|o| op_box(o)).filter(|b| controls.iter().any(|c| inside(b, c))).count();
    assert_eq!(under(&ops), 0, "ops lie inside native control rectangles");
    // NEGATIVE CONTROL for the check: a fill hand-placed inside the path field IS found.
    let planted = DrawOp::Fill { rect: GuestRect { left: 10, top: 570, width: 5, height: 4 }, pen_role: PenRole::Fill };
    assert_eq!(under(&[&planted]), 1);
    // HEIRLOOM-NATIVE-IS-A-MEASURED-CLAIM: no GUEST PIXEL is presented under a native role, not even one that only overlaps it. (A named
    // op that merely overlaps is theme-role geometry the host paints beneath the control; a Pixels op is the guest's own picture.)
    let pixel_boxes: Vec<GuestRect> = ops.iter().filter_map(|o| if let DrawOp::Pixels { rect, .. } = o { Some(*rect) } else { None }).collect();
    assert!(!pixel_boxes.is_empty(), "the lister carries pixel holes (its disk icon art, the cleared list area), so the check below is not vacuous");
    let under_px = |boxes: &[GuestRect]| boxes.iter().filter(|b| controls.iter().any(|c| overlaps(b, c))).count();
    assert_eq!(under_px(&pixel_boxes), 0, "guest pixels overlap a native control: {pixel_boxes:?} vs {controls:?}");
    let straddling = [GuestRect { left: controls[0].left - 2, top: controls[0].top - 2, width: 4, height: 4 }];
    assert_eq!(under_px(&straddling), 1, "negative control: a pixel op straddling a control's edge is found, though it is not inside it");
    assert_eq!(under(&[&DrawOp::Pixels { rect: straddling[0], rgb: vec![0; 16] }]), 0, "and full containment alone would have missed it");

    // HEIRLOOM-ROLES-REQUIRE-A-CONTRACT negative control: rows that look like a list are NOT a list. The lister's controls are exactly the
    // system gadgets it made (scrollbars, arrows, the drag strip, the path field); no control kind names a list, and no row text is a control label.
    let kinds: Vec<String> = a.ui.controls.values().filter(|(w, _)| *w == native).map(|(_, m)| format!("{:?}", m.kind)).collect();
    assert!(kinds.iter().all(|k| !k.contains("List")), "{kinds:?}");
    assert!(a.ui.controls.values().filter(|(w, _)| *w == native).all(|(_, m)| !m.label.starts_with("Env") && !matches!(&m.kind, ControlKind::Label { text, .. } if text.starts_with("Env"))));

    // The census: 8 native controls and ONE plane, and it is RAW: the lister's disk icon art and cleared list area are pixel holes. The
    // census is read from what the host still shows (every frame a later one did not fully repaint), and it must agree with the
    // pixel ops actually presented, which are counted here independently from the call log.
    let c = a.ui.route_census(native);
    eprintln!("DOpus lister route census: {c:?}; {} ops in {} frames", ops.len(), planes.len());
    assert_eq!((c.native, c.replayed, c.raw), (8, 0, 1), "{c:?}");
    assert!(ops.iter().any(|o| o.is_raw()), "raw=1 is because pixel holes were presented");
}
