//! The List Manager (`_Pack0`) end to end: a hand-assembled program pushes the Pascal frames the Toolbox glue pushes (arguments left
//! to right, the selector word LAST) and the engine must pop exactly each selector's frame, keep a real `ListRec`, project a
//! standard-text-LDEF list as a native `List` and turn a native row choice into the `mouseDown` + `LClick` a Mac saw.
//! Selector numbers and frame sizes: Inside Macintosh IV, List Manager (checked against systemless' dispatcher as an oracle).
mod common;
use common::ctl::{EV, OUT};
use common::{put, set_rect, Prog, Rig, WIN_LEFT, WIN_TOP};
use heirloom_ui_abi::{CallbackToken, ControlKind, GuestRect, Intent, UiCall, UiObjectRef};
use heirloom_mac68k_engine::translate::lists::{FRAMES, L_CELL_SIZE, L_DATA_BOUNDS, L_RVIEW, L_VISIBLE};
use heirloom_mac68k_engine::translate::Obj;
use heirloom_mac68k_engine::Step;

const VIEW: u32 = 0x3100;
const BOUNDS: u32 = 0x3110;
const DATA: u32 = 0x3200; // cell data: "alpha", "beta", "gamma", "delta"
const LH: u32 = OUT + 16; // the ListHandle
const SCRATCH2: u32 = 0x3300;

fn pt(v: i16, h: i16) -> u32 { (u32::from(v as u16) << 16) | u32::from(h as u16) }
fn sel(p: &mut Prog, s: u16) { p.w(s as i16).trap(0xA9E7); }

fn l_new(p: &mut Prog, csize: (i16, i16), proc: i16, win: u32) {
    p.clr_l().l(VIEW).l(BOUNDS).l(pt(csize.0, csize.1)).w(proc).l(win).b(1).b(0).b(0).b(0);
    sel(p, 0x44);
    p.pop_l(LH);
}
fn l_add_row(p: &mut Prog, count: i16, row: i16, save: u32) { p.clr_w().w(count).w(row).rgn_arg(LH); sel(p, 0x08); p.pop_w(save); }
fn l_del_row(p: &mut Prog, count: i16, row: i16) { p.w(count).w(row).rgn_arg(LH); sel(p, 0x24); }
fn l_set_cell(p: &mut Prog, data: u32, len: i16, cell: (i16, i16)) { p.l(data).w(len).l(pt(cell.0, cell.1)).rgn_arg(LH); sel(p, 0x58); }
fn l_set_select(p: &mut Prog, on: bool, cell: (i16, i16)) { p.b(on as u8).l(pt(cell.0, cell.1)).rgn_arg(LH); sel(p, 0x5C); }
fn l_click(p: &mut Prog, save: u32) { p.clr_w().rgn_arg(EV + 10).mem_w(EV + 14).rgn_arg(LH); sel(p, 0x18); p.pop_w(save); }
fn l_get_select(p: &mut Prog, next: bool, cell_at: u32, save: u32) { p.clr_w().b(next as u8).l(cell_at).rgn_arg(LH); sel(p, 0x3C); p.pop_w(save); }

struct World { rig: Rig, win: u32 }

/// The program: LNew (empty, 1 column), LAddRow x3, four cells, then `body`, then ExitToShell.
fn world(proc: i16, body: impl FnOnce(&mut Prog, u32)) -> World {
    let mut rig = Rig::new();
    let win = rig.win;
    set_rect(&mut rig.m, VIEW, 10, 20, 70, 140);
    set_rect(&mut rig.m, BOUNDS, 0, 0, 0, 1);
    for (i, s) in ["alpha", "beta", "gamma", "delta"].iter().enumerate() { put(&mut rig.m, DATA + 8 * i as u32, s.as_bytes()); }
    let mut p = Prog::new();
    l_new(&mut p, (0, 0), proc, win);
    l_add_row(&mut p, 3, 0, OUT + 20);
    for (i, len) in [5, 4, 5].into_iter().enumerate() { l_set_cell(&mut p, DATA + 8 * i as u32, len, (i as i16, 0)); }
    body(&mut p, win);
    p.exit();
    rig.m.start(&p.0, 0).unwrap();
    World { rig, win }
}

fn lh(w: &World) -> u32 { w.rig.m.mem.read_u32(LH).unwrap() }
fn rec(w: &World, off: u32, n: u32) -> Vec<i16> { let p = w.rig.m.mm.block(lh(w)).unwrap().ptr; (0..n).map(|i| w.rig.m.mem.read_i16(p + off + 2 * i).unwrap()).collect() }
fn list_kind(w: &World) -> ControlKind { w.rig.m.tr.projected(Obj::List(lh(w))).expect("projected").model.kind.clone() }
fn rows(names: &[&str]) -> Vec<Vec<String>> { names.iter().map(|n| vec![n.to_string()]).collect() }
fn word(w: &World, off: u32) -> u16 { w.rig.m.mem.read_u16(OUT + off).unwrap() }

#[test]
fn the_frame_table_is_the_inside_macintosh_equate_table() {
    assert_eq!(FRAMES.len(), 26, "every List Manager selector has a frame, so none can unbalance the stack");
    let pops: Vec<(u16, u32)> = FRAMES.iter().map(|f| (f.sel, f.pop)).collect();
    assert!(pops.windows(2).all(|p| p[0].0 + 4 == p[1].0), "selectors are every 4 from $00 to $64: {pops:?}");
    let get = |n: &str| FRAMES.iter().find(|f| f.name == n).unwrap();
    assert_eq!((get("LNew").sel, get("LNew").pop), (0x44, 28), "selector + 4 + 4 + 4 + 2 + 4 + 4 x 2");
    assert_eq!((get("LClick").sel, get("LClick").pop), (0x18, 12));
    assert_eq!((get("LSetCell").sel, get("LGetCell").sel, get("LSetSelect").sel), (0x58, 0x38, 0x5C));
}

#[test]
fn lnew_makes_a_real_list_rec_and_the_text_ldef_list_is_a_native_list() {
    let mut w = world(0, |p, _| { p.l(0).trap(0xA9F4); });
    let (step, _) = w.rig.m.step(1_000_000);
    assert_eq!(step, Step::Exited, "{:#?}", w.rig.m.log);
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
    assert_eq!(rec(&w, L_RVIEW, 4), [10, 20, 70, 140], "rView is the program's rectangle");
    assert_eq!(rec(&w, L_DATA_BOUNDS, 4), [0, 0, 3, 1], "LAddRow grew dataBounds to 3 rows, 1 column");
    let tx = w.rig.m.mem.read_i16(w.win + 74).unwrap();
    assert_eq!(rec(&w, L_CELL_SIZE, 2), [tx.max(9) + 2, 120], "cSize (0,0): txSize + 2 high, the view width / columns wide");
    assert_eq!(rec(&w, L_VISIBLE, 4), [0, 0, 3, 1], "all 3 rows fit the 60-pixel view");
    assert_eq!(w.rig.m.mem.read_u16(OUT + 20).unwrap(), 0, "LAddRow returns the first new row");
    let adds: Vec<_> = w.rig.m.ui.calls.iter().filter_map(|c| match c { UiCall::AddControl(_, m) => Some(m.clone()), _ => None }).collect();
    assert_eq!(adds.len(), 1, "one native control (the cells arrived by SetControl, not by rebuilds): {adds:?}");
    assert_eq!(adds[0].rect, GuestRect { left: 20, top: 10, width: 120, height: 60 });
    assert_eq!(adds[0].token, CallbackToken(u64::from(lh(&w))));
    assert_eq!(list_kind(&w), ControlKind::List { columns: vec![], rows: rows(&["alpha", "beta", "gamma"]), selected: None, top: 0 });
}

fn l_do_draw(p: &mut Prog, on: bool) { p.b(on as u8).rgn_arg(LH); sel(p, 0x2C); }
fn list_flags(w: &World) -> u8 { let p = w.rig.m.mm.block(lh(w)).unwrap().ptr; w.rig.m.mem.read_u8(p + heirloom_mac68k_engine::translate::lists::L_LIST_FLAGS).unwrap() }

#[test]
fn ldodraw_sets_and_clears_the_drawing_mode_off_bit_and_the_frame_is_balanced() {
    // PROCEDURE LDoDraw(drawIt: BOOLEAN; lHandle): AMS clears lDrawingModeOff (listFlags bit 3) for TRUE, sets it for FALSE. The frame
    // is selector + handle + a Boolean slot = 8 bytes (the FRAMES table, checked by its own test).
    let mut w = world(0, |p, _| { l_do_draw(p, false); p.exit(); });
    let (step, _) = w.rig.m.step(1_000_000);
    assert_eq!(step, Step::Exited, "{:#?}", w.rig.m.log);
    assert!(w.rig.m.refused.is_empty(), "LDoDraw is implemented: {:?}", w.rig.m.refused);
    assert_eq!(list_flags(&w), 0x08, "LDoDraw(FALSE) sets lDrawingModeOff");
    let mut w = world(0, |p, _| { l_do_draw(p, false); l_do_draw(p, true); p.exit(); });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited);
    assert_eq!(list_flags(&w), 0, "LDoDraw(TRUE) clears it again (negative control: a handler that only ever sets it is RED here)");
    // other listFlags bits are left alone
    let mut w = world(0, |p, _| { p.exit(); });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited);
    let pr = w.rig.m.mm.block(lh(&w)).unwrap().ptr;
    w.rig.m.mem.write_u8(pr + heirloom_mac68k_engine::translate::lists::L_LIST_FLAGS, 0x02).unwrap();
    let mut p = Prog::new();
    l_do_draw(&mut p, false);
    l_do_draw(&mut p, true);
    w.rig.m.start(&p.exit().0, 0).unwrap();
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited);
    assert_eq!(list_flags(&w), 0x02, "lDoVAutoscroll survives a draw-off / draw-on pair");
}

/// WaitNextEvent(mouseDownMask, &EV, 60, nil) then GlobalToLocal(EV.where): the mouseUp the native click queues stays queued.
fn wait_mouse_down(p: &mut Prog, win: u32) {
    p.set_port(win).clr_w().w(2).l(EV).l(60).l(0).trap(0xA860).pop_w(OUT + 2);
    p.l(EV + 10).trap(0xA871);
}

fn click_twice(body: impl FnOnce(&mut Prog, u32)) -> World {
    world(0, |p, win| {
        for save in [OUT + 4, OUT + 6] { wait_mouse_down(p, win); l_click(p, save); }
        body(p, win);
    })
}
fn push_row(w: &mut World, row: u32) {
    let (win, ctl): (UiObjectRef, UiObjectRef) = (w.rig.m.windows[&w.win].native.unwrap(), w.rig.m.tr.projected(Obj::List(lh(w))).unwrap().native);
    w.rig.m.ui.push_intent(win, ctl, Intent::SelectRow(row)).unwrap();
}

#[test]
fn a_native_row_choice_is_a_mouse_down_the_program_answers_with_lclick_and_a_second_one_is_a_double_click() {
    let mut w = click_twice(|p, _| { l_get_select(p, false, SCRATCH2, OUT + 8); p.exit(); });
    w.rig.m.mem.write_u32(SCRATCH2, pt(1, 0)).unwrap();
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Blocked, "{:#?}", w.rig.m.log);
    push_row(&mut w, 1);
    w.rig.m.pump_ui();
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Blocked, "{:#?}", w.rig.m.log);
    assert_eq!(word(&w, 4), 0, "the first LClick is a single click (FALSE)");
    assert_eq!(list_kind(&w), ControlKind::List { columns: vec![], rows: rows(&["alpha", "beta", "gamma"]), selected: Some(1), top: 0 }, "the cell LClick selected is the row the user chose");
    let cell_v = rec(&w, L_CELL_SIZE, 2)[0];
    let mouse = |o: u32| w.rig.m.mem.read_u16(heirloom_mac68k_engine::lowmem::MOUSE + o).unwrap() as i16;
    assert_eq!((mouse(0), mouse(2)), (10 + cell_v + cell_v / 2 + WIN_TOP, 20 + 60 + WIN_LEFT), "the mouseDown is in the middle of row 1 (global: the window origin added)");
    assert_eq!(w.rig.m.ev.queue.iter().map(|e| e.what).collect::<Vec<_>>(), vec![2], "the program consumed the mouseDown; the click's mouseUp is still queued");
    push_row(&mut w, 1);
    w.rig.m.pump_ui();
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    assert_eq!(word(&w, 6), 0x0100, "the same cell again within the double-click time: LClick returns TRUE");
    assert_eq!(word(&w, 8), 0x0100, "LGetSelect says (1,0) is selected");
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
}

#[test]
fn lclick_geometry_and_selection_flags_follow_inside_macintosh() {
    // an ordinary click replaces the selection, a Shift click extends it, lOnlyOne keeps one, a click outside rView changes nothing
    let mut w = world(0, |p, _| {
        l_set_select(p, true, (0, 0));
        l_set_select(p, true, (2, 0));
        p.exit();
    });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    let h = lh(&w);
    assert_eq!(w.rig.m.tr.lists[&h].selected.iter().copied().collect::<Vec<_>>(), vec![(0, 0), (2, 0)], "LSetSelect adds");
    let cell_v = rec(&w, L_CELL_SIZE, 2)[0];
    let click = |w: &mut World, v: i16, h_: i16, mods: u16| -> bool {
        let mut p = Prog::new();
        p.clr_w().l(pt(v, h_)).w(mods as i16).l(lh(w)); sel(&mut p, 0x18); p.pop_w(OUT + 30).exit();
        w.rig.m.start(&p.0, 0).unwrap();
        w.rig.m.run_loop(100_000).unwrap();
        w.rig.m.mem.read_u16(OUT + 30).unwrap() != 0
    };
    click(&mut w, 10 + cell_v + 1, 25, 0);
    assert_eq!(w.rig.m.tr.lists[&h].selected.iter().copied().collect::<Vec<_>>(), vec![(1, 0)], "an ordinary click clears the others");
    click(&mut w, 10 + 2 * cell_v + 1, 25, 0x0200);
    assert_eq!(w.rig.m.tr.lists[&h].selected.iter().copied().collect::<Vec<_>>(), vec![(1, 0), (2, 0)], "Shift keeps the others");
    click(&mut w, 200, 25, 0);
    assert_eq!(w.rig.m.tr.lists[&h].selected.iter().copied().collect::<Vec<_>>(), vec![(1, 0), (2, 0)], "a click outside rView selects nothing and changes nothing");
    click(&mut w, 10 + 3 * cell_v + 1, 25, 0);
    assert!(w.rig.m.tr.lists[&h].selected.is_empty(), "a click on blank space inside rView deselects (no cell under it)");
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
}

#[test]
fn rows_and_cells_shift_with_laddrow_and_ldelrow_and_the_native_list_follows() {
    let mut w = world(0, |p, _| {
        l_set_select(p, true, (1, 0)); // beta
        l_add_row(p, 1, 1, OUT + 24); // a new empty row BEFORE row 1: alpha, "", beta, gamma
        l_set_cell(p, DATA + 24, 5, (1, 0)); // alpha, delta, beta, gamma
        l_del_row(p, 1, 0); // delta, beta, gamma
        p.exit();
    });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    assert_eq!(w.rig.m.mem.read_u16(OUT + 24).unwrap(), 1, "LAddRow returns the row it inserted at");
    assert_eq!(rec(&w, L_DATA_BOUNDS, 4), [0, 0, 3, 1], "3 + 1 - 1 rows");
    assert_eq!(list_kind(&w), ControlKind::List { columns: vec![], rows: rows(&["delta", "beta", "gamma"]), selected: Some(1), top: 0 }, "beta's selection moved down with it, then up again");
    assert!(w.rig.m.refused.is_empty(), "{:?}", w.rig.m.refused);
    // NEGATIVE controls: a zero count adds and deletes nothing; a row past the end appends
    let mut p = Prog::new();
    p.clr_w().w(0).w(0).rgn_arg(LH); sel(&mut p, 0x08); p.pop_w(OUT + 26);
    p.w(0).w(0).rgn_arg(LH); sel(&mut p, 0x24);
    p.clr_w().w(1).w(99).rgn_arg(LH); sel(&mut p, 0x08); p.pop_w(OUT + 28).exit();
    w.rig.run(&p);
    assert_eq!(rec(&w, L_DATA_BOUNDS, 4), [0, 0, 4, 1], "only the append changed the bounds");
    assert_eq!(w.rig.m.mem.read_u16(OUT + 28).unwrap(), 3, "past the end appends at the end");
}

#[test]
fn cells_out_of_range_selectors_without_a_frame_and_dead_handles_are_refused_by_name() {
    let mut w = world(0, |p, _| {
        l_set_cell(p, DATA, 5, (3, 0)); // row 3 does not exist
        l_set_cell(p, DATA, 5, (0, 1)); // column 1 does not exist
        l_set_select(p, true, (-1, 0));
        p.clr_l().w(0).w(0x7C).trap(0xA9E7).pop_l(OUT + 40); // selector $7C is no List Manager selector
        p.exit();
    });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    let r = &w.rig.m.refused;
    assert!(r.iter().filter(|x| x.contains("cell (3,0) outside the list's dataBounds")).count() == 1, "{r:?}");
    assert!(r.iter().any(|x| x.contains("LSetCell") && x.contains("cell (0,1)")), "{r:?}");
    assert!(r.iter().any(|x| x.contains("LSetSelect") && x.contains("cell (-1,0)")), "{r:?}");
    assert!(r.iter().any(|x| x.contains("selector $7C is not a List Manager selector")), "{r:?}");
    assert!(w.rig.m.tr.lists[&lh(&w)].cells.len() == 3 && w.rig.m.tr.lists[&lh(&w)].selected.is_empty(), "refusals changed nothing");
    // a handle that is not a list
    let mut p = Prog::new();
    p.l(0x1234); sel(&mut p, 0x28); p.exit();
    w.rig.m.refused.clear();
    w.rig.m.start(&p.0, 0).unwrap();
    w.rig.m.run_loop(100_000).unwrap();
    assert!(w.rig.m.refused.iter().any(|x| x.contains("0x1234 is not a ListHandle")), "{:?}", w.rig.m.refused);
}

#[test]
fn a_custom_ldef_is_not_projected_but_the_list_still_works_and_dispose_retires_the_native_one() {
    // negative control first: LDEF 128 is the program's own definition
    let mut w = world(128, |p, _| { p.exit(); });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    assert!(w.rig.m.tr.projected(Obj::List(lh(&w))).is_none(), "no role is inferred from a custom LDEF");
    assert!(w.rig.m.log.iter().any(|l| l.contains("LDEF 128 is not the standard text LDEF")), "{:#?}", w.rig.m.log);
    assert_eq!(w.rig.m.tr.lists[&lh(&w)].cells.len(), 3, "but the data model works: the program's LSetCell calls were kept");
    // the standard LDEF IS projected, and LDispose retires it and forgets the handle
    let mut w = world(0, |p, _| { p.rgn_arg(LH); sel(p, 0x28); p.rgn_arg(LH); sel(p, 0x28); p.exit(); });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    assert!(w.rig.m.ui.calls.iter().any(|c| matches!(c, UiCall::RemoveControl(..))), "{:?}", w.rig.m.ui.calls);
    assert!(w.rig.m.tr.lists.is_empty() && w.rig.m.tr.live_count() == 0);
    assert!(w.rig.m.refused.iter().any(|x| x.contains("LDispose") && x.contains("is not a ListHandle")), "the second LDispose: {:?}", w.rig.m.refused);
}

#[test]
fn scrolling_sizing_and_a_native_scroll_move_the_visible_rows_and_the_native_top() {
    let mut w = world(0, |p, _| {
        l_add_row(p, 5, 3, OUT + 24); // 8 rows
        p.w(120).w(28).rgn_arg(LH); sel(p, 0x60); // LSize(width 120, height 28): two 14-pixel rows
        p.exit();
    });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited, "{:#?}", w.rig.m.log);
    let cell_v = rec(&w, L_CELL_SIZE, 2)[0];
    let want_rows = (28 + cell_v - 1) / cell_v;
    assert_eq!(rec(&w, L_RVIEW, 4), [10, 20, 38, 140], "LSize resized rView");
    assert_eq!(rec(&w, L_VISIBLE, 4), [0, 0, want_rows, 1]);
    let top = |w: &World| match list_kind(w) { ControlKind::List { top, rows, .. } => (top, rows.len()), k => panic!("{k:?}") };
    assert_eq!(top(&w), (0, 8));
    let run = |w: &mut World, f: &dyn Fn(&mut Prog)| { let mut p = Prog::new(); f(&mut p); p.exit(); w.rig.run(&p); };
    run(&mut w, &|p| { p.w(0).w(3).rgn_arg(LH); sel(p, 0x50); }); // LScroll(dCols 0, dRows 3)
    assert_eq!(rec(&w, L_VISIBLE, 4), [3, 0, 3 + want_rows, 1]);
    assert_eq!(top(&w).0, 3, "the native list is scrolled to the same row");
    run(&mut w, &|p| { p.w(0).w(100).rgn_arg(LH); sel(p, 0x50); }); // far past the end: clamped to the last row
    assert_eq!(rec(&w, L_VISIBLE, 4)[0], 7);
    run(&mut w, &|p| { l_set_select(p, true, (0, 0)); p.rgn_arg(LH); sel(p, 0x10); }); // LAutoScroll brings the selected cell into view
    assert_eq!(rec(&w, L_VISIBLE, 4)[0], 0, "scrolled back up to the selected row 0");
    // a native scroll writes the same record
    let (win, ctl) = (w.rig.m.windows[&w.win].native.unwrap(), w.rig.m.tr.projected(Obj::List(lh(&w))).unwrap().native);
    w.rig.m.ui.push_intent(win, ctl, Intent::ScrollTo(5)).unwrap();
    w.rig.m.pump_ui();
    assert_eq!((rec(&w, L_VISIBLE, 4)[0], top(&w).0), (5, 5));
    // NEGATIVE: past the last row is refused and the record keeps its origin
    w.rig.m.ui.push_intent(win, ctl, Intent::ScrollTo(8)).unwrap();
    w.rig.m.pump_ui();
    assert!(w.rig.m.refused.iter().any(|x| x.contains("ScrollTo(8)") && x.contains("outside the list's 8 rows")), "{:?}", w.rig.m.refused);
    assert_eq!(rec(&w, L_VISIBLE, 4)[0], 5);
}

#[test]
fn native_selection_of_a_row_that_does_not_exist_or_is_scrolled_out_of_view_is_refused() {
    let mut w = click_twice(|p, _| { p.exit(); });
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Blocked, "{:#?}", w.rig.m.log);
    let (win, ctl) = (w.rig.m.windows[&w.win].native.unwrap(), w.rig.m.tr.projected(Obj::List(lh(&w))).unwrap().native);
    for i in [Intent::SelectRow(3), Intent::SelectRow(900), Intent::ScrollTo(3), Intent::Activate] { w.rig.m.ui.push_intent(win, ctl, i).unwrap(); }
    w.rig.m.pump_ui();
    let r = &w.rig.m.refused;
    assert!(r.iter().any(|x| x.contains("SelectRow(3)") && x.contains("outside the list's 3 rows")), "{r:?}");
    assert!(r.iter().any(|x| x.contains("SelectRow(900)")), "{r:?}");
    assert!(r.iter().any(|x| x.contains("ScrollTo(3)")), "{r:?}");
    assert!(r.iter().any(|x| x.contains("Activate") && x.contains("means nothing for a list")), "{r:?}");
    assert!(w.rig.m.ev.queue.iter().all(|e| e.what != 1), "refused intents produced no guest input");
}

#[test]
fn lnew_with_scrollv_makes_a_real_hidden_scroll_bar_control_in_vscroll_and_ldispose_frees_it() {
    // Moving Van does `SetCRefCon((**list).vScroll, ...)` right after LNew(.., scrollV = TRUE): a NIL vScroll was "0x0 is not a control"
    let mut rig = Rig::new();
    let win = rig.win;
    set_rect(&mut rig.m, VIEW, 10, 20, 70, 140);
    set_rect(&mut rig.m, BOUNDS, 0, 0, 0, 1);
    let mut p = Prog::new();
    p.clr_l().l(VIEW).l(BOUNDS).l(pt(0, 0)).w(0).l(win).b(1).b(0).b(0).b(1); // drawIt, hasGrow, scrollH FALSE, scrollV TRUE
    sel(&mut p, 0x44);
    p.pop_l(LH);
    p.exit();
    rig.m.start(&p.0, 0).unwrap();
    assert_eq!(rig.m.step(1_000_000).0, Step::Exited, "{:#?}", rig.m.log);
    let w = World { rig, win };
    let (v, h) = (rec(&w, 28, 2), rec(&w, 32, 2));
    let v = (u32::from(v[0] as u16) << 16) | u32::from(v[1] as u16);
    assert_ne!(v, 0, "vScroll is a control handle");
    assert_eq!((h[0], h[1]), (0, 0), "hScroll was not asked for");
    let cp = w.rig.m.mm.block(v).expect("a live ControlRecord").ptr;
    assert_eq!(w.rig.m.mem.read_u8(cp + 16).unwrap(), 0, "contrlVis 0: the native list draws its own scroll bar, this one is never projected");
    // LDispose frees it
    let mut w = w;
    let mut p = Prog::new();
    p.rgn_arg(LH); sel(&mut p, 0x28);
    p.exit();
    w.rig.m.start(&p.0, 0).unwrap();
    assert_eq!(w.rig.m.step(1_000_000).0, Step::Exited);
    assert!(w.rig.m.mm.block(v).is_none(), "the scroll bar goes with the list (negative control: a leak keeps it)");
}
