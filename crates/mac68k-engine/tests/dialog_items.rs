//! `SetDialogItem` (Inside Macintosh I, Dialog Manager): the call stores the item's type, handle and box and nothing else; the
//! engine keeps the dialog's `DITL` copy, its slot table and the native part of a text item in step with it.
mod common;
use common::{set_rect, Prog, GLOBALS};
use guestui_abi::{ControlKind, RecordingGuestUi, UiCall};
use mac68k_engine::fixture::{cells, dialog_fork, ScriptArg::{Lit, Mem}, ScriptCall};
use mac68k_engine::translate::Obj;
use mac68k_engine::{Mac, Step};

const BOX: u32 = 0x3300;
const BACK_TYPE: u32 = 0x3320;
const BACK_ITEM: u32 = 0x3324;
const BACK_BOX: u32 = 0x3328;
const USER_PROC: u32 = 0x8000; // clear of the trap stubs (0x4000..0x5400), where an A-line word is a call to the original routine

fn call(name: &'static str, args: &[mac68k_engine::fixture::ScriptArg]) -> ScriptCall { ScriptCall { name, args: args.to_vec(), save: None } }

/// A machine with the dialog resources, `GetNewDialog` 128 run to completion, ready for the program under test.
fn machine() -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(dialog_fork()).unwrap());
    m
}

fn run(m: &mut Mac<RecordingGuestUi>, calls: &[ScriptCall]) {
    let mut p = Prog::new();
    let open = ScriptCall { name: "GetNewDialog", args: vec![Lit(128), Lit(0), Lit(0xFFFF_FFFF)], save: Some(cells::DIALOG) };
    p.script(m, &[open]).script(m, calls).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
}

fn text_handle(m: &mut Mac<RecordingGuestUi>, s: &str) -> u32 {
    let h = m.mm.new_handle(&mut m.mem, &mut m.heap, s.len() as u32, false).unwrap();
    let p = m.mm.block(h).unwrap().ptr;
    m.mem.write_bytes(p, s.as_bytes()).unwrap();
    h
}

fn win(m: &Mac<RecordingGuestUi>) -> u32 { *m.tr.dlg.dialogs.keys().next().unwrap() }
fn rect_at(m: &Mac<RecordingGuestUi>, a: u32) -> [i16; 4] { [0, 2, 4, 6].map(|o| m.mem.read_i16(a + o).unwrap()) }

#[test]
fn set_dialog_item_replaces_type_handle_and_box_and_get_dialog_item_reads_them_back() {
    let mut m = machine();
    let h = text_handle(&mut m, "Typed");
    set_rect(&mut m, BOX, 100, 30, 118, 190);
    run(&mut m, &[call("SetDialogItem", &[Mem(cells::DIALOG), Lit(4), Lit(16), Lit(h), Lit(BOX)]),
                  call("GetDialogItem", &[Mem(cells::DIALOG), Lit(4), Lit(BACK_TYPE), Lit(BACK_ITEM), Lit(BACK_BOX)])]);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!(m.mem.read_u16(BACK_TYPE).unwrap(), 16);
    assert_eq!(m.mem.read_u32(BACK_ITEM).unwrap(), h);
    assert_eq!(rect_at(&m, BACK_BOX), [100, 30, 118, 190]);
    // the DialogRecord's own item list (what a program reads through `DialogPeek(d)->items`) holds the same item
    let w = win(&m);
    let items = m.mem.read_u32(w + 156).unwrap();
    let ip = m.mm.block(items).unwrap().ptr;
    let mut at = ip + 2;
    for _ in 0..3 { at += 14 + u32::from(m.mem.read_u8(at + 13).unwrap()); at += at & 1; } // items 1..3 (data padded to even)
    assert_eq!(m.mem.read_u32(at).unwrap(), h);
    assert_eq!(rect_at(&m, at + 4), [100, 30, 118, 190]);
    assert_eq!(m.mem.read_u8(at + 12).unwrap(), 16);
}

#[test]
fn a_static_text_item_made_editable_is_a_native_field_at_the_new_box_and_the_old_label_goes() {
    let mut m = machine();
    let h = text_handle(&mut m, "Typed");
    set_rect(&mut m, BOX, 100, 30, 118, 190);
    run(&mut m, &[call("SetDialogItem", &[Mem(cells::DIALOG), Lit(4), Lit(16), Lit(h), Lit(BOX)])]);
    let w = win(&m);
    let slot = &m.tr.dlg.dialogs[&w].list[3];
    assert_eq!((slot.raw_type, slot.handle), (16, h));
    assert!(m.tr.projected(Obj::Text(h)).is_some(), "the new handle is projected");
    let adds: Vec<_> = m.ui.calls.iter().filter_map(|c| match c { UiCall::AddControl(_, x) => Some(x.clone()), _ => None }).collect();
    let field = adds.iter().rev().find(|x| matches!(&x.kind, ControlKind::Field { text, .. } if text == "Typed")).expect("a native field with the new handle's text");
    assert_eq!((field.rect.left, field.rect.top, field.rect.width, field.rect.height), (30, 100, 160, 18));
    assert!(m.ui.calls.iter().any(|c| matches!(c, UiCall::RemoveControl(..))), "the label that no longer stands for item 4 is retired");
}

#[test]
fn a_user_item_takes_a_procedure_and_is_not_projected() {
    let mut m = machine();
    set_rect(&mut m, BOX, 60, 220, 76, 260);
    run(&mut m, &[call("SetDialogItem", &[Mem(cells::DIALOG), Lit(6), Lit(128), Lit(USER_PROC), Lit(BOX)])]);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let w = win(&m);
    let s = &m.tr.dlg.dialogs[&w].list[5];
    assert_eq!((s.raw_type, s.handle), (128, USER_PROC));
    assert!(m.tr.projected(Obj::Text(USER_PROC)).is_none() && m.tr.projected(Obj::Control(USER_PROC)).is_none());
}

#[test]
fn set_dialog_item_refuses_by_name_and_changes_nothing() {
    for (n, ty, h, why) in [(9, 8, 0u32, "no item 9"), (4, 4, 0x1234, "not a ControlHandle"), (4, 8, 0x1234, "not a live Handle"), (4, 3, 0, "item type 3")] {
        let mut m = machine();
        set_rect(&mut m, BOX, 1, 2, 3, 4);
        run(&mut m, &[call("SetDialogItem", &[Mem(cells::DIALOG), Lit(n), Lit(ty), Lit(h), Lit(BOX)])]);
        assert!(m.refused.iter().any(|r| r.contains(why)), "{why}: {:?}", m.refused);
        let w = win(&m);
        let s = &m.tr.dlg.dialogs[&w].list[3];
        assert_eq!(s.raw_type, 8, "{why}: item 4 is untouched");
    }
}

// ---- user items: the program's draw procedure runs when the Dialog Manager draws the dialog ----

const SEEN_ITEM: u32 = 0x3600;
const SEEN_WIN: u32 = 0x3604;
const D3_AFTER: u32 = 0x3608;
const CALLS: u32 = 0x360C;
const INK: u32 = 0x3320; // the rectangle the procedure fills

fn words(p: &mut Prog, w: &[u16]) { for x in w { p.0.extend_from_slice(&x.to_be_bytes()); } }

/// `PROCEDURE UserItem(theWindow: WindowPtr; itemNo: INTEGER)` in 68k: records its arguments, counts its calls, clobbers D3, fills the
/// rectangle at `INK` through the port the Dialog Manager set, and returns THROUGH A FLAGGED ADDRESS (a 24-bit program's idiom: the
/// high byte is ignored by the bus), popping its six argument bytes as a Pascal procedure does.
fn user_item_proc() -> Vec<u8> {
    let mut p = Prog::new();
    words(&mut p, &[0x33EF, 0x0004, 0x0000, 0x3600]); // move.w 4(sp),SEEN_ITEM
    words(&mut p, &[0x23EF, 0x0006, 0x0000, 0x3604]); // move.l 6(sp),SEEN_WIN
    words(&mut p, &[0x5279, 0x0000, 0x360C]);         // addq.w #1,CALLS
    words(&mut p, &[0x76FF]);                         // moveq #-1,d3
    p.l(INK).trap(0xA8A2);                            // PaintRect(INK)
    words(&mut p, &[0x205F, 0x5C8F, 0x2008, 0x0080, 0x6000, 0x0000, 0x2040, 0x4ED0]); // move.l (sp)+,a0; addq.l #6,sp; move.l a0,d0; ori.l #$60000000,d0; movea.l d0,a0; jmp (a0)
    p.0
}

fn with_user_item(m: &mut Mac<RecordingGuestUi>) {
    for (a, b) in user_item_proc().iter().enumerate() { m.mem.write_u8(USER_PROC + a as u32, *b).unwrap(); }
    set_rect(m, BOX, 60, 220, 76, 260);
    set_rect(m, INK, 62, 222, 70, 230);
}

#[test]
fn draw_dialog_calls_each_user_item_procedure_with_the_window_and_item_and_hands_back_the_programs_registers() {
    let mut m = machine();
    with_user_item(&mut m);
    let mut p = Prog::new();
    let open = ScriptCall { name: "GetNewDialog", args: vec![Lit(128), Lit(0), Lit(0xFFFF_FFFF)], save: Some(cells::DIALOG) };
    p.script(&m, &[open, call("SetDialogItem", &[Mem(cells::DIALOG), Lit(6), Lit(128), Lit(USER_PROC), Lit(BOX)])]);
    words(&mut p, &[0x7607]); // moveq #7,d3
    p.script(&m, &[call("DrawDialog", &[Mem(cells::DIALOG)])]);
    words(&mut p, &[0x23C3, 0x0000, 0x3608]); // move.l d3,D3_AFTER
    p.exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let w = win(&m);
    assert_eq!((m.mem.read_u16(SEEN_ITEM).unwrap(), m.mem.read_u32(SEEN_WIN).unwrap(), m.mem.read_u16(CALLS).unwrap()), (6, w, 1), "called once, as proc(dialog, 6)");
    assert_eq!(m.mem.read_u32(D3_AFTER).unwrap(), 7, "the program's D3 is back (the procedure clobbered it)");
    let canvas = &m.qd.canvases[&w];
    let ink = (62..70).flat_map(|y| (222..230).map(move |x| (x, y))).filter(|&(x, y)| canvas.get(x, y) == Some(1)).count();
    assert_eq!(ink, 64, "the procedure drew into the dialog's own content plane (local coordinates)");
    assert_eq!(canvas.pixels().iter().filter(|&&px| px == 1).count(), 64, "and nothing else");
}

#[test]
fn modal_dialog_draws_the_user_items_before_it_waits_and_waits_exactly_as_before() {
    let mut m = machine();
    with_user_item(&mut m);
    let mut p = Prog::new();
    let open = ScriptCall { name: "GetNewDialog", args: vec![Lit(128), Lit(0), Lit(0xFFFF_FFFF)], save: Some(cells::DIALOG) };
    p.script(&m, &[open, call("SetDialogItem", &[Mem(cells::DIALOG), Lit(6), Lit(128), Lit(USER_PROC), Lit(BOX)]), call("ModalDialog", &[Lit(0), Lit(cells::HIT1)])]);
    p.exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "{:#?}", m.log);
    assert_eq!(m.mem.read_u16(CALLS).unwrap(), 1, "the procedure ran once, before the wait (the trap word that re-ran did not draw again)");
    let w = win(&m);
    assert_eq!(m.qd.canvases[&w].pixels().iter().filter(|&&px| px == 1).count(), 64);
    // the wait is the ordinary ModalDialog wait: the OK button's hit ends it
    let (ok, _) = { let h = m.tr.dlg.dialogs[&w].list[0].handle; (m.tr.projected(Obj::Control(h)).unwrap().native, h) };
    m.ui.push_intent(m.windows[&w].native.unwrap(), ok, guestui_abi::Intent::Activate).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert_eq!(m.mem.read_u16(cells::HIT1).unwrap(), 1);
    assert_eq!(m.mem.read_u16(CALLS).unwrap(), 1, "still one call");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

// ---- NewDialog: a dialog from a program-built item list -----------------------------------------------------------------

/// A DITL with a push button "OK" and static text "Hi" (count - 1, then per item: placeholder, rect, type, length, data padded to even).
fn ditl_bytes() -> Vec<u8> {
    let item = |rect: [i16; 4], ty: u8, data: &[u8]| {
        let mut v = vec![0u8; 4];
        for r in rect { v.extend_from_slice(&r.to_be_bytes()); }
        v.push(ty);
        v.push(data.len() as u8);
        v.extend_from_slice(data);
        if data.len() & 1 == 1 { v.push(0); }
        v
    };
    let mut d = 1i16.to_be_bytes().to_vec();
    d.extend(item([80, 200, 100, 260], 4, b"OK"));
    d.extend(item([10, 20, 26, 200], 8, b"Hi"));
    d
}

#[test]
fn new_dialog_builds_a_dialog_from_the_programs_own_item_list_and_dispose_dialog_frees_the_list() {
    let mut m = machine();
    let items = text_handle(&mut m, ""); // a handle of the right size is made below
    let bytes = ditl_bytes();
    m.mm.set_handle_size(&mut m.mem, &mut m.heap, items, bytes.len() as u32).unwrap();
    let p = m.mm.block(items).unwrap().ptr;
    m.mem.write_bytes(p, &bytes).unwrap();
    set_rect(&mut m, BOX, 40, 60, 160, 300);
    put_pstr(&mut m, 0x3500, "Mine");
    let new = ScriptCall { name: "NewDialog", args: vec![Lit(0), Lit(BOX), Lit(0x3500), Lit(1), Lit(1), Lit(0xFFFF_FFFF), Lit(0), Lit(0x1234), Lit(items)], save: Some(cells::DIALOG) };
    let mut prog = Prog::new();
    prog.script(&m, &[new]).exit();
    m.start(&prog.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let w = win(&m);
    assert_eq!(m.mem.read_u32(cells::DIALOG).unwrap(), w);
    assert_eq!(m.tr.dlg.dialogs[&w].items, items, "the dialog uses the program's own handle");
    assert_eq!(m.mem.read_u16(w + 108).unwrap(), 2, "windowKind is dialogKind");
    assert_eq!(m.mem.read_u32(w + 152).unwrap(), 0x1234, "refCon");
    assert_eq!(m.mem.read_u16(w + 164).unwrap(), 0xFFFF, "no editText item: editField stays -1");
    assert_eq!(m.mem.read_u16(w + 168).unwrap(), 1, "aDefItem");
    let list = &m.tr.dlg.dialogs[&w].list;
    assert_eq!((list.len(), list[0].raw_type, list[1].raw_type), (2, 4, 8));
    assert!(m.tr.projected(Obj::Control(list[0].handle)).is_some() && m.tr.projected(Obj::Text(list[1].handle)).is_some(), "the button and the text are native parts");
    // the placeholder long of each item in the program's list is its handle now
    assert_eq!(m.mem.read_u32(p + 2).unwrap(), list[0].handle);
    let opened = m.ui.calls.iter().any(|c| matches!(c, UiCall::OpenWindow(x) if x.title == "Mine"));
    assert!(opened, "the title and the visible flag reached the host window");
    // DisposeDialog disposes the item list too
    let mut prog = Prog::new();
    prog.script(&m, &[call("DisposeDialog", &[Mem(cells::DIALOG)])]).exit();
    m.start(&prog.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited);
    assert!(m.mm.block(items).is_none(), "the item list handle is gone");
    // NEGATIVE CONTROL: a NIL item list is refused by name and makes no dialog
    let mut m = machine();
    let nil = ScriptCall { name: "NewDialog", args: vec![Lit(0), Lit(BOX), Lit(0x3500), Lit(1), Lit(1), Lit(0xFFFF_FFFF), Lit(0), Lit(0), Lit(0)], save: Some(cells::DIALOG) };
    set_rect(&mut m, BOX, 40, 60, 160, 300);
    let mut prog = Prog::new();
    prog.script(&m, &[nil]).exit();
    m.start(&prog.0, 0).unwrap();
    m.step(1_000_000);
    assert!(m.refused.iter().any(|r| r.contains("item list is NIL")) && m.tr.dlg.dialogs.is_empty(), "{:?}", m.refused);
}

fn put_pstr(m: &mut Mac<RecordingGuestUi>, at: u32, s: &str) { let mut v = vec![s.len() as u8]; v.extend_from_slice(s.as_bytes()); m.mem.write_bytes(at, &v).unwrap(); }

#[test]
fn get_new_dialog_makes_a_dialog_kind_window_in_the_dialog_font() {
    let mut m = machine();
    m.mem.write_u16(mac68k_engine::lowmem::DLG_FONT, 3).unwrap(); // SetDAFont(geneva)
    run(&mut m, &[]);
    let w = win(&m);
    assert_eq!(m.mem.read_u16(w + 108).unwrap(), 2, "windowKind is dialogKind (2), not userKind (8)");
    assert_eq!(m.mem.read_u16(w + 68).unwrap(), 3, "txFont is DlgFont");
    assert_eq!((m.mem.read_u16(w + 164).unwrap(), m.mem.read_u16(w + 168).unwrap()), (4, 1), "editField is the first editText item (index 4: item 5), aDefItem 1");
}

#[test]
fn a_dlog_with_a_dctb_is_a_colour_window_on_a_colour_machine_and_a_plain_one_otherwise() {
    // Inside Macintosh: on a Mac with Color QuickDraw the Dialog Manager makes the dialog a COLOUR window (NewCWindow) when the DLOG has a 'dctb'.
    // ResEdit 2.1.3's About box then reads (CGrafPtr)port->portPixMap: through a plain GrafPort that was the screen's baseAddr (CopyPixMap
    // refused "0xffaa80 is not a handle").
    let make = |fork: Vec<u8>, profile: &str| {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        m.set_machine(profile).unwrap();
        mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
        m.qd_globals = GLOBALS;
        m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(fork).unwrap());
        run(&mut m, &[]);
        let w = win(&m);
        (m.windows[&w].color, m.mem.read_u16(w + 6).unwrap() & 0xC000)
    };
    assert_eq!(make(mac68k_engine::fixture::color_dialog_fork(), "mac-ii"), (true, 0xC000), "dctb + Color QuickDraw: a CGrafPort (portVersion's two high bits)");
    assert_eq!(make(mac68k_engine::fixture::dialog_fork(), "mac-ii"), (false, 0), "no dctb: a plain GrafPort");
    assert_eq!(make(mac68k_engine::fixture::color_dialog_fork(), "mac-plus"), (false, 0), "no Color QuickDraw: a plain GrafPort whatever the resources");
}

// ---- ModalDialog's filterProc: guest code run through the trampoline ----

/// `FUNCTION Filter(d, VAR event, VAR itemHit): BOOLEAN` in 68k: records the event's `what` and the dialog, counts calls, sets
/// `itemHit` to 2 when `verdict` is TRUE, pops its twelve argument bytes and returns the verdict.
fn filter_proc(verdict: u8) -> Vec<u8> {
    let mut p = Prog::new();
    words(&mut p, &[0x206F, 0x0008, 0x33D0, 0x0000, 0x3600]);       // movea.l 8(sp),a0; move.w (a0),SEEN_ITEM  (the event's what)
    words(&mut p, &[0x23EF, 0x000C, 0x0000, 0x3604]);               // move.l 12(sp),SEEN_WIN (the dialog)
    words(&mut p, &[0x5279, 0x0000, 0x360C]);                       // addq.w #1,CALLS
    words(&mut p, &[0x76FF]);                                       // moveq #-1,d3 (clobbered; the engine puts it back)
    if verdict != 0 { words(&mut p, &[0x206F, 0x0004, 0x30BC, 0x0002]); } // movea.l 4(sp),a0; move.w #2,(a0)
    words(&mut p, &[0x1F7C, verdict as u16, 0x0010]);               // move.b #verdict,16(sp)
    words(&mut p, &[0x225F, 0x4FEF, 0x000C, 0x4ED1]);               // movea.l (sp)+,a1; lea 12(sp),sp; jmp (a1)
    p.0
}

fn modal_with_filter(m: &mut Mac<RecordingGuestUi>, verdict: u8) {
    for (a, b) in filter_proc(verdict).iter().enumerate() { m.mem.write_u8(USER_PROC + a as u32, *b).unwrap(); }
    let mut p = Prog::new();
    let open = ScriptCall { name: "GetNewDialog", args: vec![Lit(128), Lit(0), Lit(0xFFFF_FFFF)], save: Some(cells::DIALOG) };
    p.script(m, &[open, call("ModalDialog", &[Lit(USER_PROC), Lit(cells::HIT1)])]);
    p.exit();
    m.start(&p.0, 0).unwrap();
}

#[test]
fn a_filter_proc_answering_true_ends_modal_dialog_with_its_item_hit() {
    let mut m = machine();
    modal_with_filter(&mut m, 1);
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let w = win(&m);
    assert_eq!((m.mem.read_u16(cells::HIT1).unwrap(), m.mem.read_u32(SEEN_WIN).unwrap(), m.mem.read_u16(SEEN_ITEM).unwrap(), m.mem.read_u16(CALLS).unwrap()), (2, w, 0, 1), "itemHit 2, called as filter(dialog, nullEvent, itemHit) once");
}

#[test]
fn a_filter_proc_answering_false_leaves_the_standard_wait_and_is_not_called_again() {
    let mut m = machine();
    modal_with_filter(&mut m, 0);
    assert_eq!(m.step(1_000_000).0, Step::Blocked, "{:#?}", m.log);
    assert_eq!(m.mem.read_u16(CALLS).unwrap(), 1);
    let w = win(&m);
    let h = m.tr.dlg.dialogs[&w].list[0].handle;
    let native = m.tr.projected(Obj::Control(h)).unwrap().native;
    m.ui.push_intent(m.windows[&w].native.unwrap(), native, guestui_abi::Intent::Activate).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert_eq!((m.mem.read_u16(cells::HIT1).unwrap(), m.mem.read_u16(CALLS).unwrap()), (1, 1), "the button's hit, one filter call");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}
