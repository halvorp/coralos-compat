//! The Control Manager on the v2 seam, end to end: a hand-assembled 68k program creates a push button, a check box and a
//! scroll bar with `NewControl`, waits for an event and answers it with `GlobalToLocal`/`FindControl`/`TrackControl`; the
//! native intents arrive through `RecordingGuestUi::push_intent` and the guest must see exactly what a Mac would have shown it.
mod common;
use common::{put, pstr, set_rect, Prog, Rig, SCRATCH, WIN_LEFT, WIN_TOP};
use guestui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestRect, GuestUiEvent, Intent, UiCall, UiObjectRef};
use mac68k_engine::translate::Obj;
use mac68k_engine::Step;

const RECT: u32 = 0x3100;
const TITLE: u32 = 0x3140;
const EV: u32 = 0x3200;
const WHICH: u32 = 0x3300;
const OUT: u32 = SCRATCH; // results: +0 ctl, +4 event what, +6 FindControl part, +8 TrackControl part, +10 value

/// NewControl(win, &RECT, &TITLE, visible, value, min, max, proc, 0) -> control handle, saved at `save`.
fn new_control(p: &mut Prog, win: u32, rect: u32, title: u32, value: i16, max: i16, proc_id: i16, save: u32) {
    p.clr_l().l(win).l(rect).l(title).b(1).w(value).w(0).w(max).w(proc_id).l(0).trap(0xA954).pop_l(save);
}

/// WaitNextEvent(everyEvent, &EV, 60, nil), then GlobalToLocal(EV.where).
fn wait_and_localise(p: &mut Prog, win: u32) {
    p.set_port(win).clr_w().w(-1).l(EV).l(60).l(0).trap(0xA860).pop_w(OUT + 4);
    p.l(EV + 10).trap(0xA871);
}

/// FindControl(EV.where, win, &WHICH) then TrackControl(which, EV.where, nil).
fn find_and_track(p: &mut Prog, win: u32) {
    p.clr_w().rgn_arg(EV + 10).l(win).l(WHICH).trap(0xA96C).pop_w(OUT + 6);
    p.clr_w().rgn_arg(WHICH).rgn_arg(EV + 10).l(0).trap(0xA968).pop_w(OUT + 8);
}

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
    assert_eq!(w.rig.m.mem.read_u8(mac68k_engine::lowmem::MB_STATE).unwrap(), 0, "the button is down");
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
