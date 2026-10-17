//! Real, unmodified AmigaOS binaries run end to end on the engine (tests/corpus/README.md).
use amiga68k_engine::Amiga;
use guestui_abi::RecordingGuestUi;

#[test]
fn amiagent_runs_its_startup_reads_its_args_and_exits_cleanly_without_a_tcp_stack() {
    let file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/amiagent.020")).unwrap();
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.stop_on_unimplemented = true;
    let r = a.run_hunk(&file, 1_000_000).unwrap();
    assert!(r.stats.halted, "it returned to the exit slot");
    assert_eq!(r.stats.bus_faults, 0);
    assert!(a.refused.is_empty(), "no refused call: {:?}", a.refused);
    let out = String::from_utf8_lossy(&a.console.stdout);
    assert!(out.contains("amiagent: no bsdsocket.library - start your TCP/IP stack first."), "{out}");
    assert!(a.log.iter().any(|l| l.starts_with("[dos] ReadArgs \"PORT/N,TLSPORT/N,TOKEN/K,QUIET/S,VERBOSE/S\"")));
    assert_eq!(a.heap.live_blocks(), 0, "it freed everything it allocated");
    assert!(a.libs.iter().all(|l| l.open_count == 0), "it closed every library it opened: {:?}",
        a.libs.iter().map(|l| (l.name, l.open_count)).collect::<Vec<_>>());
}
