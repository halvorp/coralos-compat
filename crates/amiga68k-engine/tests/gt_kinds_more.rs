//! The GadTools kinds that hold rows or a position (gt_kinds_rows.rs), driven through the call-script harness: the guest builds the
//! gadgets (CreateContext / CreateGadgetA / OpenWindowTagList with WA_Gadgets), the recording UI sees the native control, a native
//! intent comes back, and the IntuiMessage it queued is read from guest memory. Message facts: AROS workbench/libs/gadtools
//! cycleclass.c, mxclass.c, listviewclass.c, gt_filterimsg.c. Every behaviour has a negative control.
mod common;
use amiga68k_engine::Delivered;
use common::gtscript::*;
use guestui_abi::*;

const LVO_GET_ATTRS: i32 = -174;
const LVO_SET_ATTRS: i32 = -42;
const LVO_CREATE_GADGET: i32 = -30;
const LVO_CREATE_CONTEXT: i32 = -114;
const LVO_OPEN_WINDOW_TAGS: i32 = -606;
const HEAP_START: u32 = 0x20_0000;
const CYCLE: u32 = 7;
const MX: u32 = 5;
const LISTVIEW: u32 = 4;
const NUMBER: u32 = 6;
const PALETTE: u32 = 8;
const SCROLLER: u32 = 9;
const NONE: u32 = u32::MAX;
const GADGETDOWN: u32 = 0x20;
const GADGETUP: u32 = 0x40;
const MOUSEMOVE: u32 = 0x10;
// Tag numbers: AROS include/libraries/gadtools.h (GT_TagBase + n), intuition/gadgetclass.h (PGA_Freedom = TAG_USER + 0x31001).
const GTLV_TOP: u32 = GT + 5;
const GTLV_LABELS: u32 = GT + 6;
const GTLV_READONLY: u32 = GT + 7;
const GTMX_LABELS: u32 = GT + 9;
const GTMX_SPACING: u32 = GT + 61;
const GTMX_ACTIVE: u32 = GT + 10;
const GTNM_NUMBER: u32 = GT + 13;
const GTCY_LABELS: u32 = GT + 14;
const GTCY_ACTIVE: u32 = GT + 15;
const GTSC_TOP: u32 = GT + 21;
const GTSC_TOTAL: u32 = GT + 22;
const GTSC_VISIBLE: u32 = GT + 23;
const GTLV_SHOWSELECTED: u32 = GT + 53;
const GTLV_SELECTED: u32 = GT + 54;
const GTNM_BORDER: u32 = GT + 58;
const GTNM_FORMAT: u32 = GT + 75;
const PGA_FREEDOM: u32 = TAG_USER + 0x31001;
const LORIENT_HORIZ: u32 = 1;
const LORIENT_VERT: u32 = 2;

/// The harness plus the open window: the gadgets in creation order.
struct Rig { m: M, gads: Vec<u32>, win: u32 }
impl Rig {
    /// Create one gadget per `(kind, tags)` in a chain and open a window that shows it and listens for GADGETDOWN/UP/MOUSEMOVE.
    fn open(mut m: M, kinds: &[(u32, u32)]) -> Rig {
        let glist = DATA - 8;
        let ng = (0..kinds.len()).map(|i| new_gadget(&mut m, Some("g"), i as u16 + 1)).collect::<Vec<_>>();
        let wtags = m.tags(&[(WA + 3, 200), (WA + 4, 120), (WA + 7, 0x70), (WA + 9, HEAP_START)]);
        let mut calls = vec![gt(LVO_CREATE_CONTEXT, &[a(0, glist)])];
        for (i, &(kind, tags)) in kinds.iter().enumerate() {
            calls.push(gt(LVO_CREATE_GADGET, &[d(0, kind), a_res(0, i as u32), a(1, ng[i]), a(2, tags)]));
        }
        calls.push(intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, wtags)]));
        m.run(&calls);
        assert_eq!(m.result(0), HEAP_START);
        let gads = (1..=kinds.len() as u32).map(|i| m.result(i)).collect::<Vec<_>>();
        let win = m.result(kinds.len() as u32 + 1);
        Rig { m, gads, win }
    }
    /// The recording UI's model of the control the nth gadget is now.
    fn model(&self, n: usize) -> ControlModel { self.m.a.gt_ctls.get(u64::from(self.gads[n])).expect("projected").model.clone() }
    /// A native intent on the nth gadget; the delivery result.
    fn intent(&mut self, n: usize, intent: Intent) -> Result<Delivered, String> {
        let p = self.m.a.gt_ctls.get(u64::from(self.gads[n])).expect("projected").clone();
        self.m.a.deliver(GuestUiEvent::intent(p.window, p.native, CallbackToken(u64::from(self.gads[n])), intent)).map_err(|e| e.0)
    }
    /// (class, code, IAddress) of a queued message.
    fn msg(&self, d: &Delivered) -> (u32, u16, u32) {
        let Delivered::Queued(at) = d else { panic!("no message queued: {d:?}") };
        (self.m.r32(at + 20), self.m.r16(at + 24), self.m.r32(at + 28))
    }
    fn set(&mut self, n: usize, tags: u32) {
        self.m.run(&[gt(LVO_SET_ATTRS, &[a(0, self.gads[n]), a(1, self.win), a(2, 0), a(3, tags)])]);
    }
    /// GT_GetGadgetAttrsA of one tag.
    fn get(&mut self, n: usize, tag: u32) -> u32 {
        let out = DATA - 32;
        let t = self.m.tags(&[(tag, out)]);
        self.m.run(&[gt(LVO_GET_ATTRS, &[a(0, self.gads[n]), a(1, self.win), a(2, 0), a(3, t)])]);
        self.m.r32(out)
    }
}

/// A NULL-terminated STRPTR array.
fn strs(m: &mut M, items: &[&str]) -> u32 {
    let ptrs: Vec<u32> = items.iter().map(|s| m.s(s)).collect();
    let at = (m.next + 3) & !3;
    for (i, p) in ptrs.iter().chain(&[0]).enumerate() { m.a.mem.write_u32(at + 4 * i as u32, *p).unwrap(); }
    m.next = at + 4 * (ptrs.len() as u32 + 1);
    at
}

/// An Exec List of Nodes whose ln_Name are `names` (lh_Head 0, lh_Tail 4, lh_TailPred 8; Node: succ 0, pred 4, type 8, name 10).
fn list(m: &mut M, names: &[&str]) -> u32 {
    let ptrs: Vec<u32> = names.iter().map(|s| m.s(s)).collect();
    let l = (m.next + 3) & !3;
    let nodes = l + 12;
    m.next = nodes + 14 * names.len() as u32 + 2;
    let node = |i: usize| nodes + 14 * i as u32;
    m.a.mem.write_u32(l, if names.is_empty() { l + 4 } else { node(0) }).unwrap();
    m.a.mem.write_u32(l + 4, 0).unwrap();
    m.a.mem.write_u32(l + 8, if names.is_empty() { l } else { node(names.len() - 1) }).unwrap();
    for (i, p) in ptrs.iter().enumerate() {
        m.a.mem.write_u32(node(i), if i + 1 < names.len() { node(i + 1) } else { l + 4 }).unwrap();
        m.a.mem.write_u32(node(i) + 4, if i == 0 { l } else { node(i - 1) }).unwrap();
        m.a.mem.write_u32(node(i) + 10, *p).unwrap();
    }
    l
}

fn rows(names: &[&str]) -> Vec<Vec<String>> { names.iter().map(|n| vec![n.to_string()]).collect() }

#[test]
fn a_cycle_gadget_is_a_native_cycle_choice_and_a_change_is_gadgetup_with_the_new_index() {
    let mut m = M::new();
    let labels = strs(&mut m, &["Red", "Green", "Blue"]);
    let t = m.tags(&[(GTCY_LABELS, labels), (GTCY_ACTIVE, 1)]);
    let mut r = Rig::open(m, &[(CYCLE, t)]);
    assert_eq!(r.model(0).kind, ControlKind::Choice { style: ChoiceStyle::Cycle, labels: vec!["Red".into(), "Green".into(), "Blue".into()], selected: 1 });
    let d = r.intent(0, Intent::SetValue(ControlValue::Choice(2))).unwrap();
    assert_eq!(r.msg(&d), (GADGETUP, 2, r.gads[0]), "cycleclass.c: GMR_VERIFY, Code = the new active entry; IAddress = the gadget");
    assert_eq!(r.get(0, GTCY_ACTIVE), 2, "GT_GetGadgetAttrsA reads the entry the native choice set");
    assert_eq!(r.get(0, GTCY_LABELS), labels);
    // Negative controls: an index past the labels, and an intent the cycle has no row for, are refused by name and change nothing.
    let e = r.intent(0, Intent::SetValue(ControlValue::Choice(3))).unwrap_err();
    assert!(e.contains("CYCLE_KIND") && e.contains("outside its 3 labels"), "{e}");
    assert!(r.intent(0, Intent::Activate).unwrap_err().contains("means nothing"));
    assert_eq!(r.get(0, GTCY_ACTIVE), 2);
    assert_eq!(r.m.a.refused.len(), 0, "{:?}", r.m.a.refused);
}

#[test]
fn guest_changes_reach_the_native_cycle_and_new_labels_reset_the_active_entry() {
    let mut m = M::new();
    let two = strs(&mut m, &["A", "B"]);
    let three = strs(&mut m, &["X", "Y", "Z"]);
    let t = m.tags(&[(GTCY_LABELS, two), (GTCY_ACTIVE, 1)]);
    let set_active = m.tags(&[(GTCY_ACTIVE, 0)]);
    let set_labels = m.tags(&[(GTCY_LABELS, three)]);
    let bad_active = m.tags(&[(GTCY_ACTIVE, 7)]);
    let mut r = Rig::open(m, &[(CYCLE, t)]);
    r.set(0, set_active);
    assert!(matches!(r.m.a.ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Choice(0)))), "{:?}", r.m.a.ui.calls.last());
    r.get(0, GTCY_ACTIVE);
    r.set(0, set_labels);
    assert_eq!(r.model(0).kind, ControlKind::Choice { style: ChoiceStyle::Cycle, labels: vec!["X".into(), "Y".into(), "Z".into()], selected: 0 },
        "cycleclass.c: GTCY_Labels sets the active entry back to 0; the native control is rebuilt with the new labels");
    // Negative: an active entry past the labels is a named refusal, not a silent clamp, and the gadget keeps its state.
    r.set(0, bad_active);
    assert!(r.m.a.refused.iter().any(|x| x.contains("CYCLE_KIND active entry 7 outside its 3 labels")), "{:?}", r.m.a.refused);
    assert_eq!(r.get(0, GTCY_ACTIVE), 0);
}

#[test]
fn an_mx_gadget_is_a_radio_choice_and_a_change_is_gadgetdown_not_gadgetup() {
    let mut m = M::new();
    let labels = strs(&mut m, &["One", "Two", "Three"]);
    let t = m.tags(&[(GTMX_LABELS, labels), (GTMX_SPACING, 3)]);
    let relabel = m.tags(&[(GTMX_LABELS, labels)]);
    let mut r = Rig::open(m, &[(MX, t)]);
    assert_eq!(r.model(0).kind, ControlKind::Choice { style: ChoiceStyle::Radio, labels: vec!["One".into(), "Two".into(), "Three".into()], selected: 0 });
    let d = r.intent(0, Intent::SetValue(ControlValue::Choice(1))).unwrap();
    assert_eq!(r.msg(&d), (GADGETDOWN, 1, r.gads[0]), "mxclass.c: GACT_IMMEDIATE, no RELVERIFY: GADGETDOWN, Code = the new active entry");
    assert_eq!(r.get(0, GTMX_ACTIVE), 1);
    assert!(r.intent(0, Intent::SetValue(ControlValue::Choice(5))).unwrap_err().contains("MX_KIND: choice 5 outside its 3 labels"));
    // mxclass.c OM_SET knows GTMX_Active only: new labels after creation are a named refusal.
    r.set(0, relabel);
    assert!(r.m.a.refused.iter().any(|x| x.contains("GTMX_Labels") && x.contains("creation only")), "{:?}", r.m.a.refused);
}

#[test]
fn a_listview_is_a_headless_list_selection_is_gadgetup_with_the_row_and_scrolling_is_silent() {
    let mut m = M::new();
    let l = list(&mut m, &["alpha", "beta", "gamma"]);
    let t = m.tags(&[(GTLV_LABELS, l)]);
    let mut r = Rig::open(m, &[(LISTVIEW, t)]);
    assert_eq!(r.model(0).kind, ControlKind::List { columns: vec![], rows: rows(&["alpha", "beta", "gamma"]), selected: None, top: 0 },
        "nothing selected at first (GTLV_Selected ~0)");
    let d = r.intent(0, Intent::SelectRow(1)).unwrap();
    assert_eq!(r.msg(&d), (GADGETUP, 1, r.gads[0]), "listviewclass.c: a click ends with Code = the selected row");
    assert_eq!(r.get(0, GTLV_SELECTED), 1);
    let d = r.intent(0, Intent::ScrollTo(2)).unwrap();
    assert_eq!(d, Delivered::Handled, "gt_filterimsg.c hides the listview's scroller: scrolling sends no message");
    assert_eq!(r.get(0, GTLV_TOP), 2);
    assert_eq!(r.get(0, GTLV_LABELS), l);
    // Negative controls.
    assert!(r.intent(0, Intent::SelectRow(3)).unwrap_err().contains("row 3 outside its 3 rows"));
    assert!(r.intent(0, Intent::ScrollTo(3)).unwrap_err().contains("top 3 outside its 3 rows"));
    assert_eq!(r.get(0, GTLV_SELECTED), 1, "a refused selection changes nothing");
    assert!(r.intent(0, Intent::SetValue(ControlValue::Choice(0))).unwrap_err().contains("means nothing"));
}

#[test]
fn listview_labels_can_be_detached_and_replaced_and_selection_survives_as_in_the_class() {
    let mut m = M::new();
    let l1 = list(&mut m, &["a", "b", "c", "d"]);
    let l2 = list(&mut m, &["x", "y"]);
    let t = m.tags(&[(GTLV_LABELS, l1), (GTLV_TOP, 3), (GTLV_SELECTED, 3)]);
    let detach = m.tags(&[(GTLV_LABELS, NONE)]);
    let replace = m.tags(&[(GTLV_LABELS, l2)]);
    let mut r = Rig::open(m, &[(LISTVIEW, t)]);
    assert_eq!(r.model(0).kind, ControlKind::List { columns: vec![], rows: rows(&["a", "b", "c", "d"]), selected: Some(3), top: 3 });
    r.set(0, detach);
    assert_eq!(r.model(0).kind, ControlKind::List { columns: vec![], rows: vec![], selected: None, top: 0 }, "~0 detaches the list: no rows; GTLV_Labels resets Top");
    assert_eq!(r.get(0, GTLV_SELECTED), 3, "listviewclass.c keeps ld_Selected across GTLV_Labels");
    assert!(r.intent(0, Intent::SelectRow(0)).is_err(), "no rows to select");
    r.set(0, replace);
    assert_eq!(r.model(0).kind, ControlKind::List { columns: vec![], rows: rows(&["x", "y"]), selected: None, top: 0 }, "row 3 does not exist in the new list: nothing shown selected");
    assert!(matches!(r.m.a.ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Rows { .. }))), "the change is a value push, not a rebuild");
}

#[test]
fn a_read_only_listview_and_an_unbridged_show_selected_are_refused_by_name() {
    let mut m = M::new();
    let l = list(&mut m, &["a", "b"]);
    let show = m.tags(&[(GTLV_LABELS, l), (GTLV_SHOWSELECTED, 0x1234)]);
    let ng = new_gadget(&mut m, None, 9);
    let glist = DATA - 8;
    m.run(&[
        gt(LVO_CREATE_CONTEXT, &[a(0, glist)]),
        gt(LVO_CREATE_GADGET, &[d(0, LISTVIEW), a_res(0, 0), a(1, ng), a(2, show)]),
    ]);
    assert_eq!(m.result(1), 0, "the gadget is not made");
    assert!(m.a.refused.iter().any(|x| x.contains("GTLV_ShowSelected")), "{:?}", m.a.refused);
    let mut m2 = M::new();
    let l2 = list(&mut m2, &["a", "b"]);
    let ro2 = m2.tags(&[(GTLV_LABELS, l2), (GTLV_READONLY, 1)]);
    let mut r2 = Rig::open(m2, &[(LISTVIEW, ro2)]);
    assert!(r2.intent(0, Intent::SelectRow(0)).unwrap_err().contains("GTLV_ReadOnly"), "a read-only list takes no selection");
    assert!(r2.intent(0, Intent::ScrollTo(1)).is_ok(), "but it scrolls");
}

#[test]
fn a_scroller_is_a_scrollbar_a_change_is_mousemove_and_an_unchanged_release_is_gadgetup() {
    let mut m = M::new();
    let vert = m.tags(&[(GTSC_TOTAL, 100), (GTSC_VISIBLE, 10), (GTSC_TOP, 5), (PGA_FREEDOM, LORIENT_VERT)]);
    let horiz = m.tags(&[(GTSC_TOTAL, 50), (GTSC_VISIBLE, 5), (PGA_FREEDOM, LORIENT_HORIZ)]);
    let dflt = m.tags(&[(GTSC_TOTAL, 8)]);
    let mut r = Rig::open(m, &[(SCROLLER, vert), (SCROLLER, horiz), (SCROLLER, dflt)]);
    assert_eq!(r.model(0).kind, ControlKind::ScrollBar { vertical: true, total: 100, visible: 10, top: 5 });
    assert_eq!(r.model(1).kind, ControlKind::ScrollBar { vertical: false, total: 50, visible: 5, top: 0 });
    assert_eq!(r.model(2).kind, ControlKind::ScrollBar { vertical: false, total: 8, visible: 2, top: 0 }, "makescroller: horizontal, Visible 2 by default");
    let d = r.intent(0, Intent::ScrollTo(30)).unwrap();
    assert_eq!(r.msg(&d), (MOUSEMOVE, 30, r.gads[0]), "gt_filterimsg.c: a release that changed the position is IDCMP_MOUSEMOVE, Code = GTSC_Top");
    assert_eq!(r.get(0, GTSC_TOP), 30);
    let d = r.intent(0, Intent::ScrollTo(30)).unwrap();
    assert_eq!(r.msg(&d), (GADGETUP, 30, r.gads[0]), "the same position again stays GADGETUP");
    let d = r.intent(0, Intent::ScrollTo(90)).unwrap();
    assert_eq!(r.msg(&d).1, 90, "total - visible is the last top");
    // Negative controls: past the end is refused by name and the top stays.
    let e = r.intent(0, Intent::ScrollTo(91)).unwrap_err();
    assert!(e.contains("SCROLLER_KIND") && e.contains("top 91 past 90"), "{e}");
    assert_eq!(r.get(0, GTSC_TOP), 90);
    assert!(r.intent(0, Intent::SelectRow(1)).unwrap_err().contains("means nothing"));
    assert_eq!((r.get(0, GTSC_TOTAL), r.get(0, GTSC_VISIBLE)), (100, 10));
}

#[test]
fn guest_scroller_changes_are_pushed_and_clamped_like_a_prop_gadget() {
    let mut m = M::new();
    let t = m.tags(&[(GTSC_TOTAL, 20), (GTSC_VISIBLE, 5), (PGA_FREEDOM, LORIENT_VERT)]);
    let top = m.tags(&[(GTSC_TOP, 99)]);
    let total = m.tags(&[(GTSC_TOTAL, 40), (GTSC_TOP, 12)]);
    let mut r = Rig::open(m, &[(SCROLLER, t)]);
    r.set(0, top);
    assert_eq!(r.model(0).kind, ControlKind::ScrollBar { vertical: true, total: 20, visible: 5, top: 15 }, "Top is clamped to total - visible");
    r.set(0, total);
    assert_eq!(r.model(0).kind, ControlKind::ScrollBar { vertical: true, total: 40, visible: 5, top: 12 });
    assert_eq!(r.get(0, GTSC_TOP), 12);
}

#[test]
fn a_number_gadget_is_a_read_only_label_formatted_by_rawdofmt() {
    let mut m = M::new();
    let dflt = m.tags(&[(GTNM_NUMBER, (-42i32) as u32), (GTNM_BORDER, 1)]);
    let f = m.s("n=%5ld|");
    let padded = m.tags(&[(GTNM_NUMBER, 7), (GTNM_FORMAT, f)]);
    let hex_f = m.s("%lx");
    let hex = m.tags(&[(GTNM_NUMBER, 255), (GTNM_FORMAT, hex_f)]);
    let pct_f = m.s("%ld%%");
    let pct = m.tags(&[(GTNM_NUMBER, 5), (GTNM_FORMAT, pct_f)]);
    let set7 = m.tags(&[(GTNM_NUMBER, 123)]);
    let mut r = Rig::open(m, &[(NUMBER, dflt), (NUMBER, padded), (NUMBER, hex), (NUMBER, pct)]);
    assert_eq!(r.model(0).kind, ControlKind::Label { text: "-42".into(), framed: true }, "GTNM_Format defaults to %ld; GTNM_Border frames it");
    assert_eq!(r.model(1).kind, ControlKind::Label { text: "n=    7|".into(), framed: false });
    assert_eq!(r.model(2).kind, ControlKind::Label { text: "FF".into(), framed: false }, "RawDoFmt's %lx is upper case");
    assert_eq!(r.model(3).kind, ControlKind::Label { text: "5%".into(), framed: false });
    r.set(1, set7);
    assert_eq!(r.model(1).kind, ControlKind::Label { text: "n=  123|".into(), framed: false });
    assert!(matches!(r.m.a.ui.calls.last(), Some(UiCall::SetControl(_, _, ControlValue::Text(t))) if t == "n=  123|"));
    assert_eq!(r.get(1, GTNM_NUMBER), 123);
    // Negative control: a read-only display takes no intent.
    for i in [Intent::SetValue(ControlValue::Text("9".into())), Intent::Activate, Intent::CommitText("9".into())] {
        assert!(r.intent(0, i).unwrap_err().contains("means nothing for the control"));
    }
    assert_eq!(r.model(0).kind, ControlKind::Label { text: "-42".into(), framed: true });
}

#[test]
fn a_number_format_that_would_read_more_than_the_number_is_refused_by_name_and_makes_no_gadget() {
    for (fmt, why) in [("%s", "needs an argument a number does not supply"), ("%b", "needs an argument"), ("%c", "needs an argument"),
                       ("%ld %ld", "more than one directive"), ("%999ld", "a width beyond 256"), ("%", "needs an argument"), ("%n", "needs an argument")] {
        let mut m = M::new();
        let f = m.s(fmt);
        let t = m.tags(&[(GTNM_NUMBER, 1), (GTNM_FORMAT, f)]);
        let ng = new_gadget(&mut m, None, 1);
        let glist = DATA - 8;
        m.run(&[gt(LVO_CREATE_CONTEXT, &[a(0, glist)]), gt(LVO_CREATE_GADGET, &[d(0, NUMBER), a_res(0, 0), a(1, ng), a(2, t)])]);
        assert_eq!(m.result(1), 0, "{fmt:?}: no gadget");
        assert!(m.a.refused.iter().any(|x| x.contains("GTNM_Format") && x.contains(why)), "{fmt:?}: {:?}", m.a.refused);
        assert_eq!(m.a.heap.live_blocks(), 1, "{fmt:?}: nothing leaked but the context");
    }
    // A later GT_SetGadgetAttrsA with a hostile format is refused too, and the label keeps its text.
    let mut m = M::new();
    let t = m.tags(&[(GTNM_NUMBER, 3)]);
    let bad = m.s("%s");
    let set = m.tags(&[(GTNM_FORMAT, bad)]);
    let mut r = Rig::open(m, &[(NUMBER, t)]);
    r.set(0, set);
    assert!(r.m.a.refused.iter().any(|x| x.contains("GTNM_Format")), "{:?}", r.m.a.refused);
    assert_eq!(r.model(0).kind, ControlKind::Label { text: "3".into(), framed: false });
}

#[test]
fn palette_is_a_swatches_control_now() {
    // It was a named refusal while guestui-abi had no swatch role; tests/gt_palette.rs holds its behaviour.
    let mut m = M::new();
    let ng = new_gadget(&mut m, None, 1);
    let glist = DATA - 8;
    m.run(&[gt(LVO_CREATE_CONTEXT, &[a(0, glist)]), gt(LVO_CREATE_GADGET, &[d(0, PALETTE), a_res(0, 0), a(1, ng), a(2, 0)])]);
    assert_ne!(m.result(1), 0, "{:?}", m.a.refused);
    assert!(!m.a.refused.iter().any(|x| x.contains("PALETTE_KIND")), "{:?}", m.a.refused);
}
