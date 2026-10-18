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

#[test]
fn macintosh_garden_apps_stop_at_the_first_missing_trap_and_each_stop_is_named() {
    // Real shipped classic Mac applications (Macintosh Garden, owner-authorised local test use; docs/CORPUS.md, tools/corpus-apps-mac-garden.tsv).
    // Each pins how far the app gets TODAY; when a trap lands the stop moves on and the row is updated deliberately.
    for (app, stop) in [
        ("mac/macwrite/fx46/MacWrite.bin", "_Pack2 ($A9E9)"),
        ("mac/macwrite/u45/MacWrite.rsrc", "_SetTrapAddress ($A047)"),
        ("mac/macpaint/u21/MacPaint 2.1F.rsrc", "_UnloadScrap ($A9FA)"),
        ("mac/resedit/u213/resedit_2.1.3.sea_/ResEdit.rsrc", "_Alert ($A985)"),
        ("mac/simpletext/ux/SimpleText.rsrc", "_HFSDispatch ($A260)"),
        ("mac/simpletext/u13/SimpleText 1.3.rsrc", "_SetToolBoxTrapAddress ($A647)"),
        ("mac/glider-pro/fx/Glider PRO™.bin", "_SetToolBoxTrapAddress ($A647)"),
        ("mac/writenow/fx/Updaters/WriteNow4.01 to 4.02.bin", "_HFSDispatch ($A260)"),
        ("mac/writenow/fx40_1/Install WriteNow 4.0.bin", "_UnloadScrap ($A9FA)"),
    ] {
        let Some(m) = census(app) else { continue };
        // The LAST refusal is the stop; MacWrite 4.5 also records an earlier named refusal ("OpenPort: GrafPort: pointer ... outside
        // guest RAM": it hands OpenPort a pointer the engine rejects), pinned below so it is not lost.
        assert_eq!(m.refused.last(), Some(&format!("unimplemented trap {stop}")), "{app}: {:#?}", m.log);
    }
    let Some(m) = census("mac/macwrite/u45/MacWrite.rsrc") else { return };
    assert!(m.refused.iter().any(|r| r.starts_with("OpenPort: GrafPort: pointer")), "{:?}", m.refused);
    // NEGATIVE CONTROL: MacWrite 4.6 is past the Event/Resource startup, so it does not stop at the trap probe of its 4.5 sibling.
    let Some(m) = census("mac/macwrite/fx46/MacWrite.bin") else { return };
    assert!(!m.refused.iter().any(|r| r.contains("SetTrapAddress")), "{:?}", m.refused);
}

#[test]
fn writenow_2_x_runs_into_a_bus_fault_loop_after_its_second_code_segment_not_a_missing_trap() {
    // FINDING (docs/CORPUS.md): WriteNow 2.0 and 2.2 report NO unimplemented trap yet burn the whole 5M-instruction budget with
    // ~10M bus faults right after GetResource('CODE', 2): the app jumps through memory the loader never mapped. A silent hang
    // wearing a census row's clothes; this pins the symptom so the fix is a deliberate edit.
    for app in ["mac/writenow/fx/z20_0/WriteNow 2.0.bin", "mac/writenow/fx/z22_1/WriteNow 2.2.bin"] {
        let Some(m) = census(app) else { continue };
        assert!(m.refused.is_empty(), "{app}: {:?}", m.refused);
        assert!(m.log.iter().any(|l| l.contains("GetResource 'CODE' 2")), "{app}: {:#?}", m.log);
    }
}
