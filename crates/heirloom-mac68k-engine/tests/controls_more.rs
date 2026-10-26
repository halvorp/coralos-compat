//! Radio buttons and pop-up menus on the Control Manager table, end to end like translate_controls.rs: a hand-assembled program
//! creates the control, waits for an event and answers it with FindControl / TrackControl; the native intent arrives through
//! `RecordingGuestUi::push_intent` and the guest must see what a Mac would have shown it. Facts: Macintosh Toolbox Essentials,
//! Control Manager (radio = `inCheckBox`, value left to the program; shown as a one-label radio Choice; pop-up CDEF: `minValue` is the MENU ID, `contrlMin`/`contrlMax`
//! become 1 and the item count, the chosen item is `contrlValue`); the part code of a pop-up choice (`inButton`, 10) is what
//! systemless' pop-up CDEF returns, which the panel's first draft had wrong as `inMenu`.
mod common;
use common::ctl::{find_and_track, new_control_ranged, wait_and_localise, OUT, WHICH};
use common::{put, pstr, set_rect, Prog, Rig};
use heirloom_ui_abi::{ChoiceStyle, ControlKind, ControlModel, ControlValue, Intent, UiCall, UiObjectRef, CHOICE_NONE};
use heirloom_mac68k_engine::menu::{encode_block, Item, Parsed};
use heirloom_mac68k_engine::translate::controls::{part_at, row_of, Cdef, Facts};
use heirloom_mac68k_engine::translate::Obj;
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::Step;

const RECT: u32 = 0x3100;
const TITLE: u32 = 0x3140;
const MENU_ID: i16 = 200;

fn menu_fork(items: &[&str]) -> Vec<u8> {
    let p = Parsed { id: MENU_ID, flags: 0xFFFF_FFFF, title: b"Size".to_vec(), items: items.iter().map(|t| Item { text: t.as_bytes().to_vec(), icon: 0, key: 0, mark: 0, style: 0 }).collect() };
    heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"MENU", MENU_ID, None, &encode_block(&p, 0, 0, 0)).build()
}

struct World { rig: Rig, ctl: u32 }

/// One control of `proc_id` (value, min, max as given) in a window whose resource file holds MENU 200 with `items` (none = no MENU).
fn world(proc_id: i16, title: &str, value: i16, min: i16, max: i16, items: &[&str]) -> World {
    let mut rig = Rig::new();
    if !items.is_empty() { rig.m.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(menu_fork(items)).unwrap()); }
    let win = rig.win;
    set_rect(&mut rig.m, RECT, 10, 20, 34, 140);
    put(&mut rig.m, TITLE, &pstr(title));
    let mut p = Prog::new();
    new_control_ranged(&mut p, win, RECT, TITLE, value, min, max, proc_id, OUT + 12);
    wait_and_localise(&mut p, win);
    find_and_track(&mut p, win);
    p.exit();
    rig.m.start(&p.0, 0).unwrap();
    assert_eq!(rig.m.step(1_000_000).0, Step::Blocked, "{:#?}", rig.m.log);
    let ctl = rig.m.mem.read_u32(OUT + 12).unwrap();
    assert!(ctl != 0 && rig.m.refused.is_empty(), "{:?}", rig.m.refused);
    World { rig, ctl }
}

fn native(w: &World) -> UiObjectRef { w.rig.m.tr.projected(Obj::Control(w.ctl)).expect("projected").native }
fn win_ref(w: &World) -> UiObjectRef { w.rig.m.windows[&w.rig.win].native.unwrap() }
fn model(w: &World) -> ControlModel { w.rig.m.tr.projected(Obj::Control(w.ctl)).unwrap().model.clone() }
fn finish(w: &mut World) { assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log); }
fn word(w: &World, off: u32) -> u16 { w.rig.m.mem.read_u16(OUT + off).unwrap() }
fn field(w: &World, off: u32) -> i16 { let p = w.rig.m.mm.block(w.ctl).unwrap().ptr; w.rig.m.mem.read_u16(p + off).unwrap() as i16 }

#[test]
fn a_radio_button_is_a_one_label_radio_choice_and_a_click_is_in_check_box_with_the_value_left_to_the_program() {
    let mut w = world(2, "Fast", 0, 1, 1, &[]);
    assert_eq!(model(&w).kind, ControlKind::Choice { style: ChoiceStyle::Radio, labels: vec!["Fast".into()], selected: CHOICE_NONE }, "an off radio says 'none chosen'");
    assert_eq!(model(&w).label, "Fast");
    let (win, ctl) = (win_ref(&w), native(&w));
    w.rig.m.ui.push_intent(win, ctl, Intent::SetValue(ControlValue::Choice(0))).unwrap();
    finish(&mut w);
    assert_eq!((word(&w, 6), word(&w, 8)), (11, 11), "FindControl and TrackControl both answer inCheckBox");
    assert_eq!(field(&w, 18), 0, "the app keeps its radio group: the Control Manager never flips the value");
    assert!(w.rig.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, r, ControlValue::Choice(CHOICE_NONE)) if *r == ctl)), "the optimistic host state is put back");
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
}

#[test]
fn a_radio_button_refuses_what_a_radio_cannot_do_and_the_program_sets_the_value_itself() {
    let mut w = world(2, "Fast", 0, 1, 1, &[]);
    let (win, ctl) = (win_ref(&w), native(&w));
    w.rig.m.ui.push_intent(win, ctl, Intent::SelectRow(0)).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.iter().any(|r| r.contains("SelectRow") && r.contains("radioButProc")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1), "a refused intent produced no guest input");
    // NEGATIVE CONTROLS: a radio is not a check box (Checked is the wrong value), and it has exactly one label (item 1 does not exist)
    for bad in [Intent::SetValue(ControlValue::Checked(true)), Intent::SetValue(ControlValue::Choice(1))] {
        w.rig.m.refused.clear();
        w.rig.m.ui.push_intent(win, ctl, bad).unwrap();
        w.rig.m.pump_ui();
        assert_eq!(w.rig.m.refused.len(), 1, "named: {:?}", w.rig.m.refused);
        assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1), "a refused intent produced no guest input");
    }
    // positive control: SetControlValue(ctl, 1) from the program is what turns the native part on
    let mut p = Prog::new();
    p.l(w.ctl).w(1).trap(0xA963).exit();
    w.rig.m.refused.clear();
    w.rig.m.start(&p.0, 0).unwrap();
    w.rig.m.run_loop(100_000).unwrap();
    assert_eq!(model(&w).kind, ControlKind::Choice { style: ChoiceStyle::Radio, labels: vec!["Fast".into()], selected: 0 });
}

#[test]
fn a_popup_is_a_native_choice_over_its_menu_resource_and_the_cdef_publishes_min_and_max() {
    let w = world(1008, "Size:", 2, MENU_ID, 40, &["Small", "Medium", "Large"]);
    assert_eq!(model(&w).kind, ControlKind::Choice { style: ChoiceStyle::Popup, labels: vec!["Small".into(), "Medium".into(), "Large".into()], selected: 1 }, "value 2 = the second item");
    assert_eq!((field(&w, 20), field(&w, 22), field(&w, 18)), (1, 3, 2), "contrlMin 1, contrlMax = the item count, contrlValue as given");
    assert_eq!(model(&w).label, "Size:");
    assert_eq!(w.rig.m.tr.popups[&w.ctl].0, MENU_ID);
    // the variation bits (fixed width, useWFont) are still the pop-up CDEF
    for proc_id in [1009u16, 1016] { assert_eq!(row_of(proc_id).map(|r| r.cdef), Some(Cdef::PopupMenu), "{proc_id}"); }
    // NEGATIVE: neighbours of the pop-up id are not pop-ups
    for proc_id in [1007u16, 1024, 1000] { assert!(row_of(proc_id).is_none(), "{proc_id}"); }
}

#[test]
fn choosing_a_popup_item_is_a_mouse_down_and_track_control_returns_in_button_with_the_value_set() {
    let mut w = world(1008, "Size:", 1, MENU_ID, 40, &["Small", "Medium", "Large"]);
    let (win, ctl) = (win_ref(&w), native(&w));
    w.rig.m.ui.push_intent(win, ctl, Intent::SetValue(ControlValue::Choice(2))).unwrap();
    finish(&mut w);
    assert_eq!((word(&w, 6), word(&w, 8)), (10, 10), "inButton from FindControl and TrackControl (not inMenu: that is a FindWindow code)");
    assert_eq!(field(&w, 18), 3, "TrackControl left contrlValue at the chosen item (1-based) before it returned");
    assert_eq!(w.rig.m.mem.read_u32(WHICH).unwrap(), w.ctl);
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
    // the record is the truth: GetControlValue reads 3 and the native choice shows the third item
    assert_eq!(model(&w).kind, ControlKind::Choice { style: ChoiceStyle::Popup, labels: vec!["Small".into(), "Medium".into(), "Large".into()], selected: 2 });
}

#[test]
fn a_popup_refuses_an_item_it_does_not_have_and_one_without_a_readable_menu_stays_a_plane() {
    let mut w = world(1008, "Size:", 1, MENU_ID, 40, &["Small", "Medium"]);
    let (win, ctl) = (win_ref(&w), native(&w));
    w.rig.m.ui.push_intent(win, ctl, Intent::SetValue(ControlValue::Choice(2))).unwrap();
    w.rig.m.ui.push_intent(win, ctl, Intent::Activate).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.iter().any(|r| r.contains("item 2 outside the pop-up's 2 items")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.refused.iter().any(|r| r.contains("Activate") && r.contains("popupMenuProc")), "{:?}", w.rig.m.refused);
    assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1), "refused intents produced no guest input");
    assert_eq!(field(&w, 18), 1, "and changed nothing");
    // no MENU 200 in any resource file: no native part, named in the log, min/max left as the program gave them
    let w = world(1008, "Size:", 1, MENU_ID, 40, &[]);
    assert!(w.rig.m.tr.projected(Obj::Control(w.ctl)).is_none());
    assert!(w.rig.m.log.iter().any(|l| l.contains("popupMenuProc") && l.contains("no MENU resource 200")), "{:#?}", w.rig.m.log);
    assert_eq!((field(&w, 20), field(&w, 22)), (MENU_ID, 40));
}

#[test]
fn radio_and_popup_parts_come_from_the_cdef_geometry() {
    let f = |proc_id: u16| Facts { owner: 0, rect: Rect { top: 0, left: 0, bottom: 20, right: 80 }, vis: true, hilite: 0, value: 1, min: 1, max: 3, proc_id, title: "x".into(), items: vec!["a".into()] };
    assert_eq!(part_at(row_of(2).unwrap(), &f(2), 10, 10), 11);
    assert_eq!(part_at(row_of(1008).unwrap(), &f(1008), 10, 10), 10);
    assert_eq!(part_at(row_of(1008).unwrap(), &f(1008), 30, 10), 0, "outside the rect no part is hit");
}

#[test]
fn control_action_is_the_real_contrl_action_field_read_and_written_by_get_and_set_control_action() {
    let mut rig = Rig::new();
    let win = rig.win;
    set_rect(&mut rig.m, RECT, 10, 20, 34, 140);
    put(&mut rig.m, TITLE, &pstr("Go"));
    let mut p = Prog::new();
    common::ctl::new_control(&mut p, win, RECT, TITLE, 0, 1, 0, OUT);
    p.clr_l().rgn_arg(OUT).trap(0xA96A).pop_l(OUT + 4); // GetControlAction: NIL until set
    p.rgn_arg(OUT).l(0x4000).trap(0xA96B); // SetControlAction(ctl, 0x4000)
    p.clr_l().rgn_arg(OUT).trap(0xA96A).pop_l(OUT + 8);
    rig.run(p.exit());
    rig.no_refusals();
    let ctl = rig.m.mem.read_u32(OUT).unwrap();
    assert_eq!((rig.m.mem.read_u32(OUT + 4).unwrap(), rig.m.mem.read_u32(OUT + 8).unwrap()), (0, 0x4000));
    let rec = rig.m.mem.read_u32(ctl).unwrap();
    assert_eq!(rig.m.mem.read_u32(rec + 32).unwrap(), 0x4000, "contrlAction is at offset 32 of the ControlRecord");
    // NEGATIVE CONTROL: a handle that is not a control is refused by name and nothing is written
    let mut q = Prog::new();
    q.l(0x1234).l(0x5000).trap(0xA96B);
    rig.m.start(&q.exit().0, 0).unwrap();
    rig.m.run_loop(100_000).unwrap();
    assert!(rig.m.refused.iter().any(|r| r.contains("SetControlAction")), "{:?}", rig.m.refused);
}
