//! The corpus lane's 68k census on REAL applications (not committed: /data/build/corpus-bin; docs/CORPUS.md). A missing binary
//! SKIPS loudly. Each test pins how far the app gets TODAY and moves on deliberately when a trap lands.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::Mac;

fn census(rel: &str) -> Option<Mac<RecordingGuestUi>> {
    let path = format!("/data/build/corpus-bin/{rel}");
    let file = std::fs::read(&path).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); None })?;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.launch(&file).unwrap();
    m.run_loop(5_000_000).unwrap();
    Some(m)
}

#[test]
fn retro68_68k_samples_get_through_their_startup_trap_probe_and_stop_at_the_next_missing_trap() {
    // The Retro68 runtime asks GetToolTrapAddress/GetOSTrapAddress what exists (traptable.rs) and then patches a Toolbox trap
    // (_SetToolBoxTrapAddress $A647, not modelled): seven of the eight samples stop there; Dialog (GetNewDialog and ModalDialog now exist: the Dialog Manager lane) stops at _SelectDialogItemText.
    for (app, stop) in [("HelloWorld", "_SetToolBoxTrapAddress ($A647)"), ("Raytracer", "_SetToolBoxTrapAddress ($A647)"), ("Dialog", "_SelectDialogItemText ($A97E)")] {
        let Some(m) = census(&format!("mac-retro68/68k/{app}.bin")) else { return };
        assert_eq!(m.refused, vec![format!("unimplemented trap {stop}")], "{app}: {:#?}", m.log);
        if app != "Dialog" { assert!(m.log.iter().any(|l| l.contains("GetTrapAddress")), "{app}: the startup probe was answered: {:#?}", m.log); }
    }
    // NEGATIVE CONTROL: before traptable.rs the same samples stopped AT the probe, so no GetTrapAddress line existed.
    let Some(m) = census("mac-retro68/68k/HelloWorld.bin") else { return };
    assert!(!m.refused.iter().any(|r| r.contains("GetToolBoxTrapAddress") || r.contains("GetOSTrapAddress")), "{:?}", m.refused);
}

#[test]
fn the_toolbox_showcase_fat_app_runs_its_68k_half_to_the_file_manager() {
    // systemless's Toolbox Showcase (68k CODE in the resource fork, cut out by tools/corpus-extract-showcase.py): past the
    // trap probe it reaches the HFS dispatcher (_HFSDispatch $A260) after ~46k instructions.
    let Some(m) = census("systemless-showcase/showcase.rsrc") else { return };
    assert_eq!(m.refused, vec!["unimplemented trap _HFSDispatch ($A260)".to_string()], "{:#?}", m.log);
}
