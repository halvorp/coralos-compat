//! MUI object model: refusals are named, a failed build leaks nothing, labels pair with their controls.
use super::*;
use crate::Amiga;
use alloc::vec::Vec;
use guestui_abi::{RecordingGuestUi, UiCall};

fn machine() -> Amiga<RecordingGuestUi> { Amiga::new(RecordingGuestUi::new()).unwrap() }

/// Write a TagItem list (and any string) into guest RAM; the heap blocks leak on purpose (test-only).
fn tags(a: &mut Amiga<RecordingGuestUi>, items: &[(u32, u32)]) -> u32 {
    let p = a.heap.alloc(8 * (items.len() as u32 + 1)).unwrap();
    for (i, &(t, d)) in items.iter().chain([(0, 0)].iter()).enumerate() { a.mem.write_u32(p + 8 * i as u32, t).unwrap(); a.mem.write_u32(p + 8 * i as u32 + 4, d).unwrap(); }
    p
}
fn cstr(a: &mut Amiga<RecordingGuestUi>, s: &str) -> u32 { let p = a.heap.alloc(s.len() as u32 + 1).unwrap(); a.mem.write_bytes(p, s.as_bytes()).unwrap(); p }
fn make(a: &mut Amiga<RecordingGuestUi>, class: &str, items: &[(u32, u32)]) -> Result<u32, crate::Refusal> { let t = tags(a, items); new_by_name(a, class, t) }

#[test]
fn an_unimplemented_class_and_an_unimplemented_attribute_are_named_refusals() {
    let mut a = machine();
    assert_eq!(make(&mut a, "Clock.mcc", &[]).unwrap_err().0, "MUI class \"Clock.mcc\" is not implemented");
    assert!(make(&mut a, "Numeric.mui", &[]).unwrap_err().0.contains("abstract"));
    let before = a.heap.live_blocks();
    let e = make(&mut a, "Text.mui", &[(MUIA_Text_Contents, 0), (0x8042_0001, 7)]).unwrap_err().0;
    assert_eq!(e, "MUI Text class: attribute 0x80420001 is not implemented");
    assert_eq!(a.heap.live_blocks(), before + 1, "the half-built Text was freed (only the tag list remains)");
    assert!(a.objects.map.is_empty());
    // CONTROL: the same build without the bad tag succeeds, and an attribute of another class is refused for this one by NAME.
    assert!(make(&mut a, "Text.mui", &[(MUIA_Text_Contents, 0)]).is_ok());
    let e = make(&mut a, "Text.mui", &[(MUIA_Group_Columns, 2)]).unwrap_err().0;
    assert_eq!(e, "MUI Text class: attribute MUIA_Group_Columns is not implemented");
}

#[test]
fn a_null_child_fails_the_parent_and_disposes_its_siblings() {
    let mut a = machine();
    let good = make(&mut a, "Text.mui", &[]).unwrap();
    let e = make(&mut a, "Group.mui", &[(MUIA_Group_Child, good), (MUIA_Group_Child, 0)]).unwrap_err().0;
    assert!(e.contains("MUIA_Group_Child: the child object is NULL"), "{e}");
    assert!(a.objects.map.is_empty(), "the good child went too: {:?}", a.objects.map.keys().collect::<Vec<_>>());
    // A kind that does not belong is refused by name too.
    let t = make(&mut a, "Text.mui", &[]).unwrap();
    let e = make(&mut a, "Application.mui", &[(MUIA_Application_Window, t)]).unwrap_err().0;
    assert!(e.contains("a Text object cannot be a child of a Application"), "{e}");
}

#[test]
fn set_and_get_store_values_notify_fires_and_dispose_needs_the_root() {
    let mut a = machine();
    let win = make(&mut a, "Window.mui", &[]).unwrap();
    let app = make(&mut a, "Application.mui", &[(MUIA_Application_Window, win)]).unwrap();
    // Notify: setting CloseRequest=TRUE queues ReturnID(Quit) on the app.
    method(&mut a, win, MUIM_Notify, &[MUIA_Window_CloseRequest, 1, app, 2, MUIM_Application_ReturnID, MUIV_Application_ReturnID_Quit]).unwrap();
    set_attr(&mut a, win, MUIA_Window_CloseRequest, 0).unwrap(); // NEGATIVE CONTROL: trigger value 1 does not match 0
    assert_eq!(method(&mut a, app, MUIM_Application_NewInput, &[0]).unwrap(), 0);
    set_attr(&mut a, win, MUIA_Window_CloseRequest, 1).unwrap();
    assert_eq!(get_attr(&mut a, win, MUIA_Window_CloseRequest).unwrap(), 1);
    assert_eq!(method(&mut a, app, MUIM_Application_NewInput, &[0]).unwrap(), MUIV_Application_ReturnID_Quit);
    assert!(dispose(&mut a, win).unwrap_err().0.contains("owned by a parent"));
    let e = method(&mut a, app, MUIM_Application_OpenConfigWindow, &[0]).unwrap_err().0;
    assert_eq!(e, "MUI Application class: MUIM_Application_OpenConfigWindow is not implemented");
    dispose(&mut a, app).unwrap();
    assert!(a.objects.map.is_empty());
    assert!(dispose(&mut a, app).unwrap_err().0.contains("not a MUI object"), "double dispose is refused");
}

#[test]
fn a_label_before_a_text_is_that_texts_label_and_the_window_is_sized_by_the_screen_fonts_metrics() {
    let mut a = machine();
    let hello = cstr(&mut a, "hello");
    let lab = {
        let p = a.heap.alloc(8).unwrap();
        let agent = cstr(&mut a, "Agent");
        a.mem.write_u32(p, agent).unwrap();
        a.mem.write_u32(p + 4, 0).unwrap();
        make::make(&mut a, MUIO_Label, p).unwrap()
    };
    let txt = make(&mut a, "Text.mui", &[(MUIA_Text_Contents, hello), (MUIA_Frame, MUIV_Frame_Text)]).unwrap();
    let grp = make(&mut a, "Group.mui", &[(MUIA_Group_Columns, 2), (MUIA_Group_Child, lab), (MUIA_Group_Child, txt)]).unwrap();
    let title = cstr(&mut a, "T");
    let win = make(&mut a, "Window.mui", &[(MUIA_Window_Title, title), (MUIA_Window_RootObject, grp)]).unwrap();
    set_attr(&mut a, win, MUIA_Window_Open, 1).unwrap();
    let (m, c) = match (&a.ui.calls[0], &a.ui.calls[1]) { (UiCall::OpenWindow(m), UiCall::AddControl(_, c)) => (m.clone(), c.clone()), other => panic!("{other:?}") };
    assert_eq!(a.ui.calls.iter().filter(|c| matches!(c, UiCall::AddControl(..))).count(), 1, "the label is not a control of its own");
    assert_eq!((m.title.as_str(), c.label.as_str()), ("T", "Agent"));
    let tf = crate::graphics::fonts::default_font(&a).unwrap();
    let font = crate::graphics::fonts::font_at(&mut a, tf).unwrap();
    let (agent_w, hello_w, fy) = (font.text_length(b"Agent"), font.text_length(b"hello"), font.y_size as i32);
    // The text cell: its TextLength plus the frame, one text line plus the frame; the label cell is 'Agent' wide and the columns are 6 apart.
    assert_eq!((c.rect.width, c.rect.height), (hello_w + 8, fy + 6));
    assert_eq!(c.rect.left, 4 /* border */ + layout::WIN_PAD.0 + agent_w + 6, "left of the text cell: window padding, label column, group spacing");
    assert_eq!(m.rect.width, 8 /* chrome */ + layout::WIN_PAD.0 + layout::WIN_PAD.1 + agent_w + 6 + hello_w + 8);
    // A title change while open reaches the host (SetWindowTitle); a window with no root object cannot open.
    let t2 = cstr(&mut a, "T2");
    set_attr(&mut a, win, MUIA_Window_Title, t2).unwrap();
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::SetWindowTitle(_, t) if t == "T2")));
    let bare = make(&mut a, "Window.mui", &[]).unwrap();
    assert!(set_attr(&mut a, bare, MUIA_Window_Open, 1).unwrap_err().0.contains("no MUIA_Window_RootObject"));
    assert_eq!(get_attr(&mut a, bare, MUIA_Window_Open).unwrap(), 0);
}

// ---- roles and intents: each class's native control, and what a native intent does to the object ----
use guestui_abi::{CallbackToken, ChoiceStyle, ControlKind, ControlValue, GuestUiEvent, Intent};

/// An application with one window whose root group holds `kids`; the window is open.
fn open_ui(a: &mut Amiga<RecordingGuestUi>, kids: &[u32]) -> (u32, u32) {
    let items: Vec<(u32, u32)> = kids.iter().map(|&k| (MUIA_Group_Child, k)).collect();
    let root = make(a, "Group.mui", &items).unwrap();
    let title = cstr(a, "t");
    let win = make(a, "Window.mui", &[(MUIA_Window_Title, title), (MUIA_Window_RootObject, root)]).unwrap();
    let app = make(a, "Application.mui", &[(MUIA_Application_Window, win)]).unwrap();
    set_attr(a, win, MUIA_Window_Open, 1).unwrap();
    (app, win)
}
/// MUI_MakeObjectA(`ty`, params...) with every parameter a longword (strings as guest pointers).
fn made(a: &mut Amiga<RecordingGuestUi>, ty: u32, params: &[u32]) -> u32 {
    let p = a.heap.alloc(4 * params.len().max(1) as u32).unwrap();
    for (i, &v) in params.iter().enumerate() { a.mem.write_u32(p + 4 * i as u32, v).unwrap(); }
    make::make(a, ty, p).unwrap()
}
/// A NULL-terminated guest array of C strings.
fn strings(a: &mut Amiga<RecordingGuestUi>, items: &[&str]) -> u32 {
    let p = a.heap.alloc(4 * (items.len() as u32 + 1)).unwrap();
    for (i, s) in items.iter().enumerate() { let sp = cstr(a, s); a.mem.write_u32(p + 4 * i as u32, sp).unwrap(); }
    a.mem.write_u32(p + 4 * items.len() as u32, 0).unwrap();
    p
}
/// Deliver `intent` to the native control of `obj` and let the application process its input (the program's NewInput).
fn native(a: &mut Amiga<RecordingGuestUi>, app: u32, obj: u32, intent: Intent) -> Result<(), crate::Refusal> {
    let p = a.objects.proj.get(obj as u64).expect("the object is projected").clone();
    a.deliver(GuestUiEvent::intent(p.window, p.native, CallbackToken(obj as u64), intent))?;
    app::pump(a, app)
}
/// When `attr` of `obj` is set, ReturnID(the trigger value) goes to `app`.
fn return_on(a: &mut Amiga<RecordingGuestUi>, obj: u32, app: u32, attr: u32) {
    method(a, obj, MUIM_Notify, &[attr, MUIV_EveryTime, app, 2, MUIM_Application_ReturnID, MUIV_TriggerValue]).unwrap();
}
fn next_id(a: &mut Amiga<RecordingGuestUi>, app: u32) -> u32 { method(a, app, MUIM_Application_NewInput, &[0]).unwrap() }
fn kind_of_obj(a: &Amiga<RecordingGuestUi>, obj: u32) -> ControlKind { a.objects.proj.get(obj as u64).unwrap().model.kind.clone() }

#[test]
fn a_cycle_is_a_native_choice_and_a_native_choice_sets_the_active_entry_and_notifies() {
    let mut a = machine();
    let entries = strings(&mut a, &["Red", "Green", "Blue"]);
    let cyc = made(&mut a, MUIO_Cycle, &[0, entries]);
    let (app, _) = open_ui(&mut a, &[cyc]);
    assert_eq!(kind_of_obj(&a, cyc), ControlKind::Choice { style: ChoiceStyle::Cycle, labels: alloc::vec!["Red".into(), "Green".into(), "Blue".into()], selected: 0 });
    return_on(&mut a, cyc, app, MUIA_Cycle_Active);
    native(&mut a, app, cyc, Intent::SetValue(ControlValue::Choice(2))).unwrap();
    assert_eq!(get_attr(&mut a, cyc, MUIA_Cycle_Active).unwrap(), 2);
    assert_eq!(next_id(&mut a, app), 2, "MUIM_Notify ran with the trigger value");
    // The host already shows the choice: the refresh pushed no echo.
    assert!(!a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, _, ControlValue::Choice(_)))));
    // NEGATIVE CONTROLS: a level is not a choice; a row selection means nothing to a cycle. Both refused by name, nothing changes.
    for bad in [Intent::SetValue(ControlValue::Level(1)), Intent::SelectRow(1)] {
        let e = native(&mut a, app, cyc, bad).unwrap_err().0;
        assert!(e.contains("Intent") && (e.contains("not one the control holds") || e.contains("means nothing")), "{e}");
    }
    assert_eq!((get_attr(&mut a, cyc, MUIA_Cycle_Active).unwrap(), next_id(&mut a, app)), (2, 0));
    // MUIV_Cycle_Active_Next wraps around the entries.
    set_attr(&mut a, cyc, MUIA_Cycle_Active, MUIV_Cycle_Active_Next).unwrap();
    assert_eq!(get_attr(&mut a, cyc, MUIA_Cycle_Active).unwrap(), 0);
}

#[test]
fn a_checkmark_a_string_and_a_slider_take_their_native_values_into_the_objects() {
    let mut a = machine();
    let (check, string, slider) = (made(&mut a, MUIO_Checkmark, &[0]), made(&mut a, MUIO_String, &[0, 40]), made(&mut a, MUIO_Slider, &[0, 10, 20]));
    let (app, _) = open_ui(&mut a, &[check, string, slider]);
    assert_eq!(kind_of_obj(&a, check), ControlKind::Checkbox { checked: false });
    assert!(matches!(kind_of_obj(&a, string), ControlKind::Field { max_len: 39, editable: true, integer: false, .. }));
    assert_eq!(kind_of_obj(&a, slider), ControlKind::Slider { min: 10, max: 20, value: 10 }, "a Numeric value starts at its minimum");
    for (o, at) in [(check, MUIA_Selected), (string, MUIA_String_Acknowledge), (slider, MUIA_Numeric_Value)] { return_on(&mut a, o, app, at); }
    native(&mut a, app, check, Intent::SetValue(ControlValue::Checked(true))).unwrap();
    assert_eq!((get_attr(&mut a, check, MUIA_Selected).unwrap(), next_id(&mut a, app)), (1, 1));
    native(&mut a, app, check, Intent::SetValue(ControlValue::Checked(false))).unwrap();
    assert_eq!((get_attr(&mut a, check, MUIA_Selected).unwrap(), next_id(&mut a, app)), (0, 0));
    // A string: Return commits; Contents is set, then Acknowledge, whose notification carries a pointer to the object's copy.
    native(&mut a, app, string, Intent::CommitText("hello".into())).unwrap();
    let ptr = get_attr(&mut a, string, MUIA_String_Contents).unwrap();
    assert_eq!(a.mem.read_cstr(ptr).unwrap(), "hello");
    assert_eq!(next_id(&mut a, app), ptr, "the Acknowledge notification's trigger value is the contents pointer");
    // A slider stays in its range: the host's level is taken, an out-of-range level never reaches the object as given.
    native(&mut a, app, slider, Intent::SetValue(ControlValue::Level(15))).unwrap();
    assert_eq!((get_attr(&mut a, slider, MUIA_Numeric_Value).unwrap(), next_id(&mut a, app)), (15, 15));
    native(&mut a, app, slider, Intent::Drag(ControlValue::Level(99))).unwrap();
    assert_eq!(get_attr(&mut a, slider, MUIA_Numeric_Value).unwrap(), 20, "clamped to MUIA_Numeric_Max like Numeric.mui");
}

#[test]
fn a_register_shows_one_page_and_a_native_tab_turns_it() {
    let mut a = machine();
    let titles = strings(&mut a, &["One", "Two"]);
    let (t1, t2) = (cstr(&mut a, "page one"), cstr(&mut a, "page two"));
    let (p1, p2) = (make(&mut a, "Text.mui", &[(MUIA_Text_Contents, t1)]).unwrap(), make(&mut a, "Text.mui", &[(MUIA_Text_Contents, t2)]).unwrap());
    let reg = make(&mut a, "Register.mui", &[(MUIA_Register_Titles, titles), (MUIA_Group_Child, p1), (MUIA_Group_Child, p2)]).unwrap();
    let (app, _) = open_ui(&mut a, &[reg]);
    assert!(matches!(kind_of_obj(&a, reg), ControlKind::Choice { style: ChoiceStyle::Tabs, selected: 0, .. }));
    assert!(a.objects.proj.get(p1 as u64).is_some() && a.objects.proj.get(p2 as u64).is_none(), "only the active page has controls");
    let first_gen = a.objects.proj.get(p1 as u64).unwrap().native;
    native(&mut a, app, reg, Intent::SetValue(ControlValue::Choice(1))).unwrap();
    assert_eq!(get_attr(&mut a, reg, MUIA_Group_ActivePage).unwrap(), 1);
    assert!(a.objects.proj.get(p1 as u64).is_none() && a.objects.proj.get(p2 as u64).is_some(), "the old page's control is retired, the new page's added");
    assert!(matches!(a.ui.calls.last(), Some(UiCall::SetControl(..)) | Some(UiCall::AddControl(..))), "{:?}", a.ui.calls.last());
    // A stale intent aimed at the retired page control is refused by name.
    let e = a.deliver(GuestUiEvent::intent(a.objects.proj.get(p2 as u64).unwrap().window, first_gen, CallbackToken(p1 as u64), Intent::Activate)).unwrap_err().0;
    assert!(e.contains("stale") || e.contains("not a live control"), "{e}");
}

#[test]
fn a_list_shows_its_entries_a_native_row_becomes_list_active_and_getentry_answers_with_the_programs_pointer() {
    let mut a = machine();
    let list = make(&mut a, "List.mui", &[]).unwrap();
    let lv = make(&mut a, "Listview.mui", &[(MUIA_Listview_List, list)]).unwrap();
    let (app, _) = open_ui(&mut a, &[lv]);
    let rows: Vec<u32> = ["alpha", "beta", "gamma"].iter().map(|s| cstr(&mut a, s)).collect();
    for &r in &rows { method(&mut a, list, MUIM_List_InsertSingle, &[r, MUIV_List_Insert_Bottom]).unwrap(); }
    match kind_of_obj(&a, list) {
        ControlKind::List { columns, rows: r, selected: None, .. } => { assert!(columns.is_empty()); assert_eq!(r, [["alpha"], ["beta"], ["gamma"]]); }
        other => panic!("{other:?}"),
    }
    assert_eq!(get_attr(&mut a, list, MUIA_List_Entries).unwrap(), 3);
    assert_eq!(get_attr(&mut a, list, MUIA_List_Active).unwrap(), MUIV_List_Active_Off, "nothing is active at first");
    return_on(&mut a, list, app, MUIA_List_Active);
    native(&mut a, app, list, Intent::SelectRow(1)).unwrap();
    assert_eq!((get_attr(&mut a, list, MUIA_List_Active).unwrap(), next_id(&mut a, app)), (1, 1));
    let out = a.heap.alloc(4).unwrap();
    method(&mut a, list, MUIM_List_GetEntry, &[MUIV_List_GetEntry_Active, out]).unwrap();
    assert_eq!(a.mem.read_u32(out).unwrap(), rows[1], "GetEntry hands back the entry pointer the program inserted");
    method(&mut a, list, MUIM_List_Remove, &[MUIV_List_Remove_First]).unwrap();
    assert_eq!(get_attr(&mut a, list, MUIA_List_Active).unwrap(), 0, "the active row moved up with its entry");
    method(&mut a, list, MUIM_List_Clear, &[]).unwrap();
    assert!(matches!(kind_of_obj(&a, list), ControlKind::List { ref rows, selected: None, .. } if rows.is_empty()));
    // With a display hook the row's cells come from the program: the engine queues the guest call (A2 = the cell array, A1 = the entry)
    // and the row stays empty until it has run (a guest-driven test below runs it).
    set_attr(&mut a, list, MUIA_List_DisplayHook, 0x1234).unwrap();
    method(&mut a, list, MUIM_List_InsertSingle, &[rows[0], MUIV_List_Insert_Bottom]).unwrap();
    let queued: Vec<_> = a.objects.hooks.iter().map(|h| (h.hook, matches!(h.msg, hooks::Msg::Word(w) if w == rows[0]))).collect();
    assert_eq!(queued, [(0x1234, true)]);
    assert!(matches!(kind_of_obj(&a, list), ControlKind::List { ref rows, .. } if rows.len() == 1 && rows[0] == [String::new()]));
}

#[test]
fn disabled_and_hidden_objects_reach_the_host_and_a_scrollbar_scrolls() {
    let mut a = machine();
    let (b1, b2) = { let l = cstr(&mut a, "Go"); let l2 = cstr(&mut a, "Stop"); (made(&mut a, MUIO_Button, &[l]), made(&mut a, MUIO_Button, &[l2])) };
    let bar = make(&mut a, "Scrollbar.mui", &[(MUIA_Prop_Entries, 100), (MUIA_Prop_Visible, 10)]).unwrap();
    let (app, _) = open_ui(&mut a, &[b1, b2, bar]);
    assert_eq!(a.objects.proj.get(b1 as u64).unwrap().model.label, "Go");
    set_attr(&mut a, b1, MUIA_Disabled, 1).unwrap();
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControlEnabled(_, _, false))), "Disabled ghosts the native button");
    set_attr(&mut a, b2, MUIA_ShowMe, 0).unwrap();
    assert!(a.objects.proj.get(b2 as u64).is_none(), "ShowMe FALSE removes the control");
    set_attr(&mut a, b2, MUIA_ShowMe, 1).unwrap();
    assert!(a.objects.proj.get(b2 as u64).is_some(), "and ShowMe TRUE brings it back");
    native(&mut a, app, bar, Intent::ScrollTo(30)).unwrap();
    assert_eq!(get_attr(&mut a, bar, MUIA_Prop_First).unwrap(), 30);
    assert!(matches!(kind_of_obj(&a, bar), ControlKind::ScrollBar { vertical: true, total: 100, visible: 10, top: 30 }));
    // NEGATIVE CONTROL: a text commit means nothing to a scrollbar.
    assert!(native(&mut a, app, bar, Intent::CommitText("x".into())).unwrap_err().0.contains("means nothing"));
}

#[test]
fn a_menustrip_is_built_from_a_newmenu_array_and_make_object_refuses_what_it_does_not_know() {
    let mut a = machine();
    // struct NewMenu { UBYTE type; STRPTR label; STRPTR commkey; UWORD flags; LONG mutual; APTR userdata } = 20 bytes (type 0, label 2, comm 6, flags 10, mutual 12, user 16).
    let entries: [(u8, &str, &str, u16, u32); 4] = [(1, "Project", "", 0, 0), (2, "Open", "o", 0, 7), (2, "Quit", "q", 0, 9), (0, "", "", 0, 0)];
    let nm = a.heap.alloc(20 * 4).unwrap();
    for (i, (ty, label, key, fl, user)) in entries.iter().enumerate() {
        let at = nm + 20 * i as u32;
        let (l, k) = (if label.is_empty() { 0 } else { cstr(&mut a, label) }, if key.is_empty() { 0 } else { cstr(&mut a, key) });
        a.mem.write_u8(at, *ty).unwrap(); a.mem.write_u8(at + 1, 0).unwrap();
        for (off, v) in [(2, l), (6, k), (12, 0), (16, *user)] { a.mem.write_u32(at + off, v).unwrap(); }
        a.mem.write_u16(at + 10, *fl).unwrap();
    }
    let strip = made(&mut a, MUIO_MenustripNM, &[nm, 0]);
    let m = menu::model(&a.objects, strip);
    assert_eq!(m.menus.len(), 1);
    assert_eq!(m.menus[0].title, "Project");
    assert_eq!(m.menus[0].items.iter().map(|i| (i.label.as_str(), i.shortcut)).collect::<Vec<_>>(), [("Open", Some('o')), ("Quit", Some('q'))]);
    let p = a.heap.alloc(4).unwrap();
    let e = make::make(&mut a, 99, p).unwrap_err().0;
    assert_eq!(e, "MUI_MakeObjectA: object type 99 is not implemented");
    let e = make::make(&mut a, MUIO_NumericButton, p).unwrap_err().0;
    assert!(e.contains("type 16 is not implemented"), "{e}");
}
