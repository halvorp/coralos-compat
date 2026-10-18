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

/// Drive amimon-mui to its steady state: window open, blocked in Wait with the timer ticking.
fn amimon_mui_running() -> amiga68k_engine::Amiga<RecordingGuestUi> {
    use amiga68k_engine::Step;
    let file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/amimon-mui.020")).unwrap();
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.stop_on_unimplemented = true;
    a.start_hunk(&file).unwrap();
    for ticks in 0..4 {
        match a.step(1_000_000).0 {
            Step::Blocked => { let t = a.next_timer_deadline().expect("blocked with no timer"); a.advance_clock_to(t).unwrap(); }
            other => panic!("tick {ticks}: {other:?}: {:?}", a.refused),
        }
    }
    assert_eq!(a.step(1_000_000).0, Step::Blocked, "settled in Wait after the last tick");
    a
}

#[test]
fn amimon_mui_opens_its_window_as_guestui_calls_updates_its_rows_and_quits_on_a_native_close() {
    use amiga68k_engine::{Delivered, Step};
    use guestui_abi::{CallbackToken, ControlKind, ControlValue, GuestUiEvent, UiCall};
    let mut a = amimon_mui_running();
    // NEGATIVE CONTROL: nothing delivered -> the guest runs no instruction.
    let (s, st) = a.step(1_000_000);
    assert_eq!((s, st.insns), (Step::Blocked, 0));
    // NEGATIVE CONTROL: a close request for some OTHER native window changes nothing and wakes nobody.
    let other = guestui_abi::UiObjectRef { id: 999, generation: 1 };
    let d = a.deliver(GuestUiEvent::CloseRequested { win: other, token: CallbackToken(0) }).unwrap();
    assert_eq!(d, Delivered::NotForAGuestWindow);
    assert_eq!(a.step(1_000_000).1.insns, 0, "still blocked");
    // The real native close.
    let native = a.ui.calls.iter().find_map(|c| match c { UiCall::AddControl(w, _) => Some(*w), _ => None }).expect("it opened a window");
    assert_eq!(a.deliver(GuestUiEvent::CloseRequested { win: native, token: CallbackToken(0) }).unwrap(), Delivered::Handled);
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 100, "runaway");
        match a.step(1_000_000).0 { Step::Exited => break, Step::Blocked => { let t = a.next_timer_deadline().unwrap(); a.advance_clock_to(t).unwrap(); } o => panic!("{o:?}: {:?}", a.refused) }
    }
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let w = a.ui.calls.iter().find_map(|c| match c { UiCall::OpenWindow(m) => Some(m.clone()), _ => None }).unwrap();
    assert_eq!(w.title, "amimon - amiagent status");
    let labels: Vec<_> = a.ui.calls.iter().filter_map(|c| match c { UiCall::AddControl(_, m) => Some(m.label.clone()), _ => None }).collect();
    assert_eq!(labels, ["Agent", "State", "Request", "Command", "Requests", "Client", "Driver", "Up"], "same rows, same order as the GadTools amimon");
    assert!(a.ui.calls.iter().all(|c| !matches!(c, UiCall::AddControl(_, m) if !matches!(m.kind, ControlKind::Label { framed: true, .. }))));
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, _, ControlValue::Text(t)) if t.contains("amiagent"))), "set(MUIA_Text_Contents) reached the native label");
    let menu = a.ui.calls.iter().find_map(|c| match c { UiCall::SetMenuStrip(_, m) => Some(m.clone()), _ => None }).expect("menu strip");
    assert_eq!(menu.menus[0].items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["About...", "MUI...", "Quit"]);
    assert!(matches!(a.ui.calls.last(), Some(UiCall::CloseWindow(_))), "its last UI call closed the window");
    assert_eq!(a.heap.live_blocks(), 0, "MUI_DisposeObject freed every object");
    assert!(a.objects.map.is_empty());
    assert!(a.libs.iter().all(|l| l.open_count == 0), "{:?}", a.libs.iter().map(|l| (l.name, l.open_count)).collect::<Vec<_>>());
}

#[test]
fn amimon_mui_quits_on_its_native_quit_menu_item_through_menuaction_and_the_userdata() {
    use amiga68k_engine::{Delivered, Step};
    use guestui_abi::{GuestUiEvent, UiCall};
    let mut a = amimon_mui_running();
    let (native, quit) = (a.ui.calls.iter().find_map(|c| match c { UiCall::AddControl(w, _) => Some(*w), _ => None }).unwrap(), a.ui.calls.iter().find_map(|c| match c {
        UiCall::SetMenuStrip(_, m) => m.menus[0].items.iter().find(|i| i.label == "Quit").map(|i| i.token), _ => None }).unwrap());
    // NEGATIVE CONTROL: a token that is no menu item is not ours.
    assert_eq!(a.deliver(GuestUiEvent::MenuCommand { win: native, token: guestui_abi::CallbackToken(0x1234) }).unwrap_err().0, "deliver: MenuCommand { win: UiObjectRef { id: 1, generation: 1 }, token: CallbackToken(4660) } not bridged yet");
    assert_eq!(a.deliver(GuestUiEvent::MenuCommand { win: native, token: quit }).unwrap(), Delivered::Handled);
    loop { match a.step(1_000_000).0 { Step::Exited => break, Step::Blocked => { let t = a.next_timer_deadline().unwrap(); a.advance_clock_to(t).unwrap(); } o => panic!("{o:?}: {:?}", a.refused) } }
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(a.objects.map.is_empty());
}
