//! Event Manager semantics and Window Manager hit testing as a program sees them (evtest.s).
use heirloom_ui_abi::*;
use heirloom_mac68k_engine::fixture::EVTEST;
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::toolbox::{WindowSpec, IN_CONTENT, IN_DESK, IN_DRAG, IN_GO_AWAY, IN_MENU_BAR};
use heirloom_mac68k_engine::{Mac, Step};

fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }
fn w(m: &Mac<RecordingGuestUi>, a: u32) -> u16 { m.mem.read_u16(a).unwrap() }
fn point(v: i16, h: i16) -> u32 { (u32::from(v as u16) << 16) | u32::from(h as u16) }

fn spec(go_away: bool, visible: bool) -> WindowSpec {
    WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 300, right: 250 }, title: "Doc".into(), visible, proc_id: 0, behind: 0xFFFF_FFFF, go_away, ref_con: 0 }
}

fn prepared() -> (Mac<RecordingGuestUi>, u32) {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let win = m.make_window(spec(true, true)).unwrap();
    // the window's own activate + update events are queued first; drop them so the program sees only ours
    m.ev.queue.clear();
    m.post(3, 0x61, 0, 0); // keyDown 'a'
    m.post(1, 7, 30, 40); // mouseDown message 7 at (30,40)
    m.post(6, 0, 0, 0); // updateEvt
    m.advance_ticks(500);
    m.start(EVTEST, 0).unwrap();
    let (v, h) = heirloom_mac68k_engine::events::go_away_point(&Rect { top: 100, left: 50, bottom: 300, right: 250 });
    for (a, v) in [(0x3100, win), (0x3104, point(150, 100)), (0x310C, point(v, h)), (0x3110, point(5, 10)), (0x3114, point(400, 400))] { m.mem.write_u32(a, v).unwrap(); }
    (m, win)
}

#[test]
fn event_calls_filter_by_mask_flush_and_block_only_when_asked_to() {
    let (mut m, _) = prepared();
    let r = m.run_loop(100_000).unwrap();
    assert!(r.stats.blocked && !r.stats.halted, "GetNextEvent with nothing queued blocks: {:#?}", m.log);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!((w(&m, 0x3000), w(&m, 0x3002)), (0x0100, 1), "EventAvail(mouseDown): TRUE, what = mouseDown");
    assert_eq!((w(&m, 0x3004), l(&m, 0x3008), l(&m, 0x300C)), (0x0100, 7, point(30, 40)), "GetNextEvent removed it and filled the record");
    assert_eq!(w(&m, 0x3010), 0, "EventAvail(mouseDown) again: the other events do not match the mask, and the match is gone");
    assert_eq!(w(&m, 0x3012), 0, "FlushEvents(everyEvent) emptied the queue");
    assert_eq!((w(&m, 0x3014), w(&m, 0x3016)), (0, 0), "WaitNextEvent(sleep 0): null event, no block");
    assert_eq!(l(&m, 0x3018), 500, "TickCount is the host clock");
    // blocked: stepping again without an event runs nothing
    let (step, s) = m.step(10_000);
    assert_eq!((step, s.insns), (Step::Blocked, 0));
    // the host posts an event: the blocked call completes with TRUE and the record, then the program exits
    m.post(1, 99, 1, 2);
    let r = m.run_loop(100_000).unwrap();
    assert!(r.stats.halted, "{:#?}", m.log);
    assert_eq!((w(&m, 0x302A), w(&m, 0x302C)), (0x0100, 1));
}

#[test]
fn a_blocked_call_wakes_only_for_an_event_in_its_mask() {
    // clr.w -(sp); move.w #2,-(sp) (mouseDownMask); pea $3200; _GetNextEvent; move.w (sp)+,$3000; _ExitToShell
    let prog = [0x42, 0x67, 0x3F, 0x3C, 0x00, 0x02, 0x48, 0x79, 0x00, 0x00, 0x32, 0x00, 0xA9, 0x70, 0x33, 0xDF, 0x00, 0x00, 0x30, 0x00, 0xA9, 0xF4];
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.start(&prog, 0).unwrap();
    assert_eq!(m.step(1000).0, Step::Blocked);
    m.post(3, 0x61, 0, 0); // keyDown: not in the mask
    let (step, s) = m.step(1000);
    assert_eq!((step, s.insns), (Step::Blocked, 0), "a non-matching event leaves the guest asleep");
    assert_eq!(m.ev.queue.len(), 1, "and is not eaten");
    m.post(1, 5, 0, 0); // mouseDown
    assert_eq!(m.step(1000).0, Step::Exited);
    assert_eq!(m.mem.read_u16(0x3000).unwrap(), 0x0100);
    assert_eq!(m.mem.read_u16(0x3200).unwrap(), 1, "the record holds the mouseDown");
    assert_eq!(m.ev.queue.len(), 1, "the keyDown is still queued");
}

#[test]
fn find_window_and_track_go_away_answer_geometry_not_a_canned_value() {
    let (mut m, win) = prepared();
    m.run_loop(100_000).unwrap();
    assert_eq!(w(&m, 0x3020), IN_CONTENT);
    assert_eq!(l(&m, 0x3108), win, "FindWindow's VAR parameter names the window");
    assert_eq!(w(&m, 0x3022), 0, "TrackGoAway: a content click is not in the go-away box");
    assert_eq!(w(&m, 0x3024), 0x0100, "TrackGoAway: the go-away point is");
    assert_eq!(w(&m, 0x3026), IN_MENU_BAR);
    assert_eq!((w(&m, 0x3028), l(&m, 0x311C)), (IN_DESK, 0), "a desk click names no window");
}

#[test]
fn hit_test_regions_and_z_order() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let back = m.make_window(spec(true, true)).unwrap();
    let mut s2 = spec(false, true);
    s2.rect = Rect { top: 150, left: 100, bottom: 350, right: 300 };
    let front = m.make_window(s2).unwrap();
    assert_eq!(m.z, vec![front, back], "behind = -1 puts the new window in front");
    assert_eq!(m.mem.read_u32(0x9D6).unwrap(), front, "WindowList");
    assert_eq!(m.mem.read_u32(front + 144).unwrap(), back, "nextWindow chain");
    assert_eq!(m.mem.read_u32(back + 144).unwrap(), 0);
    // overlap: (160,110) is inside both contents -> the front one wins
    assert_eq!(m.find_window(160, 110), (IN_CONTENT, front));
    assert_eq!(m.find_window(120, 60), (IN_CONTENT, back));
    // a title-bar click: drag; inside the go-away box only for a window that has one
    assert_eq!(m.find_window(90, 100), (IN_DRAG, back));
    assert_eq!(m.find_window(90, 60), (IN_GO_AWAY, back), "box: v in [top-15, top-4), h in [left+8, left+21)");
    assert_eq!(m.find_window(140, 110), (IN_DRAG, front), "no go-away flag: the same spot on the other window is a drag");
    assert_eq!(m.find_window(90, 21), (IN_DESK, 0));
    // an invisible window is not hit
    let mut s3 = spec(true, false);
    s3.rect = Rect { top: 200, left: 400, bottom: 300, right: 500 };
    let hidden = m.make_window(s3).unwrap();
    assert_eq!(m.find_window(250, 450), (IN_DESK, 0), "hidden window: {hidden:#x}");
    assert!(m.ui.calls.iter().filter(|c| matches!(c, UiCall::OpenWindow(_))).count() == 2, "an invisible window opens no native window");
}

#[test]
fn the_go_away_point_is_a_go_away_hit_even_high_on_the_screen() {
    // a window whose title bar lies inside the 20-row menu bar: the exact synthetic point still means inGoAway
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let mut s = spec(true, true);
    s.rect = Rect { top: 22, left: 10, bottom: 100, right: 100 };
    let win = m.make_window(s).unwrap();
    let (v, h) = heirloom_mac68k_engine::events::go_away_point(&Rect { top: 22, left: 10, bottom: 100, right: 100 });
    assert!(v < 20);
    assert_eq!(m.find_window(v, h), (IN_GO_AWAY, win));
    assert_eq!(m.find_window(v, h + 1), (IN_MENU_BAR, 0), "CONTROL: one pixel away it is the menu bar");
}

#[test]
fn a_degenerate_window_is_refused_and_leaks_nothing() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let mut s = spec(true, true);
    s.rect = Rect { top: 10, left: 10, bottom: 10, right: 50 };
    let e = m.make_window(s).unwrap_err();
    assert!(e.0.contains("empty boundsRect"), "{e:?}");
    assert_eq!((m.mm.live_ptrs(), m.windows.len(), m.z.len()), (0, 0, 0));
}
