//! Input handlers and event handlers that FIRE (tests/guest/c/muievent.c: a class over Area.mui that registers, in Setup, a 1000 ms timer
//! input handler, a signal input handler, and two window event handlers, and removes them in Cleanup). The timer runs on the engine's
//! VIRTUAL clock; a pointer or key event from the host reaches the nodes as the IntuiMessage window.c's `HandleInputEvent` / `HandleRawkey`
//! would hand them, in priority order, until one eats it.
use amiga68k_engine::{Amiga, Step};
use guestui_abi::{GuestUiEvent, Modifiers, PointerButton, PointerPhase, RecordingGuestUi, UiObjectRef};
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/muievent.bin");
const RES: u32 = 0x1F000;
const TICKS: u32 = 6;
const SIGS: u32 = 13;
const CALLS_A_OR_B: u32 = 7;
const CLASS: u32 = 8;
const CODE: u32 = 9;
const MX: u32 = 10;
const MY: u32 = 11;

fn run(mode: u32, eat_nothing: bool) -> Amiga<RecordingGuestUi> {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.mem.write_u32(RES + 4 * 63, mode).unwrap();
    a.mem.write_u32(RES + 4 * 62, eat_nothing as u32).unwrap();
    a.start_hunk(&build(&[Hunk::code(BIN.to_vec(), vec![])], None)).unwrap();
    let (s, _) = a.step(5_000_000);
    assert!(s == Step::Blocked || (mode == 1 && s == Step::Exited), "{s:?} {:?} {:?}", a.refused, a.log.iter().rev().take(5).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    a
}
fn r(a: &Amiga<RecordingGuestUi>, i: u32) -> u32 { a.mem.read_u32(RES + 4 * i).unwrap() }
fn settle(a: &mut Amiga<RecordingGuestUi>) { assert_eq!(a.step(5_000_000).0, Step::Blocked, "{:?}", a.refused); assert!(a.refused.is_empty(), "{:?}", a.refused); }
fn window(a: &Amiga<RecordingGuestUi>) -> UiObjectRef { match &a.objects.map[&r(a, 4)].ext { amiga68k_engine::boopsi::Ext::Win { native: Some(n), .. } => *n, _ => panic!("the window is not open") } }
fn tick_to(a: &mut Amiga<RecordingGuestUi>, us: u64) { let t = a.next_timer_deadline().expect("an armed timer"); assert_eq!(t, us); a.advance_clock_to(t).unwrap(); settle(a); }

#[test]
fn a_timer_input_handler_fires_every_ihn_millis_of_virtual_time_and_not_before() {
    let mut a = run(0, false);
    assert_eq!([r(&a, 40), r(&a, 41), r(&a, 42), r(&a, 43)], [1; 4], "AddInputHandler / AddEventHandler answer TRUE");
    assert_eq!(r(&a, TICKS), 0, "nothing fires at set-up");
    assert_eq!(a.next_timer_deadline(), Some(1_000_000), "armed for 1000 ms after the Setup that added it");
    settle(&mut a);
    assert_eq!(r(&a, TICKS), 0, "a program that merely runs does not tick: only the clock does");
    for n in 1..=3u32 {
        tick_to(&mut a, 1_000_000 * n as u64);
        assert_eq!(r(&a, TICKS), n, "one method per period, re-armed from the clock at that moment");
    }
}

#[test]
fn a_timer_handler_is_gone_when_its_class_removes_it_at_cleanup() {
    // NEGATIVE CONTROL: closing the window runs Cleanup, which sends RemInputHandler; the clock has nothing to wait for and the method never comes.
    let mut a = run(1, false);
    assert_eq!(r(&a, 31), 1, "Cleanup ran");
    assert_eq!(a.next_timer_deadline(), None);
    assert!(a.objects.handlers.ihn.is_empty() && a.objects.handlers.evh.is_empty());
    let _ = &mut a;
    assert_eq!(r(&a, TICKS), 0);
}

#[test]
fn a_signal_input_handler_runs_when_its_signals_arrive_and_for_no_others() {
    let mut a = run(0, false);
    let task = a.sched.app_task.values().next().copied().unwrap_or(a.sched.cur);
    let bit = r(&a, 44);
    amiga68k_engine::exec_wait::signal_task(&mut a, task, 1 << (bit + 1)).unwrap(); // NEGATIVE CONTROL: not the handler's bit
    settle(&mut a);
    let base = r(&a, SIGS);
    assert_eq!(base, 1, "MUI's first NewInput assumes every signal is set (application.c: a program that passes 0), so a signal handler runs once at start");
    amiga68k_engine::exec_wait::signal_task(&mut a, task, 1 << bit).unwrap();
    settle(&mut a);
    assert_eq!(r(&a, SIGS), base + 1, "ihn_Signals = the class's own signal -> DoMethod(ihn_Object, ihn_Method)");
    settle(&mut a);
    assert_eq!(r(&a, SIGS), base + 1, "and only once");
}

#[test]
fn pointer_events_reach_event_handlers_in_priority_order_and_an_eaten_event_stops_the_walk() {
    let mut a = run(0, false);
    let w = window(&a);
    a.deliver(GuestUiEvent::primary(w, 30, 40, PointerPhase::Down)).unwrap();
    settle(&mut a);
    // Handler A (priority 0) saw it and returned MUI_EventHandlerRC_Eat for a SELECTDOWN: B (priority -1) did not.
    assert_eq!((r(&a, CALLS_A_OR_B), r(&a, CLASS), r(&a, CODE), r(&a, MX), r(&a, MY)), (1, 0x8, 0x68, 30, 40));
    assert_eq!(r(&a, 25), u32::MAX, "muikey is MUIKEY_NONE");
    a.deliver(GuestUiEvent::primary(w, 31, 41, PointerPhase::Up)).unwrap();
    settle(&mut a);
    assert_eq!((r(&a, CALLS_A_OR_B), r(&a, CODE)), (3, 0xE8), "SELECTUP is not eaten: A, then B");
    // A mouse move is for B alone (A did not ask for MOUSEMOVE).
    a.deliver(GuestUiEvent::primary(w, 50, 20, PointerPhase::Move)).unwrap();
    settle(&mut a);
    assert_eq!((r(&a, CALLS_A_OR_B), r(&a, CLASS), r(&a, MX)), (4, 0x10, 50));
    // A key: RAWKEY goes to A only (B did not ask), with the Amiga raw code; a release carries the up prefix.
    a.deliver(GuestUiEvent::Key { win: w, code: 0x45, text: String::new(), modifiers: Modifiers::default(), down: true }).unwrap();
    settle(&mut a);
    assert_eq!((r(&a, CALLS_A_OR_B), r(&a, CLASS), r(&a, CODE)), (5, 0x400, 0x45));
}

#[test]
fn without_eat_the_next_handler_sees_the_event_too_and_an_unlistened_class_reaches_nobody() {
    // NEGATIVE CONTROL: the same press when no handler eats (RES[62] = 1): A and B both run.
    let mut a = run(0, true);
    let w = window(&a);
    a.deliver(GuestUiEvent::primary(w, 30, 40, PointerPhase::Down)).unwrap();
    settle(&mut a);
    assert_eq!(r(&a, CALLS_A_OR_B), 2);
    // The secondary button belongs to the menu system unless the window traps it: no handler is called.
    let before = r(&a, CALLS_A_OR_B);
    a.deliver(GuestUiEvent::Pointer { win: w, x: 5, y: 5, button: PointerButton::Secondary, phase: PointerPhase::Down, clicks: 1 }).unwrap();
    settle(&mut a);
    assert_eq!(r(&a, CALLS_A_OR_B), before);
    // A key code past 0x7F has no raw key code: refused by name, nothing queued.
    assert!(a.deliver(GuestUiEvent::Key { win: w, code: 0x100, text: String::new(), modifiers: Modifiers::default(), down: true }).is_err());
}
