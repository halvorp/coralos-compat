//! The corpus lane's 68k census on REAL applications (not committed: /data/build/corpus-bin; docs/CORPUS.md). A missing binary
//! SKIPS loudly. Each test pins how far the app gets TODAY and moves on deliberately when a trap lands.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::Mac;

fn census(rel: &str) -> Option<Mac<RecordingGuestUi>> { census_in(rel, mac68k_engine::addrmode::AddrMode::Bit24) }

fn census_in(rel: &str, mode: mac68k_engine::addrmode::AddrMode) -> Option<Mac<RecordingGuestUi>> {
    let path = format!("/data/build/corpus-bin/{rel}");
    let file = std::fs::read(&path).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); None })?;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.set_address_mode(mode).unwrap();
    m.launch(&file).unwrap();
    m.run_loop(5_000_000).unwrap();
    Some(m)
}

#[test]
fn retro68_68k_samples_get_through_their_startup_and_the_trap_patch_and_stop_at_the_next_missing_trap() {
    // The Retro68 runtime asks GetToolTrapAddress/GetOSTrapAddress what exists (traptable.rs) and then PATCHES a Toolbox trap
    // (_SetToolBoxTrapAddress, now modelled: the patch routine runs and chains to the original). HelloWorld then runs to its
    // event loop and OPENS ITS WINDOW; Raytracer stops at _Button; Dialog at _SelectDialogItemText.
    for (app, stop) in [("Raytracer", "_Button ($A974)"), ("Dialog", "_SelectDialogItemText ($A97E)")] {
        let Some(m) = census(&format!("mac-retro68/68k/{app}.bin")) else { return };
        assert_eq!(m.refused, vec![format!("unimplemented trap {stop}")], "{app}: {:#?}", m.log);
        if app != "Dialog" { assert!(m.log.iter().any(|l| l.contains("GetTrapAddress")), "{app}: the startup probe was answered: {:#?}", m.log); }
    }
    let Some(m) = census("mac-retro68/68k/HelloWorld.bin") else { return };
    assert!(m.refused.is_empty(), "HelloWorld reaches its event loop with no refusal: {:?}", m.refused);
    assert!(m.log.iter().any(|l| l.contains("SetTrapAddress Toolbox")), "the runtime's trap patch was installed: {:#?}", m.log);
    assert!(m.ui.calls.iter().any(|c| format!("{c:?}").contains("OpenWindow")), "HelloWorld opened its window");
    // NEGATIVE CONTROL: before traptable.rs the same samples stopped AT the probe, so no GetTrapAddress line existed.
    assert!(!m.refused.iter().any(|r| r.contains("GetToolBoxTrapAddress") || r.contains("GetOSTrapAddress")), "{:?}", m.refused);
}

#[test]
fn the_toolbox_showcase_fat_app_runs_past_the_file_manager_to_the_colour_window_manager() {
    // systemless's Toolbox Showcase (68k CODE in the resource fork, cut out by tools/corpus-extract-showcase.py): it used to stop at
    // the HFS dispatcher (_HFSDispatch $A260); the File Manager exists now and the next wall is _GetNewCWindow (Color QuickDraw).
    let Some(m) = census("systemless-showcase/showcase.rsrc") else { return };
    assert_eq!(m.refused, vec!["unimplemented trap _GetNewCWindow ($AA46)".to_string()], "{:#?}", m.log);
}

#[test]
fn macintosh_garden_apps_stop_at_the_first_missing_trap_and_each_stop_is_named() {
    // Real shipped classic Mac applications (Macintosh Garden, owner-authorised local test use; docs/CORPUS.md, tools/corpus-apps-mac-garden.tsv).
    // Each pins how far the app gets TODAY; when a trap lands the stop moves on and the row is updated deliberately.
    for (app, stop) in [
        ("mac/macwrite/fx46/MacWrite.bin", "unimplemented trap _SetCursor ($A851)"),
        ("mac/macwrite/u45/MacWrite.rsrc", "unimplemented trap _SetCursor ($A851)"),
        ("mac/macpaint/u21/MacPaint 2.1F.rsrc", "unimplemented trap _SetWRefCon ($A918)"),
        ("mac/simpletext/ux/SimpleText.rsrc", "unimplemented trap _OSDispatch ($A88F)"),
        ("mac/glider-pro/fx/Glider PRO™.bin", "unimplemented trap _Pack7 ($A9EE)"),
        ("mac/writenow/fx/Updaters/WriteNow4.01 to 4.02.bin", "unimplemented trap _Pack7 ($A9EE)"),
        ("mac/writenow/fx40_1/Install WriteNow 4.0.bin", "unimplemented trap _GetFNum ($A900)"),
        ("mac/writenow/fx/z20_0/WriteNow 2.0.bin", "unimplemented trap _Pack6 ($A9ED)"),
        ("mac/writenow/fx/z22_1/WriteNow 2.2.bin", "unimplemented trap _Pack6 ($A9ED)"),
        ("mac/glider-pro/fx/Utilities_Glider Edit 1.1.1.bin", "unimplemented trap _ParamText ($A98B)"),
        ("mac/glider-pro/fx/Utilities_Moving Van.bin", "unimplemented trap _GetNewCWindow ($AA46)"),
    ] {
        let Some(m) = census(app) else { continue };
        assert_eq!(m.refused.last(), Some(&stop.to_string()), "{app}: {:#?}", m.log);
    }
    // Two apps now run on to a SEGMENT LOADER refusal, not a missing trap: they walk their own CODE resources and ReleaseResource a
    // segment that is still resident (their UnloadSeg is accepted but segments stay resident, segment.rs); the next lane's finding.
    for app in ["mac/resedit/u213/resedit_2.1.3.sea_/ResEdit.rsrc", "mac/simpletext/u13/SimpleText 1.3.rsrc"] {
        let Some(m) = census(app) else { continue };
        assert!(m.refused.last().is_some_and(|r| r.starts_with("LoadSeg: LoadSeg") && r.contains("entries at jump-table offset")), "{app}: {:?}", m.refused);
    }
    // MacWrite 4.5 no longer records the OpenPort refusal: its flagged GrafPort pointer (0x8020a9fa) is a good address on a 24-bit machine.
    let Some(m) = census("mac/macwrite/u45/MacWrite.rsrc") else { return };
    assert!(!m.refused.iter().any(|r| r.starts_with("OpenPort")), "{:?}", m.refused);
    assert!(m.log.iter().any(|l| l.contains("OpenPort 0x20a9fa")), "OpenPort was called with the stripped pointer: {:#?}", m.log);
    // NEGATIVE CONTROL: on a 32-bit machine the same pointer is outside RAM, as before the 24-bit work.
    let Some(m) = census_in("mac/macwrite/u45/MacWrite.rsrc", mac68k_engine::addrmode::AddrMode::Bit32) else { return };
    assert!(m.refused.iter().any(|r| r.starts_with("OpenPort: GrafPort: pointer")), "{:?}", m.refused);
}

#[test]
fn writenow_2_x_ran_into_a_bus_fault_loop_because_it_jumps_through_a_flagged_return_address() {
    // FINDING (docs/CORPUS.md): WriteNow 2.0 and 2.2 burned the whole 5M-instruction budget with ~10M bus faults after
    // GetResource('CODE', 2). THE CAUSE (found with the recent-PCs ring): its segment-call glue builds a RETURN ADDRESS and sets the high
    // byte to $80 (`move.l (a0),-(sp); addq.l #4,(sp); move.b #$80,(sp)`), then returns through it with `RTR`; the 24-bit bus of the machine
    // it was written for ignores that byte. On a 32-bit bus the PC became $80xxxxxx (RAM is 16 MiB) and every fetch faulted.
    for app in ["mac/writenow/fx/z20_0/WriteNow 2.0.bin", "mac/writenow/fx/z22_1/WriteNow 2.2.bin"] {
        let Some(m) = census(app) else { continue };
        assert!(m.log.iter().any(|l| l.contains("GetResource 'CODE' 2")), "{app}: {:#?}", m.log);
        let (stats, last) = (m.unimplemented_at.last().copied(), m.refused.last().cloned());
        assert!(stats.is_some() && last.as_deref() == Some("unimplemented trap _Pack6 ($A9ED)"), "{app}: it runs on to the International Utilities package: {last:?}");
        // NEGATIVE CONTROL: the 32-bit machine reproduces the old symptom (no trap reached; the budget spent on bus faults).
        let Some(c) = census_in(app, mac68k_engine::addrmode::AddrMode::Bit32) else { continue };
        assert!(c.refused.is_empty() && c.unimplemented_at.is_empty(), "{app}: the 32-bit run never reaches a trap, it stays in the fault loop: {:?}", c.refused);
    }
}
