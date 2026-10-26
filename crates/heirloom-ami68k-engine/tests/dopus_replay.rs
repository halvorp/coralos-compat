//! (replay fixes) The real Directory Opus 5.102 (unmodified, from /data/build/corpus-bin; skipped when absent) run to its idle point on the
//! engine and checked against what the AROS m68k oracle saw (tests/oracle/dopus5/EXPECTATIONS.md): the processes it starts,
//! the ports it publishes, the windows it opens and that they were really drawn.
use heirloom_ami68k_engine::Amiga;
use heirloom_ui_abi::{DrawOp, PenRole, Plane, RecordingGuestUi, UiCall};

const DOPUS: &str = "/data/build/corpus-bin/dopus5/DOpus5";

fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = format!("{prefix}{}", e.file_name().to_string_lossy());
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path()); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

/// Boot DirectoryOpus and run it, advancing the virtual clock whenever every task sleeps, until `seconds` of virtual time passed.
#[allow(dead_code)]
fn run(seconds: u64) -> Option<Amiga<RecordingGuestUi>> { run_cfg(seconds, true, |_| {}) }

/// [`run`] with the plane route (the default of the engine): the windows are presented as display-list planes, not canvases.
fn run_plane(seconds: u64) -> Option<Amiga<RecordingGuestUi>> { run_route(seconds, true, true, |_| {}) }

/// [`run`] with the font choice explicit: with `topaz` the screen's font is topaz 8, as on the AROS oracle that EXPECTATIONS.md and the
/// region fixtures come from (AROS is the BEHAVIOUR reference, not the pixel target): window sizes follow the font DOpus reads from the
/// screen, so the oracle's geometry holds only under it. Without it the engine is as shipped: the Gute Form face and pens.
#[allow(dead_code)]
fn run_with(seconds: u64, topaz: bool) -> Option<Amiga<RecordingGuestUi>> { run_cfg(seconds, topaz, |_| {}) }

/// [`run`] with the host's `RecordingGuestUi` prepared first (e.g. with icon artwork).
#[allow(dead_code)]
fn run_prep(seconds: u64, prep: impl FnOnce(&mut RecordingGuestUi)) -> Option<Amiga<RecordingGuestUi>> { run_cfg(seconds, true, prep) }

/// The canvas route: the pixel-level checks below read `present_content` canvases.
#[allow(dead_code)]
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
    a.start_hunk_as(&file, "DirectoryOpus").unwrap();
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


/// The planes each native window last presented, keyed by the window's id.
fn last_planes(a: &Amiga<RecordingGuestUi>) -> std::collections::BTreeMap<u32, Vec<&Plane>> {
    let mut by: std::collections::BTreeMap<u32, Vec<&Plane>> = Default::default();
    for c in &a.ui.calls { if let UiCall::PresentPlane(w, p) = c { by.entry(w.id).or_default().push(p); } }
    by
}
fn texts<'a>(ps: &[&'a Plane]) -> Vec<&'a DrawOp> { ps.iter().flat_map(|p| p.ops.iter()).filter(|o| matches!(o, DrawOp::Text { .. })).collect() }

/// Replay fixes 2 and 3 on the real binary: a pen the guest hard-codes (pen 3, the FILLPEN of DrawInfo, DOpus's directory ink) is TEXT ink on
/// its surface, never the accent; the windows it opened without a title are named after the program; its own titles stay.
#[test]
fn dopus_text_is_text_ink_and_untitled_windows_are_named_after_the_program() {
    let Some(a) = run_plane(5) else { return };
    let rows: Vec<&DrawOp> = last_planes(&a).values().flat_map(|ps| texts(ps)).filter(|o| matches!(o, DrawOp::Text { string, .. } if string.starts_with("Env ") || string.starts_with("EnvArc"))).collect();
    assert!(!rows.is_empty(), "the lister's rows were drawn");
    assert!(rows.iter().all(|o| matches!(o, DrawOp::Text { pen_role: PenRole::Text, bg_role: Some(PenRole::Background), .. })), "directory rows are Text ink on the window background: {rows:?}");
    // NEGATIVE CONTROL: no replayed run is in an AREA role (the accent fill, the background, an edge): ink roles only.
    let every: Vec<&DrawOp> = last_planes(&a).values().flat_map(|ps| texts(ps)).collect();
    assert!(every.iter().all(|o| matches!(o, DrawOp::Text { pen_role: PenRole::Text | PenRole::FillText | PenRole::Shine | PenRole::Highlight, .. })), "{every:?}");
    let titles: Vec<String> = a.ui.calls.iter().filter_map(|c| if let UiCall::OpenWindow(m) = c { Some(m.title.clone()) } else { None }).collect();
    assert_eq!(titles, ["Directory Opus", "DirectoryOpus", "DirectoryOpus", "DirectoryOpus"], "NULL titles are the program's name, the guest's own title is kept");
}

/// Replay fix 1: every text run carries the guest's own advance (TextLength) so the host can fit it; the lister's mono rows are 8 px cells.
#[test]
fn every_replayed_run_carries_the_guests_run_width() {
    let Some(a) = run_plane(5) else { return };
    let every: Vec<&DrawOp> = last_planes(&a).values().flat_map(|ps| texts(ps)).collect();
    assert!(every.iter().all(|o| matches!(o, DrawOp::Text { string, width, .. } if *width > 0 || string.is_empty())), "{every:?}");
    let row = every.iter().find_map(|o| match o { DrawOp::Text { string, width, font_role, px, .. } if string.starts_with("Env    01-Jan") => Some((string.chars().count() as i32, *width, *font_role, *px)), _ => None }).expect("a lister row");
    assert_eq!((row.1, row.2, row.3), (row.0 * 8, heirloom_ui_abi::FontRole::Mono, 8), "topaz 8: mono, 8 px cells, so the run is chars x 8");
}

/// Replay fix 4: the backdrop and the button bank are NOT empty at the idle point: the guest drew into both, and each presented its own plane
/// (a host that shows them blank is not showing them, the engine is not at fault).
#[test]
fn the_backdrop_and_the_button_bank_are_presented_as_drawn_planes() {
    let Some(a) = run_plane(5) else { return };
    let planes = last_planes(&a);
    let (backdrop, bank) = (&planes[&2], &planes[&3]);
    let (bd, bk) = (texts(backdrop), texts(bank));
    assert!(bd.iter().any(|o| matches!(o, DrawOp::Text { string, .. } if string == "RAM")), "the backdrop shows its drive names");
    assert!(bk.iter().any(|o| matches!(o, DrawOp::Text { string, .. } if string == "Makedir")) && bank.iter().map(|p| p.ops.len()).max().unwrap() > 50, "the button bank draws its buttons");
    // NEGATIVE CONTROL: the loading window that closed before idle is not asked to be shown again, and window ids that never opened have no plane.
    assert!(!planes.contains_key(&9));
}
