//! Appearance Manager entry points and the Control / Dialog / Window Manager Carbon accessors, by C name through the PowerPC glue, over the SAME guest records the 68k
//! Toolbox keeps: an accessor must see what a Classic program reads from the record, and a mutator must reach the record and the host presentation through the trap.
use guestui_abi::{RecordingGuestUi, UiCall};
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use mac68k_engine::pascal::Rect;
use mac68k_engine::toolbox::WindowSpec;

type M = MacPpc<RecordingGuestUi>;
const RECT: u32 = 0x3100;
const OUT: u32 = 0x3200;
const BUF: u32 = 0x3300;
const TITLE: u32 = 0x3140;

fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m.mac, 0x3F00).unwrap();
    m.mac.qd_globals = 0x3F00;
    let spec = WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 220, right: 250 }, title: "W".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: true, ref_con: 0 };
    let win = m.mac.make_window(spec).unwrap();
    for (i, v) in [10i16, 20, 34, 140].into_iter().enumerate() { m.mac.mem.write_u16(RECT + 2 * i as u32, v as u16).unwrap(); }
    m.mac.mem.write_bytes(TITLE, &[2, b'O', b'K']).unwrap();
    (m, win)
}
fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?} {:?}", m.mac.refused, m.mac.log.last());
    regs[3]
}
fn refused(m: &mut M, name: &str, args: &[u32]) -> String {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    match m.glue_call(name, &mut regs) { GlueOutcome::Refused(w) => w, o => panic!("{name}: expected a refusal, got {o:?}") }
}
fn rect_at(m: &M, at: u32) -> [i16; 4] { [0, 2, 4, 6].map(|o| m.mac.mem.read_u16(at + o).unwrap() as i16) }
fn new_button(m: &mut M, win: u32) -> u32 { let c = call(m, "NewControl", &[win, RECT, TITLE, 1, 0, 0, 1, 0, 0]); assert_ne!(c, 0, "{:?}", m.mac.refused); c }

#[test]
fn appearance_registration_succeeds_and_theme_cursors_are_the_classic_ones_or_refused() {
    let (mut m, _) = machine();
    assert_eq!(call(&mut m, "RegisterAppearanceClient", &[]), 0);
    assert_eq!(call(&mut m, "SetThemeCursor", &[0]), 0, "arrow");
    assert!(refused(&mut m, "SetThemeCursor", &[99]).contains("theme cursor 99 is not implemented"));
}

#[test]
fn control_accessors_read_the_record_and_mutators_go_through_the_trap() {
    let (mut m, win) = machine();
    let ctl = new_button(&mut m, win);
    assert_eq!(call(&mut m, "GetControlBounds", &[ctl, OUT]), OUT);
    assert_eq!(rect_at(&m, OUT), [10, 20, 34, 140]);
    assert_eq!(call(&mut m, "GetControlOwner", &[ctl]), win);
    assert_eq!(call(&mut m, "IsControlVisible", &[ctl]), 1);
    assert_eq!(call(&mut m, "SetControlVisibility", &[ctl, 0, 0]), 0);
    assert_eq!(call(&mut m, "IsControlVisible", &[ctl]), 0, "HideControl reached the record");
    call(&mut m, "SetControlVisibility", &[ctl, 1, 0]);
    // move + resize through MoveControl/SizeControl: contrlRect follows
    for (i, v) in [50i16, 60, 90, 200].into_iter().enumerate() { m.mac.mem.write_u16(BUF + 2 * i as u32, v as u16).unwrap(); }
    call(&mut m, "SetControlBounds", &[ctl, BUF]);
    call(&mut m, "GetControlBounds", &[ctl, OUT]);
    assert_eq!(rect_at(&m, OUT), [50, 60, 90, 200]);
    // deactivate dims (hilite 255), activate restores (0)
    let hilite = |m: &M| { let p = m.mac.mm.block(ctl).unwrap().ptr; m.mac.mem.read_u8(p + 17).unwrap() };
    call(&mut m, "DeactivateControl", &[ctl]);
    assert_eq!(hilite(&m), 255);
    call(&mut m, "ActivateControl", &[ctl]);
    assert_eq!(hilite(&m), 0);
    // NEGATIVE control: a pointer that is no control is refused by name
    assert!(refused(&mut m, "GetControlBounds", &[0x1234, OUT]).contains("not a control or dialog handle"));
}

#[test]
fn properties_focus_embedding_and_hit_testing() {
    let (mut m, win) = machine();
    let ctl = new_button(&mut m, win);
    let (creator, tag) = (u32::from_be_bytes(*b"Blzz"), u32::from_be_bytes(*b"prop"));
    // not found before it is set
    assert_eq!(call(&mut m, "GetControlProperty", &[ctl, creator, tag, 4, OUT, BUF]) as i32, -5604);
    m.mac.mem.write_u32(BUF, 0xCAFE_F00D).unwrap();
    assert_eq!(call(&mut m, "SetControlProperty", &[ctl, creator, tag, 4, BUF]), 0);
    m.mac.mem.write_u32(BUF, 0).unwrap();
    assert_eq!(call(&mut m, "GetControlProperty", &[ctl, creator, tag, 4, OUT, BUF]), 0);
    assert_eq!((m.mac.mem.read_u32(OUT).unwrap(), m.mac.mem.read_u32(BUF).unwrap()), (4, 0xCAFE_F00D));
    // a buffer that is too small reports the size and fails; windows keep their own properties under the same key shape
    assert_eq!(call(&mut m, "GetControlProperty", &[ctl, creator, tag, 2, OUT, BUF]) as i32, -5603);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 4);
    assert_eq!(call(&mut m, "GetWindowProperty", &[win, creator, tag, 4, OUT, BUF]) as i32, -5604, "the control's property is not the window's");
    // keyboard focus
    call(&mut m, "GetKeyboardFocus", &[win, OUT]);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0);
    call(&mut m, "SetKeyboardFocus", &[win, ctl, 1]);
    call(&mut m, "GetKeyboardFocus", &[win, OUT]);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), ctl);
    call(&mut m, "ClearKeyboardFocus", &[win]);
    call(&mut m, "GetKeyboardFocus", &[win, OUT]);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0);
    // root control: one per window
    call(&mut m, "CreateRootControl", &[win, OUT]);
    let root = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(root, 0);
    call(&mut m, "GetRootControl", &[win, OUT]);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), root);
    assert_eq!(call(&mut m, "EmbedControl", &[ctl, root]), 0);
    assert!(refused(&mut m, "CreateRootControl", &[win, OUT]).contains("already has a root control"));
    // FindControlUnderMouse: local point inside the button (v=20, h=30) finds it with a part; outside finds none
    let hit = call(&mut m, "FindControlUnderMouse", &[(20 << 16) | 30, win, OUT]);
    assert_eq!(hit, ctl);
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap(), 10, "inButton");
    assert_eq!(call(&mut m, "FindControlUnderMouse", &[(110 << 16) | 400, win, OUT]), 0);
}

#[test]
fn window_and_dialog_accessors_read_the_records() {
    let (mut m, win) = machine();
    assert_eq!(call(&mut m, "IsWindowVisible", &[win]), 1);
    assert_eq!(call(&mut m, "GetWindowFromPort", &[win]), win);
    assert_eq!(call(&mut m, "GetWindowFromPort", &[0x5555]), 0, "a port that is no window");
    assert_ne!(call(&mut m, "GetWindowList", &[]), 0);
    assert_eq!(call(&mut m, "GetWindowBounds", &[win, 33, OUT]), 0);
    assert_eq!(rect_at(&m, OUT), [100, 50, 220, 250], "the content area in global coordinates");
    assert!(refused(&mut m, "GetWindowBounds", &[win, 32, OUT]).contains("window region 32 is not implemented"));
    // InvalWindowRect leaves thePort where it was
    let slot = m.mac.qd_globals;
    m.mac.mem.write_u32(slot, 0).unwrap();
    for (i, v) in [0i16, 0, 10, 10].into_iter().enumerate() { m.mac.mem.write_u16(BUF + 2 * i as u32, v as u16).unwrap(); }
    call(&mut m, "InvalWindowRect", &[win, BUF]);
    assert_eq!(m.mac.mem.read_u32(slot).unwrap(), 0);
    // a DialogRecord: items handle at 156 (DITL count word = items - 1), editField at 164 (-1 = none)
    let dlg = 0x5000u32;
    let items = call(&mut m, "NewHandle", &[4]);
    let p = m.mac.mm.block(items).unwrap().ptr;
    m.mac.mem.write_u16(p, 2).unwrap();
    m.mac.mem.write_u32(dlg + 156, items).unwrap();
    m.mac.mem.write_u16(dlg + 164, 0xFFFF).unwrap();
    assert_eq!(call(&mut m, "CountDITL", &[dlg]), 3);
    assert_eq!(call(&mut m, "GetDialogKeyboardFocusItem", &[dlg]), 0);
    m.mac.mem.write_u16(dlg + 164, 1).unwrap();
    assert_eq!(call(&mut m, "GetDialogKeyboardFocusItem", &[dlg]), 2, "editField is 0-based, the Carbon item index 1-based");
    assert_eq!((call(&mut m, "GetDialogPort", &[dlg]), call(&mut m, "GetDialogWindow", &[dlg])), (dlg, dlg));
    assert_eq!(call(&mut m, "SetDialogDefaultItem", &[dlg, 2]), 0);
    assert_eq!(m.mac.mem.read_u16(dlg + 168).unwrap(), 2);
}

#[test]
fn testcontrol_reports_the_part_under_a_local_point_and_nothing_for_hidden_inactive_or_outside() {
    let (mut m, win) = machine();
    let ctl = new_button(&mut m, win);
    let at = |v: i16, h: i16| ((v as u16 as u32) << 16) | h as u16 as u32;
    assert_eq!(call(&mut m, "TestControl", &[ctl, at(20, 30)]), 10, "inButton");
    // NEGATIVE controls
    assert_eq!(call(&mut m, "TestControl", &[ctl, at(200, 30)]), 0, "outside the rectangle");
    call(&mut m, "DeactivateControl", &[ctl]);
    assert_eq!(call(&mut m, "TestControl", &[ctl, at(20, 30)]), 0, "an inactive control takes no hit");
    call(&mut m, "ActivateControl", &[ctl]);
    call(&mut m, "SetControlVisibility", &[ctl, 0, 0]);
    assert_eq!(call(&mut m, "TestControl", &[ctl, at(20, 30)]), 0, "a hidden control takes no hit");
}

#[test]
fn hidewindow_keeps_the_window_open_and_tells_the_host_and_showwindow_brings_it_back() {
    let (mut m, win) = machine();
    call(&mut m, "ShowWindow", &[win]);
    assert_eq!(call(&mut m, "IsWindowVisible", &[win]), 1);
    let hides = |m: &M| m.mac.ui.calls.iter().filter(|c| matches!(c, UiCall::HideWindow(_))).count();
    call(&mut m, "HideWindow", &[win]);
    assert_eq!(call(&mut m, "IsWindowVisible", &[win]), 0);
    assert!(m.mac.windows.contains_key(&win), "still open");
    assert_eq!(hides(&m), 1);
    // FrontWindow skips it; hiding again is a no-op that does not tell the host twice (NEGATIVE control)
    assert_eq!(call(&mut m, "FrontWindow", &[]), 0);
    call(&mut m, "HideWindow", &[win]);
    assert_eq!(hides(&m), 1);
    call(&mut m, "ShowWindow", &[win]);
    assert_eq!(call(&mut m, "IsWindowVisible", &[win]), 1);
    assert!(m.mac.ui.calls.iter().any(|c| matches!(c, UiCall::ShowWindow(_))));
    assert_eq!(call(&mut m, "FrontWindow", &[]), win);
    // an unknown window is refused by name
    m.bind("HideWindow");
    let mut regs = [0u32; 32];
    regs[3] = 0x1234;
    let _ = m.glue_call("HideWindow", &mut regs);
    assert!(m.mac.refused.iter().chain(m.mac.log.iter()).any(|l| l.contains("HideWindow") && l.contains("not an open window")), "{:?} {:?}", m.mac.refused, m.mac.log.last());
}

fn calli(m: &mut M, name: &str, args: &[u32]) -> i32 { call(m, name, args) as i32 }

fn dialog_machine() -> (M, u32) {
    let (mut m, _) = machine();
    m.mac.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(mac68k_engine::fixture::dialog_fork()).unwrap());
    let dlg = call(&mut m, "GetNewDialog", &[128, 0, 0xFFFF_FFFF]);
    assert_ne!(dlg, 0, "{:?}", m.mac.refused);
    (m, dlg)
}

#[test]
fn getdialogitemascontrol_follows_the_spec_vectors() {
    // spec: macos9-specs/AppearanceLib/GetDialogItemAsControl
    let (mut m, dlg) = dialog_machine();
    assert_eq!(call(&mut m, "GetDialogItemAsControl", &[dlg, 1, OUT]), 0);
    let ctl = m.mac.mem.read_u32(OUT).unwrap();
    assert_eq!(ctl, m.mac.tr.dlg.dialogs[&dlg].list[0].handle, "the item handle is the ControlRef");
    assert_eq!(call(&mut m, "GetDialogItemAsControl", &[dlg, 3, OUT]), 0, "a check box item is a control item");
    // NEGATIVE controls: a static text item in a classic dialog, the user item, item 0, -1, count+1, a NULL dialog
    m.mac.mem.write_u32(OUT, 0xDEAD).unwrap();
    assert_eq!(calli(&mut m, "GetDialogItemAsControl", &[dlg, 4, OUT]), -30596);
    assert_eq!(calli(&mut m, "GetDialogItemAsControl", &[dlg, 6, OUT]), -30596);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xDEAD, "outControl untouched on an error");
    for bad in [0u32, 0xFFFF_FFFF, 7] { assert_eq!(calli(&mut m, "GetDialogItemAsControl", &[dlg, bad, OUT]), -50, "item {bad:#x}"); }
    assert_eq!(calli(&mut m, "GetDialogItemAsControl", &[0, 1, OUT]), -50);
    // with a root control every item is a control
    call(&mut m, "CreateRootControl", &[dlg, OUT]);
    assert_eq!(call(&mut m, "GetDialogItemAsControl", &[dlg, 4, OUT]), 0);
}

#[test]
fn control_data_fontstyle_viewsize_and_theme_background() {
    let (mut m, win) = machine();
    let classic = new_button(&mut m, win);
    // an Appearance push button (kControlPushButtonProc = 368)
    let ctl = call(&mut m, "NewControl", &[win, RECT, TITLE, 1, 0, 0, 1, 368, 0]);
    assert_ne!(ctl, 0, "{:?}", m.mac.refused);
    let font = u32::from_be_bytes(*b"font");
    let mut rec = [0u8; 24];
    rec[0..2].copy_from_slice(&0x0002u16.to_be_bytes()); // flags: size
    rec[4..6].copy_from_slice(&12u16.to_be_bytes());
    m.mac.mem.write_bytes(BUF, &rec).unwrap();
    // spec vectors: NULL control -50; a control without data access -30580 (the classic push button); the Appearance one accepts
    assert_eq!(calli(&mut m, "SetControlData", &[0, 0, font, 24, BUF]), -50);
    assert_eq!(calli(&mut m, "SetControlData", &[classic, 0, font, 24, BUF]), -30580);
    assert_eq!(calli(&mut m, "SetControlFontStyle", &[classic, BUF]), -30580);
    assert_eq!(calli(&mut m, "SetControlFontStyle", &[ctl, 0]), -50);
    assert_eq!(call(&mut m, "SetControlFontStyle", &[ctl, BUF]), 0);
    assert_eq!(call(&mut m, "GetControlData", &[ctl, 0, font, 24, OUT, OUT + 0x40]), 0);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 24).unwrap()).unwrap(), &rec[..], "SetControlFontStyle = SetControlData('font')");
    assert_eq!(m.mac.mem.read_u32(OUT + 0x40).unwrap(), 24);
    // wrong size for a fixed-size tag -30591, unknown tag -30581 (errDataNotSupported)
    assert_eq!(calli(&mut m, "SetControlData", &[ctl, 0, font, 8, BUF]), -30591);
    assert_eq!(calli(&mut m, "SetControlData", &[ctl, 0, u32::from_be_bytes(*b"zzzz"), 1, BUF]), -30581);
    assert_eq!(calli(&mut m, "GetControlData", &[ctl, 0, u32::from_be_bytes(*b"dflt"), 1, OUT, 0]), -30581, "nothing stored under dflt");
    m.mac.mem.write_u8(BUF, 1).unwrap();
    assert_eq!(call(&mut m, "SetControlData", &[ctl, 0, u32::from_be_bytes(*b"dflt"), 1, BUF]), 0);
    // view size: stored; unknown control ignored
    call(&mut m, "SetControlViewSize", &[ctl, 50]);
    assert_eq!(call(&mut m, "GetControlViewSize", &[ctl]), 50);
    call(&mut m, "SetControlViewSize", &[0x1234, 5]);
    // SetThemeBackground: black and white set the port's background colour; brush 0, 49, -3 are themeInvalidBrushErr and change nothing
    let bk = |m: &mut M| { call(m, "GetBackColor", &[OUT + 0x80]); (0..3).map(|i| m.mac.mem.read_u16(OUT + 0x80 + 2 * i).unwrap()).collect::<Vec<_>>() };
    call(&mut m, "SetPort", &[win]);
    assert_eq!(call(&mut m, "SetThemeBackground", &[(-1i32) as u32, 8, 1]), 0);
    assert_eq!(bk(&mut m), vec![0, 0, 0], "kThemeBrushBlack: background colour black");
    assert_eq!(call(&mut m, "SetThemeBackground", &[(-2i32) as u32, 32, 1]), 0);
    assert_eq!(bk(&mut m), vec![0xFFFF; 3], "kThemeBrushWhite");
    for bad in [0i32, 49, -3] { assert_eq!(calli(&mut m, "SetThemeBackground", &[bad as u32, 8, 1]), -30560, "brush {bad}"); assert_eq!(bk(&mut m), vec![0xFFFF; 3], "no port change on a bad brush"); }
    assert_eq!(call(&mut m, "SetThemeBackground", &[1, 8, 1]), 0, "a theme brush is accepted (the host paints it)");
}
