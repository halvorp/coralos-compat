//! A real (hand-assembled) classic Mac application, as a resource fork: CODE 0/1/2, WIND, MENU, MBAR. It runs from a
//! MacBinary, an AppleDouble header file or a raw fork through the Segment Loader (incl. a `_LoadSeg` via the jump
//! table), opens its window and menu bar on GuestUi, blocks in WaitNextEvent and quits on a native close or File > Quit.
use heirloom_ui_abi::*;
use heirloom_mac68k_engine::fixture::{test_app_fork, test_app_macbinary};
use heirloom_mac68k_engine::{Delivered, Mac, Step};
use heirloom_fmt_resfork::writer::{appledouble, ForkBuilder};
use heirloom_fmt_resfork::ResFork;

fn launch(file: &[u8]) -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.launch(file).unwrap();
    m
}
fn run_to_block(m: &mut Mac<RecordingGuestUi>) {
    let r = m.run_loop(100_000).unwrap();
    assert!(r.stats.blocked && !r.stats.halted, "the app should be waiting for an event: {:#?} {:?}", m.log, m.refused);
}
const WIN: UiObjectRef = UiObjectRef { id: 1, generation: 1 };

fn menu_command(menu: u64, item: u64) -> GuestUiEvent { GuestUiEvent::MenuCommand { win: WIN, token: CallbackToken(menu << 16 | item) } }

/// Rebuild the app's fork with one resource dropped or replaced.
fn rebuild(skip: Option<(&[u8; 4], i16)>, replace: Option<(&[u8; 4], i16, Vec<u8>)>) -> Vec<u8> {
    let f = ResFork::parse(test_app_fork()).unwrap();
    let mut b = ForkBuilder::new();
    for i in 0..f.len() {
        let e = f.entry(i).unwrap();
        if skip.is_some_and(|(t, id)| *t == e.ty && id == e.id) { continue; }
        let data = match &replace { Some((t, id, d)) if **t == e.ty && *id == e.id => d.clone(), _ => f.data(i).unwrap().to_vec() };
        b = b.add(&e.ty, e.id, f.name(i).map(|n| core::str::from_utf8(n).unwrap()), &data);
    }
    b.build()
}

#[test]
fn startup_builds_the_a5_world_loads_both_segments_and_opens_window_and_menus() {
    let mut m = launch(&test_app_macbinary());
    run_to_block(&mut m);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    // Segment Loader: both CODE segments loaded through their jump-table entries, now `jmp abs.l`
    assert_eq!(m.seg.loaded.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    let jt = m.seg.a5 + m.seg.jt_offset;
    for (k, seg) in [(0u32, 1u16), (1, 2)] {
        assert_eq!((m.mem.read_u16(jt + 8 * k).unwrap(), m.mem.read_u16(jt + 8 * k + 2).unwrap()), (seg, 0x4EF9), "entry {k}");
    }
    // A5 world: InitGraf stored thePort's address at (A5)
    assert_eq!(m.mem.read_u32(m.seg.a5).unwrap(), m.seg.a5 - 4);
    assert_eq!(m.mem.read_u32(0x904).unwrap(), m.seg.a5, "CurrentA5");
    // GuestUi: window from WIND 128, then the menu bar (DrawMenuBar ran before the window existed, so the strip follows it)
    let wptr = *m.windows.keys().next().unwrap();
    assert_eq!(m.mem.read_u32(0x9D6).unwrap(), wptr, "WindowList");
    assert_eq!(m.mem.read_u16(wptr + 16 + 6).unwrap(), 320, "window->portRect.right is the content width");
    let tok = |menu: u64, item: u64| CallbackToken(menu << 16 | item);
    let item = |label: &str, enabled, shortcut, n: u64, menu| MenuItem { label: label.into(), enabled, shortcut, check: None, sub_items: vec![], token: tok(menu, n) };
    assert_eq!(m.ui.calls, vec![
        UiCall::OpenWindow(WindowModel {
            title: "Test Window".into(), rect: GuestRect { left: 40, top: 60, width: 320, height: 200 },
            role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(u64::from(wptr)),
        }),
        UiCall::SetMenuStrip(WIN, MenuModel { menus: vec![
            Menu { title: "Apple".into(), enabled: true, items: vec![item("About Test App...", true, None, 1, 128), item("-", true, None, 2, 128)] },
            Menu { title: "File".into(), enabled: true, items: vec![item("New", true, Some('N'), 1, 129), item("Quit", true, Some('Q'), 2, 129)] },
            Menu { title: "Edit".into(), enabled: true, items: vec![
                item("Undo", true, Some('Z'), 1, 130), item("-", false, None, 2, 130), item("Cut", true, Some('X'), 3, 130),
                item("Copy", true, Some('C'), 4, 130), item("Paste", true, Some('V'), 5, 130)] },
        ] }),
        // the app drew nothing, so the window's first content frame is an EMPTY display list for the whole window (the kit's
        // surface shows through): a replayed plane, not a raw canvas
        UiCall::PresentPlane(WIN, heirloom_ui_abi::Plane { id: 0, rect: GuestRect { left: 0, top: 0, width: 320, height: 200 }, ops: vec![] }),
    ]);
    // the program asked for an activate and an update event for its window; it consumed both while waiting
    assert!(m.ev.queue.is_empty());
}

#[test]
fn it_runs_identically_from_macbinary_appledouble_and_a_raw_fork() {
    let fork = test_app_fork();
    let mut seen = vec![];
    for (name, file) in [("macbinary", test_app_macbinary()), ("appledouble", appledouble("Test App", &fork)), ("raw", fork.clone())] {
        let mut m = launch(&file);
        run_to_block(&mut m);
        seen.push((name, m.ui.calls.len(), m.windows.len()));
        assert!(m.refused.is_empty(), "{name}: {:?}", m.refused);
    }
    assert!(seen.iter().all(|&(_, n, w)| n == 3 && w == 1), "{seen:?}");
}

#[test]
fn file_quit_ends_the_program_and_closes_its_window() {
    let mut m = launch(&test_app_macbinary());
    run_to_block(&mut m);
    m.ui.events.push_back(menu_command(129, 2));
    let (step, _) = m.step(100_000);
    assert_eq!(step, Step::Exited, "{:#?}", m.log);
    assert_eq!(m.ui.calls.last(), Some(&UiCall::CloseWindow(WIN)));
    assert!(m.windows.is_empty() && m.mm.live_ptrs() == 2, "the WindowRecord went back (what is left: the A5 world and the Window Manager port InitWindows made)");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn another_menu_item_is_handled_without_quitting() {
    // NEGATIVE control for the Quit test: File > New (item 1) and an Edit item are handled and the program keeps waiting.
    let mut m = launch(&test_app_macbinary());
    run_to_block(&mut m);
    m.ui.events.push_back(menu_command(129, 1));
    m.ui.events.push_back(menu_command(130, 3));
    run_to_block(&mut m);
    let others = m.mem.read_u16(m.seg.a5 - 252).unwrap();
    assert_eq!(others, 2, "both choices reached MenuSelect and were counted");
    assert_eq!(m.windows.len(), 1);
    assert!(!m.ui.calls.iter().any(|c| matches!(c, UiCall::CloseWindow(_))));
}

#[test]
fn a_native_close_becomes_the_go_away_click_the_program_handles() {
    let mut m = launch(&test_app_macbinary());
    run_to_block(&mut m);
    m.ui.events.push_back(GuestUiEvent::CloseRequested { win: WIN, token: CallbackToken(0) });
    let (step, _) = m.step(100_000);
    assert_eq!(step, Step::Exited, "FindWindow said inGoAway and TrackGoAway said TRUE: {:#?}", m.log);
    assert!(m.log.iter().any(|l| l.contains("FindWindow") && l.contains("part 6")), "{:#?}", m.log);
    assert_eq!(m.ui.calls.last(), Some(&UiCall::CloseWindow(WIN)));
}

#[test]
fn events_for_other_windows_and_bad_menu_tokens_do_not_wake_the_program() {
    let mut m = launch(&test_app_macbinary());
    run_to_block(&mut m);
    let other = UiObjectRef { id: 99, generation: 1 };
    assert_eq!(m.deliver(GuestUiEvent::CloseRequested { win: other, token: CallbackToken(0) }), Ok(Delivered::NotForAGuestWindow));
    assert_eq!(m.deliver(GuestUiEvent::MenuCommand { win: other, token: CallbackToken(129 << 16 | 2) }), Ok(Delivered::NotForAGuestWindow));
    let (step, s) = m.step(10_000);
    assert_eq!((step, s.insns), (Step::Blocked, 0), "nothing queued: the blocked guest does not even run");
    // a token naming an item that does not exist is a named refusal and queues nothing
    let e = m.deliver(menu_command(129, 9)).unwrap_err();
    assert!(e.0.contains("menu 129 has no item 9"), "{e:?}");
    let e = m.deliver(menu_command(77, 1)).unwrap_err();
    assert!(e.0.contains("no menu 77"), "{e:?}");
    assert_eq!(m.step(10_000).0, Step::Blocked);
    // an intent for a control that does not exist (the app made no controls) is refused by name
    let ctl = GuestUiEvent::intent(WIN, WIN, CallbackToken(0), heirloom_ui_abi::Intent::Activate);
    assert!(m.deliver(ctl).unwrap_err().0.contains("is not a live control"));
}

#[test]
fn resizing_updates_the_window_record_and_queues_an_update_event() {
    let mut m = launch(&test_app_macbinary());
    run_to_block(&mut m);
    let wptr = *m.windows.keys().next().unwrap();
    m.deliver(GuestUiEvent::Resized { win: WIN, rect: GuestRect { left: 40, top: 60, width: 500, height: 300 } }).unwrap();
    assert_eq!(m.mem.read_u16(wptr + 16 + 6).unwrap(), 500, "portRect.right");
    assert_eq!(m.mem.read_u16(wptr + 16 + 4).unwrap(), 300, "portRect.bottom");
    assert_eq!(m.ev.queue.back().map(|e| (e.what, e.message)), Some((6, wptr)), "updateEvt for that window");
}

#[test]
fn a_malformed_or_missing_piece_is_a_named_refusal() {
    // not an application at all
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    assert!(m.launch(&[0x55; 300]).unwrap_err().contains("resource fork"), "garbage");
    // a corrupted MacBinary header
    let mut bad = test_app_macbinary();
    bad[69] ^= 1;
    assert!(Mac::new(RecordingGuestUi::new()).unwrap().launch(&bad).unwrap_err().contains("CRC"));
    // a fork with no CODE 0
    let e = Mac::new(RecordingGuestUi::new()).unwrap().launch(&rebuild(Some((b"CODE", 0)), None)).unwrap_err();
    assert!(e.contains("'CODE' 0"), "{e}");
    // CODE 0 whose jump table cannot fit above A5
    let mut c0 = Vec::new();
    for v in [40u32, 512, 16, 32] { c0.extend_from_slice(&v.to_be_bytes()); }
    c0.extend_from_slice(&[0; 16]);
    let e = Mac::new(RecordingGuestUi::new()).unwrap().launch(&rebuild(None, Some((b"CODE", 0, c0)))).unwrap_err();
    assert!(e.contains("does not fit above A5"), "{e}");
}

#[test]
fn a_missing_or_far_model_segment_halts_the_program_with_a_named_refusal() {
    // CODE 2 is only needed when main calls SetupMenus through the jump table
    let mut m = launch(&rebuild(Some((b"CODE", 2)), None));
    let r = m.run_loop(100_000).unwrap();
    assert!(r.stats.halted && !r.stats.blocked);
    assert!(m.refused.iter().any(|r| r.starts_with("LoadSeg:") && r.contains("no 'CODE' 2 resource") && r.contains("system error 15")), "{:?}", m.refused);
    assert!(m.ui.calls.is_empty(), "the program never got as far as a window");
    // CONTROL: the intact app gets that far
    let mut ok = launch(&test_app_fork());
    run_to_block(&mut ok);
    assert!(!ok.ui.calls.is_empty());
    // far-model header ($FFFF)
    let mut far = vec![0xFF, 0xFF, 0, 1];
    far.extend_from_slice(&[0; 8]);
    let mut m = launch(&rebuild(None, Some((b"CODE", 2, far))));
    assert!(m.run_loop(100_000).unwrap().stats.halted);
    assert!(m.refused.iter().any(|r| r.contains("far-model")), "{:?}", m.refused);
}

#[test]
fn launch_needs_a_fresh_machine_so_the_a5_world_sits_at_curstackbase() {
    let mut m = launch(&test_app_macbinary());
    assert_eq!(m.mem.read_u32(0x908).unwrap(), m.seg.a5 - 512, "CurStackBase = the bottom of the below-A5 globals (THINK C startup code relies on it)");
    let e = m.launch(&test_app_macbinary()).unwrap_err();
    assert!(e.contains("not at STACK_TOP") && e.contains("fresh machine"), "{e}");
}
