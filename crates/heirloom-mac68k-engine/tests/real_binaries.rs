//! Real, unmodified classic Mac applications. The binaries are NOT in this repository: they live in the Executor source
//! tree (MIT-licensed; `packages/go` is ARDI's own "go" launcher, AppleDouble header file `%go.appl` holding its
//! resource fork). Set `MAC68K_CORPUS` to that tree's root (default `/data/refs/heirloom/executor`); a missing tree SKIPS
//! the test loudly instead of failing, so a checkout without the references still builds.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::Mac;

fn corpus(rel: &str) -> Option<Vec<u8>> {
    let root = std::env::var("MAC68K_CORPUS").unwrap_or_else(|_| "/data/refs/heirloom/executor".into());
    match std::fs::read(format!("{root}/{rel}")) {
        Ok(b) => Some(b),
        Err(e) => { eprintln!("SKIPPED: real binary {root}/{rel} not available ({e})"); None }
    }
}

#[test]
fn go_runs_its_think_c_startup_the_init_sequence_and_its_dialog_setup_and_waits_for_events() {
    let Some(file) = corpus("packages/go/%go.appl") else { return };
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.launch(&file).unwrap();
    let r = m.run_loop(2_000_000).unwrap();
    assert!(r.stats.blocked && r.stats.bus_faults == 0, "{:?}", r.stats);
    // The THINK C startup: ZERO / DATA / DREL resources fill the A5 world (it copies from CurStackBase up to A5, so the
    // world must start exactly at CurStackBase), then segment 6 is loaded through the jump table.
    let log = m.log.join("\n");
    for needle in ["GetResource 'ZERO' 0", "GetResource 'DATA' 0", "GetResource 'DREL' 0", "LoadSeg 6", "InitGraf", "InitWindows", "InitMenus", "InitDialogs"] {
        assert!(log.contains(needle), "{needle}: {log}");
    }
    assert_eq!(m.mem.read_u32(0x908).unwrap(), m.seg.a5 - 12524, "CurStackBase is the bottom of go's 12524-byte below-A5 world");
    // Nothing is missing any more: go sets its dialog's user items (SetDialogItem) and its controls' action procedures (SetControlAction),
    // then waits for events. Its only refusals are `HandToHand(NIL)` for the `icl4` icon it does not have (ResErr -192): the call answers
    // memWZErr, as on a real Mac, and go carries on.
    assert!(m.unimplemented_at.is_empty(), "{:?}", m.refused);
    assert!(m.refused.iter().all(|r| r.starts_with("HandToHand: argument is not a block")), "{:?}", m.refused);
    assert!(m.log.iter().any(|l| l.contains("[control] SetControlAction")), "{:#?}", m.log.len());
}

#[test]
fn applications_without_code_0_or_with_a_data_only_fork_are_refused_by_name() {
    // HFS_XFer_DA is a desk accessory (DRVR resources, no CODE 0): not an application
    let Some(file) = corpus("packages/go/HFS_XFer_DA/%HFS_XFer_DA") else { return };
    let e = Mac::new(RecordingGuestUi::new()).unwrap().launch(&file).unwrap_err();
    assert!(e.contains("'CODE' 0") && e.contains("not a 68k application"), "{e}");
}
