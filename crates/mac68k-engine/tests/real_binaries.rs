//! Real, unmodified classic Mac applications. The binaries are NOT in this repository: they live in the Executor source
//! tree (MIT-licensed; `packages/go` is ARDI's own "go" launcher, AppleDouble header file `%go.appl` holding its
//! resource fork). Set `MAC68K_CORPUS` to that tree's root (default `/data/refs/retro/executor`); a missing tree SKIPS
//! the test loudly instead of failing, so a checkout without the references still builds.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::Mac;

fn corpus(rel: &str) -> Option<Vec<u8>> {
    let root = std::env::var("MAC68K_CORPUS").unwrap_or_else(|_| "/data/refs/retro/executor".into());
    match std::fs::read(format!("{root}/{rel}")) {
        Ok(b) => Some(b),
        Err(e) => { eprintln!("SKIPPED: real binary {root}/{rel} not available ({e})"); None }
    }
}

#[test]
fn go_runs_its_think_c_startup_and_the_init_sequence_before_the_first_missing_trap() {
    let Some(file) = corpus("packages/go/%go.appl") else { return };
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.launch(&file).unwrap();
    let r = m.run_loop(2_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?}", r.stats);
    // The THINK C startup: ZERO / DATA / DREL resources fill the A5 world (it copies from CurStackBase up to A5, so the
    // world must start exactly at CurStackBase), then segment 6 is loaded through the jump table.
    let log = m.log.join("\n");
    for needle in ["GetResource 'ZERO' 0", "GetResource 'DATA' 0", "GetResource 'DREL' 0", "LoadSeg 6", "InitGraf", "InitWindows", "InitMenus", "InitDialogs"] {
        assert!(log.contains(needle), "{needle}: {log}");
    }
    assert_eq!(m.mem.read_u32(0x908).unwrap(), m.seg.a5 - 12524, "CurStackBase is the bottom of go's 12524-byte below-A5 world");
    // The census names exactly one missing trap and nothing else went wrong before it.
    assert_eq!(m.refused, vec!["unimplemented trap _GetCCursor ($AA1B)".to_string()]);
}

#[test]
fn applications_without_code_0_or_with_a_data_only_fork_are_refused_by_name() {
    // HFS_XFer_DA is a desk accessory (DRVR resources, no CODE 0): not an application
    let Some(file) = corpus("packages/go/HFS_XFer_DA/%HFS_XFer_DA") else { return };
    let e = Mac::new(RecordingGuestUi::new()).unwrap().launch(&file).unwrap_err();
    assert!(e.contains("'CODE' 0") && e.contains("not a 68k application"), "{e}");
}
