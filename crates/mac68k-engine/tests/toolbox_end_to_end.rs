//! The Mac engine end to end: a hand-assembled program runs through retro-m68k and reaches GuestUi.
use guestui_abi::{CallbackToken, GuestRect, GuestUiEvent, RecordingGuestUi, UiCall, UiObjectRef, WindowModel, WindowRole};
use mac68k_engine::fixture::{HELLO, UNKNOWN_TRAP};

use mac68k_engine::{Delivered, Mac};

fn mac() -> Mac<RecordingGuestUi> { Mac::new(RecordingGuestUi::new()).unwrap() }

#[test]
fn hello_mac_opens_then_closes_a_window_on_guestui() {
    let mut m = mac();
    let r = m.run_program(HELLO, 0, 10_000).unwrap();
    assert!(r.stats.halted, "ExitToShell ends the run");
    assert_eq!(r.stats.bus_faults, 0);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let win = UiObjectRef { id: 1, generation: 1 };
    // the WindowPtr: the WindowRecord is a Ptr from the Memory Manager (after InitMenus' MenuList handle and its master block)
    let wptr: u64 = m.log.iter().find(|l| l.contains("NewWindow")).and_then(|l| l.rsplit("-> 0x").next()).map(|h| u64::from_str_radix(h, 16).unwrap()).unwrap();
    assert_eq!(m.ui.calls, vec![
        UiCall::OpenWindow(WindowModel {
            title: "Hello Mac".into(),
            // Rect is top,left,bottom,right = 40,30,240,350
            rect: GuestRect { left: 30, top: 40, width: 320, height: 200 },
            role: WindowRole::Document, closable: true, resizable: true,
            token: CallbackToken(wptr),
        }),
        UiCall::CloseWindow(win),
    ]);
    // The Pascal frames balanced: NewWindow's result reached the program's A3 and DisposeWindow found that very window.
    assert_eq!(m.mm.live_ptrs(), 0, "DisposeWindow gave the WindowRecord back");
    assert!(m.windows.is_empty());
    assert_eq!(m.log.len(), 10, "{:#?}", m.log);
    assert!(m.log[0].contains("InitGraf") && m.log[8].starts_with("[toolbox] DisposeWindow"), "{:#?}", m.log);
    assert!(m.log[7].starts_with("[toolbox] NewWindow \"Hello Mac\" top=40 left=30 w=320 h=200"), "{:#?}", m.log);
}

#[test]
fn bad_guest_pointer_is_a_named_refusal_not_a_crash() {
    // Dispose a window that was never opened.
    let mut m = mac();
    let prog = [0x2F, 0x3C, 0x00, 0x12, 0x34, 0x56, 0xA9, 0x14, 0xA9, 0xF4]; // move.l #$123456,-(sp); _DisposeWindow; _ExitToShell
    let r = m.run_program(&prog, 0, 1000).unwrap();
    assert!(r.stats.halted);
    assert_eq!(m.refused, vec!["DisposeWindow: DisposeWindow: 0x123456 is not an open window".to_string()]);
    assert!(m.ui.calls.is_empty());
}

#[test]
fn unknown_trap_is_a_named_refusal_and_the_run_continues() {
    let mut m = mac();
    let r = m.run_program(UNKNOWN_TRAP, 0, 1000).unwrap();
    assert!(r.stats.halted);
    assert_eq!(m.refused, vec!["unimplemented trap _SysBeep ($A9C8)".to_string()]);
    let inits = m.log.iter().filter(|l| l.contains("InitCursor")).count();
    assert_eq!(inits, 2, "non-census: execution went on past the refusal: {:#?}", m.log);
}

#[test]
fn census_mode_halts_at_the_first_unknown_trap() {
    // Control for the test above: same program, census on -> the second InitCursor never runs.
    let mut m = mac();
    m.stop_on_unimplemented = true;
    let r = m.run_program(UNKNOWN_TRAP, 0, 1000).unwrap();
    assert!(r.stats.halted);
    assert_eq!(m.refused, vec!["unimplemented trap _SysBeep ($A9C8)".to_string()]);
    let inits = m.log.iter().filter(|l| l.contains("InitCursor")).count();
    assert_eq!(inits, 1, "{:#?}", m.log);
}

#[test]
fn an_unnamed_trap_says_so() {
    let mut m = mac();
    m.stop_on_unimplemented = true;
    m.run_program(&[0xA8, 0x1E, 0xA9, 0xF4], 0, 100).unwrap();
    assert_eq!(m.refused, vec!["unimplemented trap $A81E (unnamed)".to_string()]);
}

#[test]
fn deliver_close_request_queues_for_a_guest_window_only() {
    let mut m = mac();
    // Open a window, then spin: the DisposeWindow sequence (move.l a3,-(sp); _DisposeWindow at 0x36) becomes `bra.s .; nop`.
    let mut code = HELLO.to_vec();
    code[0x36..0x3A].copy_from_slice(&[0x60, 0xFE, 0x4E, 0x71]);
    m.start(&code, 0).unwrap();
    let (step, _) = m.step(300);
    assert_eq!(step, mac68k_engine::Step::BudgetSpent);
    assert_eq!(m.windows.len(), 1, "{:#?}", m.log);
    let win = *m.windows.keys().next().unwrap();
    let native = UiObjectRef { id: 1, generation: 1 };
    assert_eq!(m.deliver(GuestUiEvent::CloseRequested { win: native, token: CallbackToken(0) }), Ok(Delivered::Queued));
    assert_eq!(m.ev.queue.back().map(|e| (e.what, e.message)), Some((1, win)), "mouseDown carrying the WindowPtr's go-away point");
    // Negative controls: someone else's window, and an event kind not bridged yet.
    let other = UiObjectRef { id: 99, generation: 1 };
    assert_eq!(m.deliver(GuestUiEvent::CloseRequested { win: other, token: CallbackToken(0) }), Ok(Delivered::NotForAGuestWindow));
    assert!(m.deliver(GuestUiEvent::MenuCommand { win: native, token: CallbackToken(0) }).is_err());
}
