//! Slice 4: the list box. The model the guest fills (a `CDesC16ArrayFlat` behind a `CTextListBoxModel`) reaches the host as one List
//! control (synthetic fixture, no corpus), the engine-built vtables are guarded against calls past their end, and FileBrowser's own
//! list (the owner's corpus; skipped when absent) shows the host what its model holds.
mod support;
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::control::{CTRL_I_POSITION, CTRL_I_SIZE};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::{loader, Process};
use heirloom_ui_abi::{CallbackToken, ControlKind, ControlValue, GuestRect, RecordingGuestUi, UiCall, WindowModel, WindowRole};

const LB_CTOR: &str = "_ZN29CAknDoubleGraphicStyleListBoxC1Ev";
const CONSTRUCT_L: &str = "_ZN24CEikFormattedCellListBox10ConstructLEPK11CCoeControli";
const MODEL: &str = "_ZNK15CEikTextListBox5ModelEv";
const SET_ARRAY: &str = "_ZN17CTextListBoxModel16SetItemTextArrayEP12MDesC16Array";
const ARR_CTOR: &str = "_ZN16CDesC16ArrayFlatC1Ei";
const APPEND: &str = "_ZN12CDesC16Array7AppendLERK7TDesC16";
const ITEM_ADDED: &str = "_ZN11CEikListBox19HandleItemAdditionLEv";
const OFFER_KEY: &str = "_ZN11CEikListBox14OfferKeyEventLERK9TKeyEvent10TEventCode";
const ALLOC: &str = "_ZN4User6AllocZEi";
const MDESC_COUNT: &str = "_ZThn24_NK12CDesC16Array9MdcaCountEv";
const MDESC_POINT: &str = "_ZThn24_NK12CDesC16Array9MdcaPointEi";

fn proc() -> Process {
    loader::bare(&[LB_CTOR, CONSTRUCT_L, MODEL, SET_ARRAY, ARR_CTOR, APPEND, ITEM_ADDED, ALLOC, MDESC_COUNT, MDESC_POINT, OFFER_KEY, "_ZN4User4FreeEPv"]).unwrap()
}

fn call(p: &mut Process, name: &str, args: &[u32]) -> (NativeOutcome, u32) {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} is not a stub"));
    for (i, a) in args.iter().enumerate() { p.cpu.regs[i] = *a; }
    let o = natives::call(p, idx);
    (o, p.cpu.regs[0])
}

fn ok(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    match call(p, name, args) { (NativeOutcome::Return, r) => r, (o, _) => panic!("{name}: {o:?}") }
}

/// A `TPtrC16` over `text` in guest memory (word 0 = length | type 1 << 28, word 1 = the data).
fn ptrc16(p: &mut Process, text: &str) -> u32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    let data = ok(p, ALLOC, &[units.len() as u32 * 2 + 2]);
    for (i, u) in units.iter().enumerate() { p.mem.write_u16(data + i as u32 * 2, *u).unwrap(); }
    let d = ok(p, ALLOC, &[8]);
    p.mem.write_u32(d, units.len() as u32 | 1 << 28).unwrap();
    p.mem.write_u32(d + 4, data).unwrap();
    d
}

/// A list box built the way FileBrowser builds it, with the application window open and the rectangle `SetRect` leaves in the control.
fn list_box(p: &mut Process) -> u32 {
    p.set_drive(support::drive_with_rom());
    p.set_ui(RecordingGuestUi::new());
    let model = WindowModel { title: "t".into(), rect: GuestRect { left: 0, top: 0, width: 360, height: 640 }, role: WindowRole::Document, closable: true, resizable: false, token: CallbackToken(1) };
    p.host.window = Some(p.host.ui.as_mut().unwrap().open_window(model).unwrap());
    let lb = ok(p, ALLOC, &[0x104]);
    ok(p, LB_CTOR, &[lb]);
    ok(p, CONSTRUCT_L, &[lb, 0, 0x4081]);
    for (o, v) in [(CTRL_I_POSITION, 0), (CTRL_I_POSITION + 4, 0), (CTRL_I_SIZE, 360), (CTRL_I_SIZE + 4, 554)] { p.mem.write_u32(lb + o, v).unwrap(); }
    lb
}

fn calls(p: &Process) -> Vec<UiCall> { p.ui_as::<RecordingGuestUi>().unwrap().calls.clone() }

fn rows(cells: &[&[&str]]) -> Vec<Vec<String>> { cells.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect() }

#[test]
fn the_guests_item_texts_reach_the_host_as_one_list_control_and_follow_the_model() {
    let mut p = proc();
    let lb = list_box(&mut p);
    let arr = ok(&mut p, ALLOC, &[28]);
    ok(&mut p, ARR_CTOR, &[arr, 4]);
    let model = ok(&mut p, MODEL, &[lb]);
    ok(&mut p, SET_ARRAY, &[model, arr + 24]);
    // An empty model, on screen: the control exists with no rows.
    p.test_list_activate(lb).unwrap();
    let c = calls(&p);
    assert_eq!(c.len(), 2, "open + add: {c:?}");
    let UiCall::AddControl(_, m) = &c[1] else { panic!("{c:?}") };
    assert_eq!(m.rect, GuestRect { left: 0, top: 0, width: 360, height: 554 });
    assert_eq!(m.token, CallbackToken(lb as u64));
    assert!(matches!(&m.kind, ControlKind::List { rows, .. } if rows.is_empty()));
    // The guest appends items and tells the list (HandleItemAdditionL): the rows follow, UTF-16 -> UTF-8 at the boundary.
    for t in ["0\tC:\tLocal disk\t0", "0\tZ:\tROM\t0", "1\tK\u{f6}ln \u{1f600}"] {
        let d = ptrc16(&mut p, t);
        ok(&mut p, APPEND, &[arr, d]);
    }
    ok(&mut p, ITEM_ADDED, &[lb]);
    let want = rows(&[&["0", "C:", "Local disk", "0"], &["0", "Z:", "ROM", "0"], &["1", "K\u{f6}ln \u{1f600}", "", ""]]);
    let c = calls(&p);
    assert_eq!(c.len(), 3, "{c:?}");
    let UiCall::SetControl(_, _, ControlValue::Rows { rows: got, selected, top }) = &c[2] else { panic!("{c:?}") };
    assert_eq!((got, *selected, *top), (&want, Some(0), 0));
    // The same model again costs no UI call; one more item sends the whole content again.
    ok(&mut p, ITEM_ADDED, &[lb]);
    assert_eq!(calls(&p).len(), 3);
    let d = ptrc16(&mut p, "0\tD:\tRAM\t0");
    ok(&mut p, APPEND, &[arr, d]);
    ok(&mut p, ITEM_ADDED, &[lb]);
    let c = calls(&p);
    let UiCall::SetControl(_, _, ControlValue::Rows { rows: got, .. }) = &c[3] else { panic!("{c:?}") };
    assert_eq!(got.len(), 4);
    assert_eq!(got[3], ["0", "D:", "RAM", "0"]);
    // The host got exactly what the guest's model holds.
    let items = p.test_list_items(lb).unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(items[2], "1\tK\u{f6}ln \u{1f600}".encode_utf16().collect::<Vec<u16>>());
}

/// `OfferKeyEventL(TKeyEvent{iCode, iScanCode, iModifiers, iRepeats}, type)`: the response (r0), or the refusal.
fn offer(p: &mut Process, lb: u32, code: u32, scan: u32, ty: u32) -> Result<u32, String> {
    let key = ok(p, ALLOC, &[16]);
    for (i, v) in [code, scan, 0, 0].iter().enumerate() { p.mem.write_u32(key + 4 * i as u32, *v).unwrap(); }
    match call(p, OFFER_KEY, &[lb, key, ty]) { (NativeOutcome::Return, r) => Ok(r), (NativeOutcome::Refused(m), _) => Err(m), (o, _) => panic!("{o:?}") }
}

fn selected_sent(p: &Process) -> Vec<Option<u32>> {
    calls(p).iter().filter_map(|c| match c { UiCall::SetControl(_, _, ControlValue::Rows { selected, .. }) => Some(*selected), _ => None }).collect()
}

fn current(p: &Process, lb: u32) -> u32 {
    let view = p.guest_word(lb + heirloom_ekaarm_engine::listbox::LB_I_VIEW).unwrap();
    p.guest_word(view + heirloom_ekaarm_engine::listbox::VIEW_I_CURRENT).unwrap()
}

#[test]
fn arrow_keys_move_the_current_item_clamped_and_other_keys_are_refused_by_name() {
    use heirloom_ekaarm_engine::listbox::{LB_I_VIEW, VIEW_I_ITEM_HEIGHT, VIEW_I_TOP, VIEW_I_VIEW_RECT};
    const DOWN: u32 = 0xf80a;
    const UP: u32 = 0xf809;
    const EEVENT_KEY: u32 = 1;
    let mut p = proc();
    let lb = list_box(&mut p);
    let arr = ok(&mut p, ALLOC, &[28]);
    ok(&mut p, ARR_CTOR, &[arr, 4]);
    let model = ok(&mut p, MODEL, &[lb]);
    ok(&mut p, SET_ARRAY, &[model, arr + 24]);
    p.test_list_activate(lb).unwrap();
    // An empty list eats every key but Yes, and tells nobody.
    assert_eq!(offer(&mut p, lb, DOWN, 0x11, EEVENT_KEY), Ok(1));
    assert_eq!(offer(&mut p, lb, 0, 0xc4, EEVENT_KEY), Ok(0));
    for t in ["0\tA\t1\t0", "0\tB\t2\t0", "0\tC\t3\t0"] {
        let d = ptrc16(&mut p, t);
        ok(&mut p, APPEND, &[arr, d]);
    }
    ok(&mut p, ITEM_ADDED, &[lb]);
    let sent = selected_sent(&p).len();
    // Two items fit the view: 2 x 100 high rows in 200.
    let view = p.guest_word(lb + LB_I_VIEW).unwrap();
    for (o, v) in [(VIEW_I_VIEW_RECT, 0), (VIEW_I_VIEW_RECT + 4, 0), (VIEW_I_VIEW_RECT + 8, 360), (VIEW_I_VIEW_RECT + 12, 200), (VIEW_I_ITEM_HEIGHT, 100)] { p.mem.write_u32(view + o, v).unwrap(); }
    // Up on the first item stays and tells nobody.
    assert_eq!(offer(&mut p, lb, UP, 0x10, EEVENT_KEY), Ok(1));
    assert_eq!((current(&p, lb), selected_sent(&p).len()), (0, sent));
    // Down moves and tells the host once each time; the top follows only when the current item would leave the view.
    assert_eq!(offer(&mut p, lb, DOWN, 0x11, EEVENT_KEY), Ok(1));
    assert_eq!((current(&p, lb), p.guest_word(view + VIEW_I_TOP).unwrap()), (1, 0));
    assert_eq!(offer(&mut p, lb, DOWN, 0x11, EEVENT_KEY), Ok(1));
    assert_eq!((current(&p, lb), p.guest_word(view + VIEW_I_TOP).unwrap()), (2, 1));
    assert_eq!(selected_sent(&p)[sent..], [Some(1), Some(2)]);
    // Down on the last item stays; a key-down / key-up event (code 0) changes nothing.
    assert_eq!(offer(&mut p, lb, DOWN, 0x11, EEVENT_KEY), Ok(1));
    assert_eq!(offer(&mut p, lb, 0, 0x11, 3), Ok(1));
    assert_eq!((current(&p, lb), selected_sent(&p).len()), (2, sent + 2));
    // Enter is the next slice: a named refusal, not a guess.
    let e = offer(&mut p, lb, 0x0d, 0x03, EEVENT_KEY).unwrap_err();
    assert!(e.contains("key code 0xd") && e.contains("only the up and down arrows"), "{e}");
    // A list in a mode the engine does not run refuses by name too.
    p.mem.write_u32(lb + heirloom_ekaarm_engine::listbox::LB_I_FLAGS, 0x1000).unwrap();
    assert!(offer(&mut p, lb, DOWN, 0x11, EEVENT_KEY).unwrap_err().contains("loop and page-at-once"));
}

#[test]
fn a_list_that_is_not_on_screen_tells_the_host_nothing() {
    let mut p = proc();
    let lb = list_box(&mut p);
    ok(&mut p, ITEM_ADDED, &[lb]);
    assert_eq!(calls(&p).len(), 1, "only the window");
}

#[test]
fn a_guest_implemented_item_array_is_refused_by_name_not_guessed() {
    let mut p = proc();
    let lb = list_box(&mut p);
    let model = ok(&mut p, MODEL, &[lb]);
    let fake = ok(&mut p, ALLOC, &[8]); // an MDesC16Array whose vtable is guest code: here a zero vptr
    let vt = ok(&mut p, ALLOC, &[8]);
    p.mem.write_u32(fake, vt).unwrap();
    ok(&mut p, SET_ARRAY, &[model, fake]);
    p.test_list_activate(lb).expect_err("guest-implemented array");
    let e = p.test_list_activate(lb).unwrap_err();
    assert!(e.contains("the guest's own implementation"), "{e}");
}

/// Engine-built vtables sit back to back in the heap; a call past the end lands on the guard stub, which refuses by name.
#[test]
fn a_call_past_the_end_of_an_engine_vtable_refuses_by_name() {
    let mut p = proc();
    let first = p.build_vtable("Alpha", &[Some("_ZN5Alpha1aEv"), Some("_ZN5Alpha1bEv")]).unwrap();
    let second = p.build_vtable("Beta", &[Some("_ZN4Beta1aEv")]).unwrap();
    assert!(second > first);
    let guard = p.mem.read_u32(first + 2 * 4).unwrap();
    assert_ne!(guard, p.mem.read_u32(second).unwrap(), "slot 2 of Alpha is its guard, not Beta's first slot");
    let idx = ((guard - heirloom_ekaarm_engine::layout::STUB_BASE) / 8) as usize;
    match natives::call(&mut p, idx) {
        NativeOutcome::Refused(m) => assert!(m.contains("vtable overrun Alpha slot 2"), "{m}"),
        o => panic!("{o:?}"),
    }
    // The list box's own table: 44 CCoeControl slots + 40 of its own, so slot 84 is past the end.
    let lb = list_box(&mut p);
    let vptr = p.mem.read_u32(lb).unwrap();
    let guard = p.mem.read_u32(vptr + 84 * 4).unwrap();
    let name = p.stubs[((guard - heirloom_ekaarm_engine::layout::STUB_BASE) / 8) as usize].name.clone().unwrap();
    assert_eq!(name, "vtable overrun CAknDoubleGraphicStyleListBox slot 84");
    // A slot nobody has confirmed refuses as <class>::vslotN.
    let unconfirmed = p.mem.read_u32(vptr + 70 * 4).unwrap();
    let name = p.stubs[((unconfirmed - heirloom_ekaarm_engine::layout::STUB_BASE) / 8) as usize].name.clone().unwrap();
    assert_eq!(name, "CAknDoubleGraphicStyleListBox::vslot70");
}

// ---- the owner's corpus ----

const TREE: &str = "/data/refs/heirloom/symbian-uiresources/layouts/aknlayout2";
const RSG: &str = "/data/build/corpus-symbian/sdk/sdk/CPP_API/epoc32/include/avkon.rsg";
const UDEB: &str = "/data/build/corpus-symbian/sdk/sdk/Emulator/epoc32/release/winscw/udeb";
const EXE: &str = "/data/build/corpus-symbian/e32/FileBrowser.exe";
const DRIVE: &str = "/data/build/corpus-symbian/drive/nokiafilebrowser";

/// FileBrowser's list reaches the host: its control is added with the list's rectangle, and the rows the host holds are the strings
/// its model holds: FileBrowser's own drive entries, built from the drive the engine answers (C: and Z:).
#[test]
fn file_browser_list_shows_the_host_what_its_model_holds() {
    use heirloom_ekaarm_engine::hostdir::{load_dir, DirRom};
    use heirloom_ekaarm_engine::{cdlextract, Outcome};
    use heirloom_fmt_e32::OrdinalTable;
    use std::path::Path;
    const ORD_TSV: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
    let qhd = format!("{UDEB}/10283389.dll");
    let rom = format!("{UDEB}/z");
    if ![TREE, DRIVE, &rom].iter().all(|d| Path::new(d).is_dir()) || ![RSG, EXE, &qhd].iter().all(|f| Path::new(f).is_file()) { println!("SKIP: corpus absent"); return; }
    let text = cdlextract::extract(Path::new(TREE), "Qhd_touch_akn_app", Some(Path::new(RSG)), Some(Path::new(&qhd))).expect("extract").0;
    let mut p = loader::load(&std::fs::read(EXE).unwrap(), &OrdinalTable::parse(ORD_TSV).unwrap()).unwrap();
    let (mut drive, _) = load_dir(Path::new(DRIVE)).unwrap();
    drive.set_rom(Box::new(DirRom::new(Path::new(&rom)).unwrap()));
    p.set_drive(drive);
    p.set_ui(RecordingGuestUi::new());
    p.set_cdl_data(text.as_bytes()).unwrap();
    match p.run(50_000_000) { Outcome::Blocked => {}, o => panic!("{o:?}") }
    let c = calls(&p);
    let (lb, model) = c.iter().find_map(|c| match c { UiCall::AddControl(_, m) if matches!(m.kind, ControlKind::List { .. }) => Some((m.token.0 as u32, m.clone())), _ => None }).expect("the list control was added");
    assert_eq!(model.rect, GuestRect { left: 0, top: 0, width: 360, height: 554 }, "the list fills the main pane");
    // What the host last held for the control: the add, then each SetControl(Rows).
    let mut held = match &model.kind { ControlKind::List { rows, .. } => rows.clone(), _ => unreachable!() };
    for call in &c { if let UiCall::SetControl(_, _, ControlValue::Rows { rows, .. }) = call { held = rows.clone(); } }
    let guest: Vec<Vec<String>> = p.test_list_items(lb).unwrap().iter().map(|i| i.split(|u| *u == 9).map(|c| String::from_utf16_lossy(c)).collect()).collect();
    let guest: Vec<Vec<String>> = guest.into_iter().map(|mut r| { r.resize(4, String::new()); r }).collect();
    assert_eq!(held, guest, "the host holds what the guest's model holds");
    assert!(!held.is_empty(), "the model holds FileBrowser's drive entries, so the host shows rows");
    let drives: Vec<&str> = held.iter().map(|r| r[1].as_str()).collect();
    assert!(drives.iter().any(|d| d.starts_with("C:")) && drives.iter().any(|d| d.starts_with("Z:")), "one row per drive the engine answers to: {drives:?}");
}
