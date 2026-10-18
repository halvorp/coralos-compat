//! The Control Manager on the v2 seam, end to end: a hand-assembled 68k program creates a push button, a check box and a
//! scroll bar with `NewControl`, waits for an event and answers it with `GlobalToLocal`/`FindControl`/`TrackControl`; the
//! native intents arrive through `RecordingGuestUi::push_intent` and the guest must see exactly what a Mac would have shown it.
mod common;
use common::{put, pstr, set_rect, Prog, Rig, WIN_LEFT, WIN_TOP};
use guestui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestRect, GuestUiEvent, Intent, UiCall, UiObjectRef};
use mac68k_engine::translate::Obj;
use mac68k_engine::Step;

const RECT: u32 = 0x3100;
const TITLE: u32 = 0x3140;
use common::ctl::{find_and_track, new_control, wait_and_localise, EV, OUT, WHICH};

struct World { rig: Rig, btn: u32, chk: u32, bar: u32 }

fn world() -> World {
    let mut rig = Rig::new();
    let win = rig.win;
    set_rect(&mut rig.m, RECT, 10, 20, 34, 100);
    set_rect(&mut rig.m, RECT + 8, 50, 20, 66, 120);
    set_rect(&mut rig.m, RECT + 16, 10, 140, 110, 156);
    put(&mut rig.m, TITLE, &pstr("OK"));
    put(&mut rig.m, TITLE + 8, &pstr("Check"));
    put(&mut rig.m, TITLE + 16, &pstr(""));
    let mut p = Prog::new();
    new_control(&mut p, win, RECT, TITLE, 0, 1, 0, OUT + 12);
    new_control(&mut p, win, RECT + 8, TITLE + 8, 0, 1, 1, OUT + 16);
    new_control(&mut p, win, RECT + 16, TITLE + 16, 0, 100, 16, OUT + 20);
    wait_and_localise(&mut p, win);
    find_and_track(&mut p, win);
    p.exit();
    rig.m.start(&p.0, 0).unwrap();
    let (step, _) = rig.m.step(1_000_000);
    assert_eq!(step, Step::Blocked, "the program stops at WaitNextEvent: {:#?}", rig.m.log);
    let r = |m: &mut Rig, a: u32| m.m.mem.read_u32(a).unwrap();
    let (btn, chk, bar) = (r(&mut rig, OUT + 12), r(&mut rig, OUT + 16), r(&mut rig, OUT + 20));
    assert!(btn != 0 && chk != 0 && bar != 0 && rig.m.refused.is_empty(), "{:?}", rig.m.refused);
    World { rig, btn, chk, bar }
}

fn native(w: &World, ctl: u32) -> UiObjectRef { w.rig.m.tr.projected(Obj::Control(ctl)).expect("projected").native }
fn win_ref(w: &World) -> UiObjectRef { w.rig.m.windows[&w.rig.win].native.unwrap() }

fn finish(w: &mut World) {
    let (step, _) = w.rig.m.step(1_000_000);
    assert_eq!(step, Step::Exited, "{:#?}", w.rig.m.log);
}
fn word(w: &World, off: u32) -> u16 { w.rig.m.mem.read_u16(OUT + off).unwrap() }

#[test]
fn controls_are_projected_from_the_control_records() {
    let w = world();
    let win = win_ref(&w);
    let adds: Vec<&ControlModel> = w.rig.m.ui.calls.iter().filter_map(|c| match c { UiCall::AddControl(_, m) => Some(m), _ => None }).collect();
    assert_eq!(adds.len(), 3);
    assert_eq!(adds[0], &ControlModel { kind: ControlKind::Button, label: "OK".into(), rect: GuestRect { left: 20, top: 10, width: 80, height: 24 }, enabled: true, token: CallbackToken(u64::from(w.btn)) });
    assert_eq!(adds[1].kind, ControlKind::Checkbox { checked: false });
    assert_eq!(adds[2].kind, ControlKind::ScrollBar { vertical: true, total: 101, visible: 1, top: 0 }, "orientation from the rect, units from min..max");
    assert_eq!(w.rig.m.ui.route_census(win).native, 3, "the route census counts them as native");
    // the record is real guest memory, newest control first in the window's list
    assert_eq!(w.rig.m.window_controls(w.rig.win).unwrap(), vec![w.bar, w.chk, w.btn]);
}

#[test]
fn a_native_click_is_a_mouse_down_and_the_part_track_control_returns() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.btn));
    w.rig.m.ui.push_intent(win, ctl, Intent::Activate).unwrap();
    finish(&mut w);
    let ev = |o: u32| w.rig.m.mem.read_u16(EV + o).unwrap() as i16;
    assert_eq!(ev(0), 1, "what = mouseDown");
    // the middle of the button is local (22,60); the program's GlobalToLocal turned the event's GLOBAL where into that
    assert_eq!((ev(10), ev(12)), (22, 60));
    let mouse = |o: u32| w.rig.m.mem.read_u16(mac68k_engine::lowmem::MOUSE + o).unwrap() as i16;
    assert_eq!((mouse(0), mouse(2)), (22 + WIN_TOP, 60 + WIN_LEFT), "the low-memory Mouse is global, the window origin added");
    // the click is complete: the button is UP again (MBState bit 7) and the mouseUp is queued after the mouseDown, at the same place
    assert_eq!(w.rig.m.mem.read_u8(mac68k_engine::lowmem::MB_STATE).unwrap(), 0x80, "the button is up again once TrackControl has returned");
    let whats: Vec<u16> = w.rig.m.ev.queue.iter().map(|e| e.what).collect();
    assert_eq!(whats, vec![2], "the mouseDown was consumed by the program, its mouseUp is still queued: {whats:?}");
    assert_eq!((w.rig.m.ev.queue[0].v, w.rig.m.ev.queue[0].h), (22 + WIN_TOP, 60 + WIN_LEFT));
    assert_eq!((word(&w, 6), word(&w, 8)), (10, 10), "FindControl and TrackControl both answer inButton");
    assert_eq!(w.rig.m.mem.read_u32(WHICH).unwrap(), w.btn);
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
}

#[test]
fn a_check_box_click_leaves_the_value_to_the_program() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.chk));
    w.rig.m.ui.push_intent(win, ctl, Intent::SetValue(ControlValue::Checked(true))).unwrap();
    finish(&mut w);
    assert_eq!((word(&w, 6), word(&w, 8)), (11, 11), "inCheckBox");
    let p = w.rig.m.mm.block(w.chk).unwrap().ptr;
    assert_eq!(w.rig.m.mem.read_u16(p + 18).unwrap(), 0, "the Control Manager never flips a check box: the program does");
    // the host toggled optimistically; the engine put the guest's truth back (so a program that ignores the click is not out of step)
    assert!(w.rig.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, r, ControlValue::Checked(false)) if *r == ctl)), "{:#?}", w.rig.m.ui.calls);
}

#[test]
fn a_scroll_bar_drag_grabs_the_thumb_where_it_was_then_track_control_leaves_the_value() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.bar));
    w.rig.m.ui.push_intent(win, ctl, Intent::ScrollTo(40)).unwrap();
    finish(&mut w);
    let p = w.rig.m.mm.block(w.bar).unwrap().ptr;
    assert_eq!(w.rig.m.mem.read_u16(p + 18).unwrap(), 40, "TrackControl left contrlValue at the dragged value");
    assert_eq!(word(&w, 8), 129, "inThumb");
    // the mouseDown was on the thumb's OLD position (the user grabs where it is): value 0 -> just under the top arrow
    let v = w.rig.m.mem.read_u16(EV + 10).unwrap() as i16; // already local (the program ran GlobalToLocal)
    assert!((26..42).contains(&v), "mouseDown at the thumb, local v = {v}");
    assert_eq!(word(&w, 6), 129, "FindControl at that point finds the thumb");
}

#[test]
fn stale_forged_and_unsupported_intents_are_refused_by_name() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.btn));
    // forged generation
    w.rig.m.ui.events.push_back(GuestUiEvent::Intent { win, control: ctl, token: CallbackToken(u64::from(w.btn)), generation: ctl.generation + 7, intent: Intent::Activate });
    // an intent the role does not have
    w.rig.m.ui.push_intent(win, ctl, Intent::SelectRow(2)).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.iter().any(|r| r.contains("does not match control ref")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.refused.iter().any(|r| r.contains("SelectRow") && r.contains("pushButProc")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1), "a refused intent produced no guest input");
    // HideControl retires the part; ShowControl is a NEW incarnation, so the old ref is stale
    let (h, s) = (w.rig.m.control_facts(w.btn).unwrap(), 0);
    assert!(h.vis && s == 0);
    let mut p = Prog::new();
    p.l(w.btn).trap(0xA958).l(w.btn).trap(0xA957).exit();
    w.rig.m.start(&p.0, 0).unwrap();
    w.rig.m.run_loop(100_000).unwrap();
    let fresh = native(&w, w.btn);
    assert_ne!(fresh, ctl);
    assert_eq!(fresh.id != ctl.id, true);
    w.rig.m.refused.clear();
    w.rig.m.ui.events.push_back(GuestUiEvent::intent(win, ctl, CallbackToken(u64::from(w.btn)), Intent::Activate));
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.iter().any(|r| r.contains("stale control ref")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1));
    // positive control: the fresh ref works
    w.rig.m.ui.events.push_back(GuestUiEvent::intent(win, fresh, CallbackToken(u64::from(w.btn)), Intent::Activate));
    w.rig.m.pump_ui();
    assert!(w.rig.m.ev.queue.iter().any(|e| e.what == 1), "{:?}", w.rig.m.refused);
}

#[test]
fn an_inactive_control_takes_no_clicks_and_set_value_is_clamped() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.chk));
    let mut p = Prog::new();
    p.l(w.chk).w(255).trap(0xA95D); // HiliteControl(chk, 255)
    p.l(w.chk).w(9).trap(0xA963); // SetControlValue(chk, 9): max is 1
    p.exit();
    w.rig.m.start(&p.0, 0).unwrap();
    w.rig.m.run_loop(100_000).unwrap();
    assert_eq!(w.rig.m.control_facts(w.chk).unwrap().value, 1, "clamped to contrlMax");
    assert!(w.rig.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetControlEnabled(_, r, false) if *r == ctl)));
    w.rig.m.ui.push_intent(win, ctl, Intent::SetValue(ControlValue::Checked(true))).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.iter().any(|r| r.contains("inactive")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1));
}

// ---- review findings: guest data cannot panic the engine, controls with no kit part are not lost, ControlRecord fields ----

#[test]
fn guest_supplied_rects_cannot_overflow_the_engine() {
    use mac68k_engine::pascal::Rect;
    use mac68k_engine::translate::controls::{part_at, row_of, Facts};
    for proc_id in [0u16, 1, 16] {
        let mut rig = Rig::new();
        let (win, nwin) = (rig.win, rig.m.windows[&rig.win].native.unwrap());
        let wild = Rect { top: 20_000, left: 30_000, bottom: 32_000, right: 32_767 };
        let ctl = rig.m.create_control(win, wild, "x", true, 0, 0, 10, proc_id, 0).unwrap();
        let f = rig.m.control_facts(ctl).unwrap();
        let row = row_of(proc_id).unwrap();
        for (v, h) in [(20_001, 30_001), (i16::MIN, i16::MIN), (i16::MAX, i16::MAX), (31_999, 32_766)] { let _ = part_at(row, &f, v, h); }
        // the intent path computes the click point from the same rect
        let n = rig.m.tr.projected(Obj::Control(ctl)).unwrap().native;
        let intent = if proc_id == 16 { Intent::ScrollTo(3) } else if proc_id == 1 { Intent::SetValue(ControlValue::Checked(true)) } else { Intent::Activate };
        rig.m.ui.push_intent(nwin, n, intent).unwrap();
        rig.m.pump_ui();
        assert!(rig.m.refused.is_empty(), "{:?}", rig.m.refused);
        // negative control: a sane rect still finds its part (the arithmetic was not just made to return 0)
        let sane = Facts { rect: Rect { top: 0, left: 0, bottom: 20, right: 80 }, ..f.clone() };
        assert_ne!(part_at(row, &sane, 10, 10), 0);
    }
}

#[test]
fn contrl_hilite_255_alone_is_inactive_and_contrl_vis_is_the_roms_255() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.btn));
    let p = w.rig.m.mm.block(w.btn).unwrap().ptr;
    assert_eq!(w.rig.m.mem.read_u8(p + 16).unwrap(), 255, "contrlVis of a visible control is 255 (the ROM's NewControl), not 1");
    // 254 is a part being tracked, NOT inactive: the control still takes a click and is enabled on the host
    let mut prog = Prog::new();
    prog.l(w.btn).w(254).trap(0xA95D).exit();
    w.rig.m.start(&prog.0, 0).unwrap();
    w.rig.m.run_loop(100_000).unwrap();
    assert!(!w.rig.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetControlEnabled(_, r, false) if *r == ctl)), "254 does not ghost the control");
    w.rig.m.ui.push_intent(win, ctl, Intent::Activate).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.is_empty() && w.rig.m.ev.queue.iter().any(|e| e.what == 1), "{:?}", w.rig.m.refused);
    // 255 IS inactive (the existing test pins the refusal); ShowControl/HideControl keep 255/0
    let mut prog = Prog::new();
    prog.l(w.btn).trap(0xA958).exit();
    w.rig.m.start(&prog.0, 0).unwrap();
    w.rig.m.run_loop(100_000).unwrap();
    assert_eq!(w.rig.m.mem.read_u8(p + 16).unwrap(), 0);
}

#[test]
fn a_native_click_ends_with_the_button_up_so_a_stale_gesture_cannot_replay() {
    let mut w = world();
    let (win, ctl) = (win_ref(&w), native(&w, w.btn));
    w.rig.m.ui.push_intent(win, ctl, Intent::Activate).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.tr.tracks.contains_key(&w.btn), "the gesture waits for the program's TrackControl");
    // a later pointer click (different input) supersedes it: no stale part can replay into a later TrackControl
    let nwin = win_ref(&w);
    w.rig.m.ui.events.push_back(GuestUiEvent::Pointer { win: nwin, x: 5, y: 5, button: guestui_abi::PointerButton::Primary, phase: guestui_abi::PointerPhase::Down, clicks: 1 });
    w.rig.m.pump_ui();
    assert!(w.rig.m.tr.tracks.is_empty(), "a newer input ended the old gesture");
    // negative control: pointer motion alone does not
    w.rig.m.ui.push_intent(win, ctl, Intent::Activate).unwrap();
    w.rig.m.pump_ui();
    w.rig.m.ui.events.push_back(GuestUiEvent::Pointer { win: nwin, x: 6, y: 6, button: guestui_abi::PointerButton::Primary, phase: guestui_abi::PointerPhase::Move, clicks: 0 });
    w.rig.m.pump_ui();
    assert!(w.rig.m.tr.tracks.contains_key(&w.btn));
}

#[test]
fn controls_with_no_kit_part_are_drawn_into_the_content_plane_and_counted() {
    use mac68k_engine::pascal::Rect;
    // a pop-up whose MENU resource is missing, one that builds its menu at run time (popupUseAddResMenu), and a procID with no
    // row at all (not a CDEF)
    for (proc_id, name) in [(1008u16, "Popup"), (1012, "AddRes"), (0x7ffe, "Unknown")] {
        let mut rig = Rig::new();
        let (win, nwin) = (rig.win, rig.m.windows[&rig.win].native.unwrap());
        let rect = Rect { top: 20, left: 20, bottom: 40, right: 120 };
        let before = rig.count_in(20, 20, 120, 40);
        let ctl = rig.m.create_control(win, rect, name, true, 1, 0, 1, proc_id, 0).unwrap();
        let mut p = Prog::new();
        p.l(win).trap(0xA969).exit(); // DrawControls
        rig.run(&p);
        rig.no_refusals();
        assert!(rig.m.tr.projected(Obj::Control(ctl)).is_none(), "{name}: no kit part");
        assert_eq!(before, 0);
        assert!(rig.count_in(20, 20, 120, 40) > 20, "{name}: the default look is in the window's bitmap:\n{}", rig.pic(18, 18, 124, 42));
        let c = rig.m.ui.route_census(nwin);
        assert_eq!((c.native, c.replayed, c.raw), (0, 1, 0), "{name}: the census counts the plane, not nothing: {c:?}");
        let ops = rig.m.ui.planes[&(nwin.id, 0)].ops.clone();
        assert!(ops.iter().any(|o| matches!(o, guestui_abi::DrawOp::Text { string, .. } if string == name)), "{name}: the title is a text run: {ops:?}");
        // HideControl erases it again
        let mut p = Prog::new();
        p.l(ctl).trap(0xA958).exit();
        rig.run(&p);
        assert_eq!(rig.count_in(20, 20, 120, 40), 0, "{name}: hidden = erased");
    }
    // NEGATIVE control: a push button HAS a kit part, so nothing is drawn into the bitmap for it
    let mut rig = Rig::new();
    let win = rig.win;
    rig.m.create_control(win, Rect { top: 20, left: 20, bottom: 40, right: 120 }, "OK", true, 0, 0, 1, 0, 0).unwrap();
    let mut p = Prog::new();
    p.l(win).trap(0xA969).exit();
    rig.run(&p);
    assert_eq!(rig.count_in(20, 20, 120, 40), 0);
    assert_eq!(rig.m.ui.route_census(rig.m.windows[&win].native.unwrap()).native, 1);
}
