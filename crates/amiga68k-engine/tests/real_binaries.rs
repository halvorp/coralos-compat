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

#[test]
fn amimon_opens_its_gadtools_window_ticks_on_its_timer_and_quits_on_a_native_close() {
    use amiga68k_engine::{Delivered, Step};
    use guestui_abi::{CallbackToken, ControlKind, ControlValue, GuestUiEvent, UiCall};
    let file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/amimon.020")).unwrap();
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.stop_on_unimplemented = true;
    a.start_hunk(&file).unwrap();
    let mut ticks = 0;
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 10_000, "runaway");
        match a.step(1_000_000).0 {
            Step::Blocked if ticks < 3 => { let t = a.next_timer_deadline().expect("blocked with no timer"); a.advance_clock_to(t).unwrap(); ticks += 1; }
            Step::Blocked => {
                // NEGATIVE CONTROL first: a step with nothing delivered runs no instruction and stays blocked.
                let (s, st) = a.step(1_000_000);
                assert_eq!((s, st.insns), (Step::Blocked, 0), "no guest instruction runs while its wait is unsatisfied");
                let (&win, &native) = a.windows.iter().next().expect("its window is open");
                let d = a.deliver(GuestUiEvent::CloseRequested { win: native, token: CallbackToken(win as u64) }).unwrap();
                assert!(matches!(d, Delivered::Queued(_)), "{d:?}");
            }
            Step::Exited => break,
            other => panic!("{other:?}: {:?}", a.refused),
        }
    }
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let labels: Vec<_> = a.ui.calls.iter().filter_map(|c| match c { UiCall::AddControl(_, m) => Some(m.label.clone()), _ => None }).collect();
    assert_eq!(labels, ["Agent", "State", "Request", "Command", "Requests", "Client", "Driver", "Up"]);
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::AddControl(_, m) if matches!(m.kind, ControlKind::Label { framed: true, .. }))));
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, _, ControlValue::Text(t)) if t.contains("amiagent"))), "it reported the missing amiagent");
    assert!(matches!(a.ui.calls.last(), Some(UiCall::CloseWindow(_))), "its last UI call closed its window");
    assert!(a.windows.is_empty() && a.engine_msgs.is_empty(), "window closed, the IntuiMessage replied");
    assert_eq!(a.heap.live_blocks(), 0, "it freed everything it allocated");
    assert!(a.libs.iter().all(|l| l.open_count == 0), "{:?}", a.libs.iter().map(|l| (l.name, l.open_count)).collect::<Vec<_>>());
}
