//! GadTools breadth, driven by a real m68k program (tests/fixtures/gtmenu.s): CreateMenusA / LayoutMenusA / FreeMenus,
//! checkbox / string / slider gadgets, and native events coming back as IDCMP_MENUPICK / IDCMP_GADGETUP.
use amiga68k_engine::{Amiga, Delivered, Step};
use guestui_abi::*;
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("fixtures/gtmenu.bin");
const REL: &str = include_str!("fixtures/gtmenu.rel");
const QUIT: u32 = 0x5155_4954;
const IDCMP_GADGETUP: u32 = 0x40;
const IDCMP_MENUPICK: u32 = 0x100;

fn hunk() -> Vec<u8> {
    let relocs = REL.lines().map(|l| (0u32, l.parse::<u32>().unwrap())).collect();
    build(&[Hunk::code(BIN.to_vec(), relocs)], None)
}

/// A started machine, run until the program blocks in WaitPort.
struct Fx { a: Amiga<RecordingGuestUi>, base: u32 }
impl Fx {
    fn new() -> Fx {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        a.stop_on_unimplemented = true;
        let loaded = a.start_hunk(&hunk()).unwrap();
        let mut fx = Fx { a, base: loaded.bases[0] };
        assert_eq!(fx.run().0, Step::Blocked, "the program reaches its event loop: {:?}", fx.a.refused);
        fx
    }
    fn run(&mut self) -> (Step, u64) { let (s, st) = self.a.step(2_000_000); (s, st.insns) }
    fn u32_at(&self, off: u32) -> u32 { self.a.mem.read_u32(self.base + off).unwrap() }
    /// The program's result block: (event count, records {class, code, iaddress/item, userdata}).
    fn records(&self) -> (u32, Vec<[u32; 4]>) {
        let n = self.u32_at(4);
        (n, (0..n).map(|i| [0, 4, 8, 12].map(|o| self.u32_at(8 + i * 16 + o))).collect())
    }
    fn win(&self) -> (u32, UiObjectRef) { self.a.windows.iter().next().map(|(&w, &n)| (w, n)).expect("window open") }
    /// The nth native control the program added (0 Go, 1 Check, 2 string, 3 slider): its ref (the recording UI numbers
    /// objects in creation order, the window being 1) and its token (the guest Gadget address).
    fn control(&self, n: usize) -> (UiObjectRef, CallbackToken) {
        let ctl: Vec<_> = self.a.ui.calls.iter().filter_map(|c| match c { UiCall::AddControl(_, m) => Some(m.token), _ => None }).collect();
        (UiObjectRef { id: 2 + n as u32, generation: 1 }, ctl[n])
    }
    /// A word of the program's result block (hunk base + 4).
    fn res(&self, off: u32) -> u32 { self.u32_at(4 + off) }
    fn deliver_and_run(&mut self, ev: GuestUiEvent) -> Delivered {
        let d = self.a.deliver(ev).unwrap();
        assert_eq!(self.run().0, Step::Blocked, "{:?}", self.a.refused);
        d
    }
    /// Finish the program: a native close, then run to its exit.
    fn close(&mut self) {
        let (w, n) = self.win();
        self.a.deliver(GuestUiEvent::CloseRequested { win: n, token: CallbackToken(w as u64) }).unwrap();
        assert_eq!(self.run().0, Step::Exited, "{:?}", self.a.refused);
    }
}

#[test]
fn the_program_builds_its_gadgets_and_menus_and_the_native_side_sees_them() {
    let fx = Fx::new();
    let calls = &fx.a.ui.calls;
    let kinds: Vec<_> = calls.iter().filter_map(|c| match c { UiCall::AddControl(_, m) => Some(m.kind.clone()), _ => None }).collect();
    assert_eq!(kinds, vec![
        ControlKind::Button,
        ControlKind::Checkbox { checked: true },
        ControlKind::String { text: "abc".into(), max_len: 8 },
        ControlKind::Slider { min: 1, max: 9, value: 3 },
    ]);
    assert!(calls.contains(&UiCall::SetWindowTitle(UiObjectRef { id: 1, generation: 1 }, "Renamed".into())));
    let strips: Vec<&MenuModel> = calls.iter().filter_map(|c| match c { UiCall::SetMenuStrip(_, m) => Some(m), _ => None }).collect();
    assert_eq!(strips.len(), 2, "SetMenuStrip, then OffMenu re-sent it");
    let project = &strips[0].menus[0];
    assert_eq!(project.title, "Project");
    let labels: Vec<_> = project.items.iter().map(|i| (i.label.as_str(), i.shortcut, i.enabled)).collect();
    assert_eq!(labels, [("Open...", Some('O'), true), ("Save", Some('S'), true), ("-", None, false), ("Quit", Some('Q'), true)]);
    let options = &strips[0].menus[1];
    assert_eq!(options.items.iter().map(|i| i.check).collect::<Vec<_>>(), [Some(true), Some(true), Some(false), None]);
    assert_eq!(options.items[3].sub_items[0].label, "Sub One");
    assert!(!strips[1].menus[0].items[1].enabled, "OffMenu(Save) is on the native menu");
    assert!(strips[1].menus[0].items[0].enabled);
    assert!(fx.a.refused.is_empty(), "{:?}", fx.a.refused);
}

#[test]
fn layout_gave_the_menus_guest_visible_geometry() {
    let fx = Fx::new();
    let strip = *fx.a.menu_strips.values().next().unwrap();
    let r16 = |a: u32| fx.a.mem.read_u16(a).unwrap();
    // "Project" is 7 chars * 8 = 56 px + MenuHBorder 4*2; the next title starts after BarHBorder 5*2.
    assert_eq!((r16(strip + 4), r16(strip + 8), r16(strip + 10)), (0, 64, 8));
    let options = fx.a.mem.read_u32(strip).unwrap();
    assert_eq!(r16(options + 4), 64 + 10);
    let (open, save) = { let o = fx.a.mem.read_u32(strip + 18).unwrap(); (o, fx.a.mem.read_u32(o).unwrap()) };
    assert_eq!((r16(open + 6), r16(save + 6)), (0, 10), "items stack by their height (font 8 + 1 + 1)");
    assert_eq!(r16(open + 8), r16(save + 8), "every item in a menu is as wide as the widest");
    let (bar, quit) = { let b = fx.a.mem.read_u32(save).unwrap(); (b, fx.a.mem.read_u32(b).unwrap()) };
    assert_eq!((r16(bar + 6), r16(bar + 10), r16(quit + 6)), (20, 7, 27), "a separator is 7 px high");
}

#[test]
fn a_menu_command_reaches_the_program_as_menupick_and_a_quit_item_ends_it() {
    let mut fx = Fx::new();
    let (w, native) = fx.win();
    // NEGATIVE CONTROL: the disabled Save item is refused by name and nothing is queued.
    let save = menu_token(0, 1, None);
    let e = fx.a.deliver(GuestUiEvent::MenuCommand { win: native, token: save }).unwrap_err();
    assert!(e.0.contains("is disabled"), "{e:?}");
    assert_eq!(fx.run().0, Step::Blocked);
    assert_eq!(fx.records().0, 0, "the program saw nothing");
    // The Open item: queued, the program records it with its GTMENUITEM_USERDATA.
    let open = menu_token(0, 0, None);
    assert!(matches!(fx.deliver_and_run(GuestUiEvent::MenuCommand { win: native, token: open }), Delivered::Queued(_)));
    let (n, recs) = fx.records();
    assert_eq!(n, 1);
    assert_eq!((recs[0][0], recs[0][1] as u16, recs[0][3]), (IDCMP_MENUPICK, 0xF800 | 0, 1), "Code = FULLMENUNUM(0,0,NOSUB), userdata 1");
    assert_ne!(recs[0][2], 0, "ItemAddress resolved the number to the MenuItem");
    // Quit ends the program, which then frees everything it made.
    fx.a.deliver(GuestUiEvent::MenuCommand { win: native, token: menu_token(0, 3, None) }).unwrap();
    assert_eq!(fx.run().0, Step::Exited, "{:?}", fx.a.refused);
    let (n, recs) = fx.records();
    assert_eq!((n, recs[1][3]), (2, QUIT));
    assert_eq!(recs[1][1], 0x60 | 0xF800, "FULLMENUNUM(0,3,NOSUB)");
    assert_eq!(fx.res(148), 0xC0DE, "the program reached its cleanup");
    assert!(fx.a.refused.is_empty(), "{:?}", fx.a.refused);
    assert_eq!(fx.a.heap.live_blocks(), 0, "FreeMenus / FreeGadgets / CloseWindow gave every block back");
    assert!(fx.a.menu_allocs.is_empty() && fx.a.gadgets.is_empty() && fx.a.windows.is_empty());
    let _ = w;
}

fn menu_token(m: u64, i: u64, s: Option<u64>) -> CallbackToken { CallbackToken(m | (i << 5) | (s.unwrap_or(0x1F) << 11)) }

#[test]
fn check_items_toggle_and_exclude_each_other_and_the_native_menu_follows() {
    let mut fx = Fx::new();
    let (_, native) = fx.win();
    let checks = |fx: &Fx| -> Vec<Option<bool>> {
        let UiCall::SetMenuStrip(_, m) = fx.a.ui.calls.iter().rev().find(|c| matches!(c, UiCall::SetMenuStrip(..))).unwrap() else { unreachable!() };
        m.menus[1].items.iter().map(|i| i.check).collect()
    };
    assert_eq!(checks(&fx), [Some(true), Some(true), Some(false), None]);
    fx.deliver_and_run(GuestUiEvent::MenuCommand { win: native, token: menu_token(1, 0, None) }); // Verbose: MENUTOGGLE
    assert_eq!(checks(&fx), [Some(false), Some(true), Some(false), None], "a toggle item unchecks");
    fx.deliver_and_run(GuestUiEvent::MenuCommand { win: native, token: menu_token(1, 2, None) }); // Mode B excludes Mode A
    assert_eq!(checks(&fx), [Some(false), Some(false), Some(true), None], "mutual exclusion moved the check");
    let rec = fx.records().1;
    assert_eq!(rec.iter().map(|r| r[3]).collect::<Vec<_>>(), [3, 5], "GTMENUITEM_USERDATA per item");
    // A sub item resolves too.
    fx.deliver_and_run(GuestUiEvent::MenuCommand { win: native, token: menu_token(1, 3, Some(0)) });
    let rec = fx.records().1;
    assert_eq!((rec[2][1], rec[2][3]), ((1 | 3 << 5) as u32, 7));
    fx.close();
    assert_eq!(fx.a.heap.live_blocks(), 0);
}

#[test]
fn menu_commands_that_name_nothing_are_refused_by_name() {
    let mut fx = Fx::new();
    let (_, native) = fx.win();
    for (token, why) in [
        (CallbackToken(0xFFFF), "not a menu number"),
        (CallbackToken(1 << 40), "not a menu number"),
        (menu_token(0, 9, None), "names nothing"),
        (menu_token(5, 0, None), "names nothing"),
        (menu_token(0, 0x3F, None), "menu title, not an item"),
    ] {
        let e = fx.a.deliver(GuestUiEvent::MenuCommand { win: native, token }).unwrap_err();
        assert!(e.0.contains(why), "{token:?}: {e:?}");
    }
    assert_eq!(fx.a.engine_msgs.len(), 0, "no message was left allocated");
    fx.close();
}

#[test]
fn gadget_events_become_gadgetup_with_the_gadget_and_update_what_the_program_reads_back() {
    let mut fx = Fx::new();
    let (w, native) = fx.win();
    let idcmp = fx.a.mem.read_u32(w + 82).unwrap();
    fx.a.mem.write_u32(w + 82, idcmp | 0x10).unwrap(); // + IDCMP_MOUSEMOVE: a changed slider level arrives as one
    let (button, bt) = fx.control(0);
    let (check, ct) = fx.control(1);
    let (string, st) = fx.control(2);
    let (slider, lt) = fx.control(3);
    let ev = |control, token, value: ControlValue| GuestUiEvent::intent(native, control, token, match value {
        ControlValue::Pressed => Intent::Activate,
        ControlValue::Text(t) => Intent::CommitText(t),
        v => Intent::SetValue(v),
    });
    // Button -> GADGETUP, IAddress = the guest Gadget, Code 0.
    assert!(matches!(fx.deliver_and_run(ev(button, bt, ControlValue::Pressed)), Delivered::Queued(_)));
    let r = fx.records().1;
    assert_eq!((r[0][0], r[0][1], r[0][2]), (IDCMP_GADGETUP, 0, bt.0 as u32));
    // Checkbox -> Code = checked; Gadget.Flags SELECTED follows.
    fx.deliver_and_run(ev(check, ct, ControlValue::Checked(false)));
    assert_eq!(fx.records().1[1][1..3], [0, ct.0 as u32]);
    assert_eq!(fx.a.mem.read_u16(ct.0 as u32 + 12).unwrap() & 0x80, 0, "SELECTED cleared");
    fx.deliver_and_run(ev(check, ct, ControlValue::Checked(true)));
    assert_eq!(fx.records().1[2][1], 1);
    assert_ne!(fx.a.mem.read_u16(ct.0 as u32 + 12).unwrap() & 0x80, 0, "SELECTED set again");
    // String: the buffer the program's StringInfo points at holds the committed text (cut to MaxChars 8).
    fx.deliver_and_run(ev(string, st, ControlValue::Text("hello world".into())));
    assert_eq!(fx.records().1[3][1], 0x0D, "strgclass: GADGETUP's Code is the key that ended the edit (Return)");
    let si = fx.a.mem.read_u32(st.0 as u32 + 34).unwrap();
    let buf = fx.a.mem.read_u32(si).unwrap();
    assert_eq!(fx.a.mem.read_cstr(buf).unwrap(), "hello wo");
    assert_eq!((fx.a.mem.read_u16(si + 16).unwrap(), fx.a.mem.read_u16(si + 10).unwrap()), (8, 9), "NumChars; MaxChars counts the NUL");
    // Slider (GT_FilterIMsg): a release that changed the level is IDCMP_MOUSEMOVE with Code = the level; one that did not stays GADGETUP.
    fx.deliver_and_run(ev(slider, lt, ControlValue::Level(7)));
    assert_eq!((fx.records().1[4][0], fx.records().1[4][1]), (0x10, 7));
    fx.deliver_and_run(ev(slider, lt, ControlValue::Level(7)));
    assert_eq!((fx.records().1[5][0], fx.records().1[5][1]), (IDCMP_GADGETUP, 7), "NEGATIVE CONTROL: an unchanged level is a GADGETUP");
    // NEGATIVE CONTROLS: an out-of-range level, a value of the wrong kind, a stale control and an unknown token.
    let bad = |fx: &mut Fx, e| fx.a.deliver(e).unwrap_err().0;
    assert!(bad(&mut fx, ev(slider, lt, ControlValue::Level(10))).contains("outside 1..=9"));
    assert!(bad(&mut fx, ev(slider, lt, ControlValue::Pressed)).contains("means nothing for the control"));
    assert!(bad(&mut fx, ev(button, bt, ControlValue::Checked(true))).contains("means nothing for the control"));
    assert!(bad(&mut fx, ev(check, bt, ControlValue::Pressed)).contains("stale control ref"));
    assert!(bad(&mut fx, ev(button, CallbackToken(0x1234), ControlValue::Pressed)).contains("not a GadTools gadget"));
    assert_eq!(fx.records().0, 6, "none of those reached the program");
    // The program reads its gadgets back on the way out.
    fx.close();
    assert_eq!(fx.res(136), 1, "GTCB_Checked");
    assert_eq!(fx.res(140), 7, "GTSL_Level");
    assert_eq!(fx.a.mem.read_cstr(fx.res(144)).unwrap(), "hello wo", "GTST_String points at the buffer the native side filled");
    assert!(fx.a.refused.is_empty(), "{:?}", fx.a.refused);
    assert_eq!(fx.a.heap.live_blocks(), 0, "StringInfo and buffer went with the gadget");
}

#[test]
fn a_window_that_did_not_ask_for_a_class_gets_no_message_but_the_state_still_moves() {
    let mut fx = Fx::new();
    let (w, native) = fx.win();
    let (check, ct) = fx.control(1);
    fx.a.mem.write_u32(w + 82, 0x200).unwrap(); // IDCMPFlags: CLOSEWINDOW only
    let (s, st) = fx.a.step(1_000_000);
    assert_eq!((s, st.insns), (Step::Blocked, 0));
    // Menu: Options > Verbose (MENUTOGGLE, starts checked).
    let d = fx.a.deliver(GuestUiEvent::MenuCommand { win: native, token: menu_token(1, 0, None) }).unwrap();
    assert_eq!(d, Delivered::IgnoredUnsubscribed);
    let UiCall::SetMenuStrip(_, m) = fx.a.ui.calls.last().unwrap() else { panic!("the native menu was re-sent") };
    assert_eq!(m.menus[1].items[0].check, Some(false), "Intuition moves the check even when no MENUPICK is wanted");
    // Gadget: SELECTED follows the native checkbox.
    let d = fx.a.deliver(GuestUiEvent::intent(native, check, ct, Intent::SetValue(ControlValue::Checked(false)))).unwrap();
    assert_eq!(d, Delivered::IgnoredUnsubscribed);
    assert_eq!(fx.a.mem.read_u16(ct.0 as u32 + 12).unwrap() & 0x80, 0);
    assert!(fx.a.engine_msgs.is_empty(), "nothing queued, nothing allocated");
    let (s, st) = fx.a.step(1_000_000);
    assert_eq!((s, st.insns), (Step::Blocked, 0), "the program was not woken");
    // CONTROL: subscribe again and the same event is delivered.
    fx.a.mem.write_u32(w + 82, 0x340).unwrap();
    let d = fx.a.deliver(GuestUiEvent::intent(native, check, ct, Intent::SetValue(ControlValue::Checked(true)))).unwrap();
    assert!(matches!(d, Delivered::Queued(_)));
    fx.run();
    fx.close();
}

#[test]
fn events_for_a_window_that_is_gone_are_not_for_a_guest_window() {
    let mut fx = Fx::new();
    let (_, native) = fx.win();
    let (check, ct) = fx.control(1);
    fx.close();
    assert_eq!(fx.a.deliver(GuestUiEvent::MenuCommand { win: native, token: menu_token(0, 0, None) }).unwrap(), Delivered::NotForAGuestWindow);
    let ev = GuestUiEvent::intent(native, check, ct, Intent::Activate);
    assert_eq!(fx.a.deliver(ev).unwrap(), Delivered::NotForAGuestWindow);
    let e = fx.a.deliver(GuestUiEvent::Resized { win: native, rect: GuestRect { left: 0, top: 0, width: 1, height: 1 } }).unwrap_err();
    assert!(e.0.contains("not bridged yet"), "{e:?}");
}

/// Seam v2 + the GadTools table (src/translate/gadtools.rs): the guest-visible state changes FIRST, then exactly one IDCMP_GADGETUP
/// follows with IAddress = the Gadget and the Code the table names; an intent the table has no row for, or a stale generation, is
/// refused by name and changes nothing.
#[test]
fn state_first_then_one_gadgetup_in_the_table_order_and_stale_or_rowless_intents_are_refused() {
    let mut fx = Fx::new();
    let (w, native) = fx.win();
    let (button, bt) = fx.control(0);
    let (check, ct) = fx.control(1);
    let (slider, lt) = fx.control(3);
    let idcmp = fx.a.mem.read_u32(w + 82).unwrap();
    fx.a.mem.write_u32(w + 82, idcmp | 0x10).unwrap();
    // State first: the checkbox's SELECTED flag is set the moment the event is delivered, before the program runs and sees the message.
    assert_eq!(fx.a.mem.read_u16(ct.0 as u32 + 12).unwrap() & 0x80 != 0, true, "starts checked (GTCB_Checked)");
    fx.a.mem.write_u16(w + 14, 21).unwrap(); // Window.MouseX / MouseY: every IntuiMessage is stamped with them
    fx.a.mem.write_u16(w + 12, 9).unwrap();
    let d = fx.a.deliver(GuestUiEvent::intent(native, check, ct, Intent::SetValue(ControlValue::Checked(false)))).unwrap();
    assert_eq!(fx.a.mem.read_u16(ct.0 as u32 + 12).unwrap() & 0x80, 0, "SELECTED cleared before the guest ran");
    let Delivered::Queued(msg) = d else { panic!("{d:?}") };
    let (class, code, iaddress) = (fx.a.mem.read_u32(msg + 20).unwrap(), fx.a.mem.read_u16(msg + 24).unwrap(), fx.a.mem.read_u32(msg + 28).unwrap());
    assert_eq!((class, code, iaddress), (IDCMP_GADGETUP, 0, ct.0 as u32), "IntuiMessage Class / Code (the new state) / IAddress (the gadget)");
    assert_eq!((fx.a.mem.read_u16(msg + 32).unwrap(), fx.a.mem.read_u16(msg + 34).unwrap()), (21, 9), "MouseX / MouseY are the window's");
    assert_eq!(fx.run().0, Step::Blocked);
    // Several intents, one message each, in delivery order (the program logs them).
    fx.deliver_and_run(GuestUiEvent::intent(native, button, bt, Intent::Activate));
    fx.deliver_and_run(GuestUiEvent::intent(native, slider, lt, Intent::SetValue(ControlValue::Level(4))));
    let r = fx.records().1;
    assert_eq!(r.iter().map(|x| (x[0], x[1], x[2])).collect::<Vec<_>>(), [(IDCMP_GADGETUP, 0, ct.0 as u32), (IDCMP_GADGETUP, 0, bt.0 as u32), (0x10, 4, lt.0 as u32)]);
    // NEGATIVE CONTROLS. A stale generation (the ref as the host held it before the control was recreated) is refused by name.
    let stale = GuestUiEvent::Intent { win: native, control: button, token: bt, generation: button.generation + 1, intent: Intent::Activate };
    let e = fx.a.deliver(stale).unwrap_err();
    assert!(e.0.contains("stale"), "{e:?}");
    // The table has no row for Press on a GadTools button (GadTools gadgets report on release only) or for a row selection.
    for intent in [Intent::Press, Intent::SelectRow(1), Intent::ScrollTo(1)] {
        let e = fx.a.deliver(GuestUiEvent::intent(native, button, bt, intent)).unwrap_err();
        assert!(e.0.contains("means nothing for the control"), "{e:?}");
    }
    assert_eq!(fx.records().0, 3, "none of those reached the program");
    fx.close();
}
