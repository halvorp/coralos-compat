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
    assert_eq!(m.mm.live_ptrs(), 1, "DisposeWindow gave the WindowRecord back (what is left is the Window Manager port InitWindows made)");
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
    // Negative controls: someone else's window, and an event kind with no classic Mac meaning (a MenuCommand is bridged now: see app_end_to_end).
    let other = UiObjectRef { id: 99, generation: 1 };
    assert_eq!(m.deliver(GuestUiEvent::CloseRequested { win: other, token: CallbackToken(0) }), Ok(Delivered::NotForAGuestWindow));
    let ctl = GuestUiEvent::intent(native, native, CallbackToken(0), guestui_abi::Intent::Activate);
    assert!(m.deliver(ctl).is_err());
}

#[test]
fn init_graf_fills_the_quickdraw_globals_below_the_port() {
    let mut m = mac();
    m.run_program(HELLO, 0, 10_000).unwrap();
    // hello.s passes the address of its `qdglobals` label (the last 4 bytes of the image) as thePort
    let p = 0x1_0000 + HELLO.len() as u32 - 4;
    let r = |a: u32| m.mem.read_u32(a).unwrap();
    assert_eq!(r(p - 126), 1, "randSeed");
    assert_eq!(m.mem.read_u16(p - 122 + 4).unwrap(), 64, "screenBits.rowBytes");
    assert_eq!([0, 2, 4, 6].map(|o| m.mem.read_u16(p - 122 + 6 + o).unwrap()), [0, 0, 342, 512], "screenBits.bounds");
    assert_eq!(m.mem.read_u16(p - 108 + 2).unwrap(), 0x4000, "arrow data row 1");
    assert_eq!(m.mem.read_u16(p - 108 + 32 + 30).unwrap(), 0x0380, "arrow mask row 15");
    assert_eq!((m.mem.read_u16(p - 108 + 64).unwrap(), m.mem.read_u16(p - 108 + 66).unwrap()), (1, 1), "hot spot");
    assert_eq!((r(p - 40), r(p - 32), r(p - 24), r(p - 16), r(p - 8)), (0x77DD77DD, 0x88228822, 0xAA55AA55, 0xFFFFFFFF, 0), "dkGray ltGray gray black white");
    assert_eq!(r(p - 40 + 4), 0x77DD77DD, "patterns are 8 bytes: the long repeats");
    // NewWindow made the new window the current port, DisposeWindow cleared it again
    assert_eq!(r(p), 0, "thePort is NIL after the only window was disposed");
    // NEGATIVE: the program's own code and data (everything before the 256-byte reserve) were not overwritten
    assert_eq!(m.mem.bytes(m.mem.span(0x1_0000, 0x52).unwrap()).unwrap(), &HELLO[..0x52], "QuickDraw wrote only inside its 206 bytes");
}
