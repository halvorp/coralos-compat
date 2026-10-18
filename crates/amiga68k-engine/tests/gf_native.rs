//! Gute Form spike: standard Intuition gadgets are NATIVE controls. A fixture window holds a button, a prop scrollbar, a string
//! gadget, an arrow and a classic boolean gadget; the recording host sees each as a control (kind, rect, value), the canvas has
//! no pixels where they are, and native gestures change the guest-visible state and post the IDCMP messages Intuition would.
mod gfxcommon;
use amiga68k_engine::boopsi_ids::*;
use amiga68k_engine::Delivered;
use gfxcommon::*;
use guestui_abi::*;

const ADD_GADGET: i32 = -42;
const ADD_GLIST: i32 = -438;
const REMOVE_GLIST: i32 = -444;
const REFRESH_GLIST: i32 = -432;
const OPEN_WINDOW_TAG_LIST: i32 = -606;
const NEW_OBJECT_A: i32 = -636;
const SET_GADGET_ATTRS_A: i32 = -660;
const SYSIA_WHICH: u32 = IA + 0x0D;
const PGA: u32 = TAG_USER + 0x31000;
const STRINGA: u32 = TAG_USER + 0x32000;
const WD_RPORT: u32 = 50;
const WD_USERPORT: u32 = 86;
const GADGETDOWN: u32 = 0x20;
const GADGETUP: u32 = 0x40;
const MOUSEMOVE: u32 = 0x10;

struct Fx { g: G, win: u32, btn: u32, prop: u32, strg: u32, arrow: u32, classic: u32 }

fn ctl_of<'a>(g: &'a G, gadget: u32) -> (UiObjectRef, &'a ControlModel) {
    g.a.ui.controls.iter().find(|(_, (_, m))| m.token.0 == gadget as u64).map(|(&id, (_, m))| (UiObjectRef { id, generation: 1 }, m)).unwrap_or_else(|| panic!("gadget {gadget:#x} is not a native control: {:?}", g.a.ui.controls))
}

fn fixture() -> Fx {
    let mut g = G::new();
    let (title, ok, hello) = (g.s("Fixture"), g.s("OK"), g.s("hello"));
    let (bc, pc, sc, ic) = (g.s("buttongclass"), g.s("propgclass"), g.s("strgclass"), g.s("sysiclass"));
    let win_tags = g.tags(&[(WA + 7, 0x70), (WA + 3, 320), (WA + 4, 200), (WA + 11, title)]); // IDCMP: MOUSEMOVE|GADGETDOWN|GADGETUP
    let btn_tags = g.tags(&[(GA_LEFT, 10), (GA_TOP, 30), (GA_WIDTH, 60), (GA_HEIGHT, 14), (GA_TEXT, ok), (GA_ID, 1), (GA_REL_VERIFY, 1), (GA_IMMEDIATE, 1)]);
    let prop_tags = g.tags(&[(GA_LEFT, 280), (GA_TOP, 40), (GA_WIDTH, 12), (GA_HEIGHT, 100), (GA_ID, 2), (GA_REL_VERIFY, 1), (GA_IMMEDIATE, 1), (GA_FOLLOW_MOUSE, 1),
        (PGA + 7, 100), (PGA + 8, 10), (PGA + 9, 0)]);
    let strg_tags = g.tags(&[(GA_LEFT, 10), (GA_TOP, 60), (GA_WIDTH, 120), (GA_HEIGHT, 10), (GA_ID, 3), (GA_REL_VERIFY, 1), (STRINGA + 1, 20), (STRINGA + 18, hello)]);
    let sysi_tags = g.tags(&[(SYSIA_WHICH, 0x0B)]); // UPIMAGE
    // a classic boolean gadget (struct Gadget, 44 bytes) with an IntuiText label
    let label = g.s("Cancel");
    let mut it = vec![0u8; 20];
    it[12..16].copy_from_slice(&label.to_be_bytes());
    let itext = g.bytes(&it);
    let mut gad = vec![0u8; 44];
    for (off, v) in [(4usize, 150u16), (6, 30), (8, 70), (10, 14), (14, 1), (16, 1), (38, 5)] { gad[off..off + 2].copy_from_slice(&v.to_be_bytes()); }
    gad[26..30].copy_from_slice(&itext.to_be_bytes());
    let classic = g.bytes(&gad);
    g.run(&[intu(OPEN_WINDOW_TAG_LIST, &[a(0, 0), a(1, win_tags)]), intu(NEW_OBJECT_A, &[a(0, 0), a(1, bc), a(2, btn_tags)]),
        intu(NEW_OBJECT_A, &[a(0, 0), a(1, pc), a(2, prop_tags)]), intu(NEW_OBJECT_A, &[a(0, 0), a(1, sc), a(2, strg_tags)]),
        intu(NEW_OBJECT_A, &[a(0, 0), a(1, ic), a(2, sysi_tags)])]);
    let (win, btn, prop, strg, up) = (g.result(0), g.result(1), g.result(2), g.result(3), g.result(4));
    assert!(win != 0 && btn != 0 && prop != 0 && strg != 0 && up != 0, "{:?}", g.a.refused);
    let arrow_tags = g.tags(&[(GA_LEFT, 280), (GA_TOP, 142), (GA_WIDTH, 12), (GA_HEIGHT, 12), (GA_IMAGE, up), (GA_ID, 4), (GA_REL_VERIFY, 1)]);
    g.run(&[intu(NEW_OBJECT_A, &[a(0, 0), a(1, bc), a(2, arrow_tags)])]);
    let arrow = g.result(0);
    assert_ne!(arrow, 0, "{:?}", g.a.refused);
    let all = [btn, prop, strg, arrow, classic];
    let calls: Vec<Call> = all.iter().map(|&x| intu(ADD_GADGET, &[a(0, win), a(1, x), d(0, 0xFFFF)])).collect();
    g.run(&calls);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
    Fx { g, win, btn, prop, strg, arrow, classic }
}

/// Every IntuiMessage queued on the window's UserPort so far: (class, code, IAddress).
macro_rules! drive { ($f:ident, $g:ident, $intent:expr) => {{ let gd = $f.$g; drive_impl(&mut $f, gd, $intent) }}; }

fn messages(f: &Fx) -> Vec<(u32, u32, u32)> {
    let m = &f.g.a.mem;
    let port = m.read_u32(f.win + WD_USERPORT).unwrap();
    let (mut out, mut n) = (vec![], m.read_u32(port + 20).unwrap());
    while m.read_u32(n).unwrap() != 0 { out.push((m.read_u32(n + 20).unwrap(), m.read_u16(n + 24).unwrap() as u32, m.read_u32(n + 28).unwrap())); n = m.read_u32(n).unwrap(); }
    out
}

fn drive_impl(f: &mut Fx, gadget: u32, intent: Intent) -> Result<Delivered, amiga68k_engine::Refusal> {
    let (ctl, _) = ctl_of(&f.g, gadget);
    let native = *f.g.a.windows.get(&f.win).unwrap();
    f.g.a.deliver(GuestUiEvent::intent(native, ctl, CallbackToken(gadget as u64), intent))
}

#[test]
fn each_system_gadget_is_a_native_control_with_its_real_value() {
    let f = fixture();
    let (_, b) = ctl_of(&f.g, f.btn);
    assert_eq!((&b.kind, b.label.as_str(), b.rect), (&ControlKind::Button, "OK", GuestRect { left: 10, top: 30, width: 60, height: 14 }));
    let (_, p) = ctl_of(&f.g, f.prop);
    assert_eq!((&p.kind, p.rect), (&ControlKind::ScrollBar { vertical: true, total: 100, visible: 10, top: 0 }, GuestRect { left: 280, top: 40, width: 12, height: 100 }));
    let (_, s) = ctl_of(&f.g, f.strg);
    assert_eq!((&s.kind, s.rect), (&ControlKind::Field { text: "hello".into(), max_len: 19, editable: true, integer: false }, GuestRect { left: 10, top: 60, width: 120, height: 10 }));
    let (_, ar) = ctl_of(&f.g, f.arrow);
    assert_eq!((&ar.kind, ar.rect), (&ControlKind::ArrowButton { dir: ArrowDir::Up }, GuestRect { left: 280, top: 142, width: 12, height: 12 }));
    let (_, c) = ctl_of(&f.g, f.classic);
    assert_eq!((&c.kind, c.label.as_str()), (&ControlKind::Button, "Cancel"), "a classic boolean gadget: label from its IntuiText");
    assert_eq!(f.g.a.ui.controls.len(), 5, "exactly the five gadgets, nothing else registered");
}

#[test]
fn the_canvas_has_no_pixels_where_the_controls_are() {
    let mut f = fixture();
    let rp = f.g.r32(f.win + WD_RPORT);
    // RefreshGList asks every BOOPSI gadget for GM_RENDER; before the native controls that painted the AROS look into the canvas.
    let first = f.btn;
    f.g.run(&[intu(REFRESH_GLIST, &[a(0, first), a(1, f.win), a(2, 0), d(0, 0xFFFF)])]);
    let samples: Vec<(i32, i32)> = vec![(11, 31), (40, 37), (68, 42), (281, 41), (285, 90), (290, 138), (11, 61), (60, 65), (128, 68), (281, 143), (288, 150)];
    let reads: Vec<Call> = samples.iter().map(|&(x, y)| gfx(READ_PIXEL, &[a(1, rp), di(0, x), di(1, y)])).collect();
    f.g.run(&reads);
    for (i, &(x, y)) in samples.iter().enumerate() { assert_eq!(f.g.result(i as u32), 0, "pixel ({x},{y}) under a native control was painted"); }
    // Negative control: the same reads DO see a guest-drawn rectangle, so a zero above means "nothing drawn".
    f.g.run(&[gfx(SET_APEN, &[a(1, rp), d(0, 3)]), gfx(RECT_FILL, &[a(1, rp), d(0, 10), d(1, 30), d(2, 69), d(3, 43)]), gfx(READ_PIXEL, &[a(1, rp), di(0, 40), di(1, 37)])]);
    assert_eq!(f.g.result(2), 3, "control: the read sees drawn pixels");
    assert!(f.g.a.refused.is_empty(), "{:?}", f.g.a.refused);
}

#[test]
fn a_scroll_drag_updates_the_prop_info_and_posts_the_idcmp_sequence() {
    let mut f = fixture();
    let pi = f.g.r32(f.prop + 34);
    let scroll = |top| ControlValue::Scroll { total: 100, visible: 10, top };
    assert_eq!(drive!(f, prop, Intent::Press).unwrap(), Delivered::Queued(f.g.a.mem.read_u32(f.g.r32(f.win + WD_USERPORT) + 20).unwrap()));
    drive!(f, prop, Intent::Drag(scroll(40))).unwrap();
    assert_eq!(f.g.r16(pi + 4) as u32, 40 * 0xFFFF / 90, "VertPot follows Top");
    assert_eq!(f.g.r32(pi + 32), 40, "propgclass keeps PGA_Top");
    drive!(f, prop, Intent::ScrollTo(500)).unwrap();
    assert_eq!(f.g.r16(pi + 4), 0xFFFF, "a top past the end clamps to Total - Visible");
    assert_eq!(f.g.r16(pi + 8) as u32, 10 * 0xFFFF / 100, "the body is untouched");
    assert_eq!(messages(&f), [(GADGETDOWN, 0, f.prop), (MOUSEMOVE, 0, f.prop), (GADGETUP, 0, f.prop)], "IAddress = the Gadget");
    // The engine's picture matches the host's (no redundant call back): a resync after the drag sends nothing.
    let calls = f.g.a.ui.calls.len();
    f.g.run(&[intu(SET_GADGET_ATTRS_A, &[a(0, f.prop), a(1, f.win), a(2, 0), a(3, 0)])]);
    assert_eq!(f.g.a.ui.calls.len(), calls);
    // Negative controls: a stale control and a nonsense phase are refused by name and change nothing.
    let pot = f.g.r16(pi + 4);
    assert!(drive!(f, prop, Intent::Drag(ControlValue::Text("x".into()))).is_err());
    let native = *f.g.a.windows.get(&f.win).unwrap();
    let stale = f.g.a.deliver(GuestUiEvent::intent(native, UiObjectRef { id: 999, generation: 1 }, CallbackToken(f.prop as u64), Intent::Press));
    assert!(stale.is_err());
    assert_eq!((f.g.r16(pi + 4), messages(&f).len()), (pot, 3));
}

#[test]
fn guest_changes_reach_the_host_as_values_and_rects() {
    let mut f = fixture();
    let (ctl, _) = ctl_of(&f.g, f.prop);
    let t = f.g.tags(&[(PGA + 9, 20), (GA_LEFT, 270)]);
    f.g.run(&[intu(SET_GADGET_ATTRS_A, &[a(0, f.prop), a(1, f.win), a(2, 0), a(3, t)])]);
    let m = &f.g.a.ui.controls[&ctl.id].1;
    assert_eq!(m.kind, ControlKind::ScrollBar { vertical: true, total: 100, visible: 10, top: 20 }, "SetGadgetAttrs PGA_Top");
    assert_eq!(m.rect.left, 270, "GA_Left moved the control");
    assert!(f.g.a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControlRect(..))) && f.g.a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, _, ControlValue::Scroll { top: 20, .. }))));
}

#[test]
fn string_edit_and_return_update_the_buffer_and_post_gadgetup() {
    let mut f = fixture();
    let si = f.g.r32(f.strg + 34);
    let buf = f.g.r32(si);
    let calls = f.g.a.ui.calls.len();
    drive!(f, strg, Intent::Drag(ControlValue::Text("hello wor".into()))).unwrap();
    assert_eq!(f.g.a.mem.read_cstr(buf).unwrap(), "hello wor", "typing updates the StringInfo buffer live");
    assert!(messages(&f).is_empty(), "no message until Return");
    drive!(f, strg, Intent::CommitText("hello world, far too long for twenty".into())).unwrap();
    assert_eq!(f.g.a.mem.read_cstr(buf).unwrap(), "hello world, far to", "cut to MaxChars 20, which counts the NUL (Intuition)");
    assert_eq!((f.g.r16(si + 16), f.g.r16(si + 8)), (19, 19), "NumChars and BufferPos");
    assert_eq!(messages(&f), [(GADGETUP, 0x0D, f.strg)], "Return ends the edit: GADGETUP, Code 0x0D, IAddress = the gadget");
    assert!(matches!(&f.g.a.native_gads.get(u64::from(f.strg)).unwrap().model.kind, ControlKind::Field { text, .. } if text == "hello world, far to"), "the engine's copy of what the host shows is kept in line");
    assert_eq!(f.g.a.ui.calls.len(), calls, "...without a call back to the host (it made the edit)");
}

#[test]
fn buttons_select_on_press_and_verify_on_release_inside() {
    let mut f = fixture();
    let fl = |f: &Fx, g| f.g.r16(g + 12) & 0x80;
    drive!(f, btn, Intent::Press).unwrap();
    assert_eq!(fl(&f, f.btn), 0x80, "GFLG_SELECTED while held");
    drive!(f, btn, Intent::Release { inside: true }).unwrap();
    assert_eq!(fl(&f, f.btn), 0);
    assert_eq!(messages(&f), [(GADGETDOWN, 0, f.btn), (GADGETUP, 0, f.btn)]);
    // Released outside: selection ends, no GADGETUP.
    drive!(f, btn, Intent::Press).unwrap();
    drive!(f, btn, Intent::Release { inside: false }).unwrap();
    assert_eq!((fl(&f, f.btn), messages(&f).len()), (0, 3), "only the second GADGETDOWN was added");
    // The arrow has GACT_RELVERIFY but not GACT_IMMEDIATE: no GADGETDOWN, GADGETUP on release (control for the flag check).
    assert_eq!(drive!(f, arrow, Intent::Press).unwrap(), Delivered::Handled);
    drive!(f, arrow, Intent::Release { inside: true }).unwrap();
    assert_eq!(messages(&f)[3..], [(GADGETUP, 0, f.arrow)]);
    // A button gets no Move or Scroll.
    assert!(drive!(f, btn, Intent::Drag(ControlValue::Scroll { total: 1, visible: 1, top: 0 })).is_err());
}

#[test]
fn a_removed_gadget_loses_its_control_and_pointer_input_never_reaches_native_gadgets() {
    let mut f = fixture();
    f.g.run(&[intu(REMOVE_GLIST, &[a(0, f.win), a(1, f.arrow), d(0, 1)])]);
    assert!(f.g.a.ui.controls.values().all(|(_, m)| m.token.0 != f.arrow as u64), "the arrow's control is gone");
    assert_eq!(f.g.a.ui.controls.len(), 4);
    f.g.run(&[intu(ADD_GLIST, &[a(0, f.win), a(1, f.arrow), d(0, 0xFFFF), d(1, 1), a(2, 0)])]);
    assert_eq!(f.g.a.ui.controls.len(), 5, "AddGList registers it again");
    // A raw pointer press over a native gadget is not offered to a guest dispatcher (a system class would refuse GM_HITTEST).
    let native = *f.g.a.windows.get(&f.win).unwrap();
    let d = f.g.a.deliver(GuestUiEvent::primary(native, 20, 15, PointerPhase::Down)).unwrap();
    assert_eq!(d, Delivered::IgnoredUnsubscribed);
    assert!(f.g.a.refused.is_empty(), "{:?}", f.g.a.refused);
}
