//! MUI object model: refusals are named, a failed build leaks nothing, labels pair with their controls.
use super::*;
use crate::Amiga;
use alloc::vec::Vec;
use heirloom_ui_abi::{RecordingGuestUi, UiCall};

fn machine() -> Amiga<RecordingGuestUi> { Amiga::new(RecordingGuestUi::new()).unwrap() }

/// Write a TagItem list (and any string) into guest RAM; the heap blocks leak on purpose (test-only).
fn tags(a: &mut Amiga<RecordingGuestUi>, items: &[(u32, u32)]) -> u32 {
    let p = a.heap.alloc(8 * (items.len() as u32 + 1)).unwrap();
    for (i, &(t, d)) in items.iter().chain([(0, 0)].iter()).enumerate() { a.mem.write_u32(p + 8 * i as u32, t).unwrap(); a.mem.write_u32(p + 8 * i as u32 + 4, d).unwrap(); }
    p
}
fn cstr(a: &mut Amiga<RecordingGuestUi>, s: &str) -> u32 { let p = a.heap.alloc(s.len() as u32 + 1).unwrap(); a.mem.write_bytes(p, s.as_bytes()).unwrap(); p }
/// A guest C string with its NUL written (the heap hands back recycled blocks, so the NUL cannot be assumed).
fn zstr(a: &mut Amiga<RecordingGuestUi>, s: &str) -> u32 { let p = a.heap.alloc(s.len() as u32 + 1).unwrap(); a.mem.write_bytes(p, s.as_bytes()).unwrap(); a.mem.write_u8(p + s.len() as u32, 0).unwrap(); p }
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
fn a_no_notify_true_anywhere_in_a_set_list_silences_the_whole_call() {
    // Notify.mui's OM_SET checks the list for MUIA_NoNotify first: `nnset(obj, attr, v)` is SetAttrs(obj, MUIA_NoNotify, TRUE, attr, v, TAG_DONE).
    let mut a = machine();
    let win = make(&mut a, "Window.mui", &[]).unwrap();
    let app = make(&mut a, "Application.mui", &[(MUIA_Application_Window, win)]).unwrap();
    method(&mut a, win, MUIM_Notify, &[MUIA_Window_CloseRequest, 1, app, 2, MUIM_Application_ReturnID, MUIV_Application_ReturnID_Quit]).unwrap();
    set_list(&mut a, win, &[(MUIA_Window_CloseRequest, 1), (MUIA_NoNotify, 1)]).unwrap(); // the NoNotify AFTER the tag still silences it
    assert_eq!(get_attr(&mut a, win, MUIA_Window_CloseRequest).unwrap(), 1, "the value is stored");
    assert_eq!(method(&mut a, app, MUIM_Application_NewInput, &[0]).unwrap(), 0, "no notification ran");
    // NEGATIVE CONTROLS: without the tag, or with it FALSE, the same set notifies.
    set_attr(&mut a, win, MUIA_Window_CloseRequest, 0).unwrap();
    set_list(&mut a, win, &[(MUIA_Window_CloseRequest, 1)]).unwrap();
    assert_eq!(method(&mut a, app, MUIM_Application_NewInput, &[0]).unwrap(), MUIV_Application_ReturnID_Quit);
    set_list(&mut a, win, &[(MUIA_Window_CloseRequest, 0)]).unwrap();
    set_list(&mut a, win, &[(MUIA_NoNotify, 0), (MUIA_Window_CloseRequest, 1)]).unwrap();
    assert_eq!(method(&mut a, app, MUIM_Application_NewInput, &[0]).unwrap(), MUIV_Application_ReturnID_Quit);
}

#[test]
fn windows_join_and_leave_a_live_application_with_om_addmember_and_remmember() {
    let mut a = machine();
    let app = make(&mut a, "Application.mui", &[]).unwrap();
    let txt = make(&mut a, "Text.mui", &[]).unwrap();
    let win = make(&mut a, "Window.mui", &[(MUIA_Window_RootObject, txt)]).unwrap();
    assert_eq!(method(&mut a, app, OM_ADDMEMBER, &[win]).unwrap(), 1);
    assert_eq!(a.objects.children(app), &[win], "the application owns the window now");
    set_attr(&mut a, win, MUIA_Window_Open, 1).unwrap();
    assert_eq!(a.ui.calls.iter().filter(|c| matches!(c, UiCall::OpenWindow(_))).count(), 1);
    method(&mut a, app, OM_REMMEMBER, &[win]).unwrap();
    assert_eq!(get_attr(&mut a, win, MUIA_Window_Open).unwrap(), 0, "a window leaving the application closes first");
    assert!(a.objects.children(app).is_empty() && a.objects.map[&win].parent.is_none(), "the program owns it again");
    // NEGATIVE CONTROLS: a second removal, and a child that is no window.
    assert!(method(&mut a, app, OM_REMMEMBER, &[win]).unwrap_err().0.contains("is not a child"));
    let t2 = make(&mut a, "Text.mui", &[]).unwrap();
    assert!(method(&mut a, app, OM_ADDMEMBER, &[t2]).unwrap_err().0.contains("cannot be a child of a Application"));
}

#[test]
fn semaphore_nests_for_its_owner_refuses_to_block_and_release_needs_a_hold() {
    let mut a = machine();
    let sem = make(&mut a, "Semaphore.mui", &[]).unwrap();
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_Obtain, &[]).unwrap(), 1);
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_Obtain, &[]).unwrap(), 1, "the owner nests");
    a.sched.cur = 0x1234; // another task
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_Attempt, &[]).unwrap(), 0, "Attempt answers FALSE while another task holds it");
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_AttemptShared, &[]).unwrap(), 0);
    assert!(method(&mut a, sem, MUIM_Semaphore_Obtain, &[]).unwrap_err().0.contains("would block"), "an Obtain that would wait is a named refusal");
    assert!(method(&mut a, sem, MUIM_Semaphore_Release, &[]).unwrap_err().0.contains("does not hold"), "only the holder releases");
    a.sched.cur = crate::process::PROCESS;
    method(&mut a, sem, MUIM_Semaphore_Release, &[]).unwrap();
    a.sched.cur = 0x1234;
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_Attempt, &[]).unwrap(), 0, "still held: the nesting count was 2");
    a.sched.cur = crate::process::PROCESS;
    method(&mut a, sem, MUIM_Semaphore_Release, &[]).unwrap();
    a.sched.cur = 0x1234;
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_ObtainShared, &[]).unwrap(), 1, "free now");
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_AttemptShared, &[]).unwrap(), 1, "shared holders add up");
    assert_eq!(method(&mut a, sem, MUIM_Semaphore_Attempt, &[]).unwrap(), 0, "but exclude an exclusive request");
    a.sched.cur = crate::process::PROCESS;
    assert!(method(&mut a, sem, MUIM_Semaphore_Obtain, &[]).unwrap_err().0.contains("would block"));
    // A Semaphore.mui subclass is a class of the tree: Dataspace.mui is one.
    assert!(classes::Kind::Dataspace.is_a(classes::Kind::Semaphore) && !classes::Kind::Semaphore.is_a(classes::Kind::Notify));
}

#[test]
fn dataspace_adds_replaces_finds_removes_and_clears() {
    let mut a = machine();
    let ds = make(&mut a, "Dataspace.mui", &[]).unwrap();
    let put = |a: &mut Amiga<RecordingGuestUi>, bytes: &[u8]| { let p = a.heap.alloc(bytes.len() as u32).unwrap(); a.mem.write_bytes(p, bytes).unwrap(); p };
    let d1 = put(&mut a, &[1, 2, 3, 4]);
    assert_eq!(method(&mut a, ds, MUIM_Dataspace_Add, &[d1, 4, 7]).unwrap(), 1);
    let at = method(&mut a, ds, MUIM_Dataspace_Find, &[7]).unwrap();
    assert_eq!(a.mem.read_u32(at).unwrap(), 0x0102_0304, "Find returns the address of the stored bytes");
    let d2 = put(&mut a, &[9, 9]);
    method(&mut a, ds, MUIM_Dataspace_Add, &[d2, 2, 7]).unwrap();
    let at2 = method(&mut a, ds, MUIM_Dataspace_Find, &[7]).unwrap();
    assert_eq!(a.mem.read_u16(at2).unwrap(), 0x0909, "the same id is replaced");
    // NEGATIVE CONTROLS (dataspace.c Add): no data, len < 1 and id < 1 add nothing; an unknown id finds nothing.
    assert_eq!((method(&mut a, ds, MUIM_Dataspace_Add, &[0, 4, 8]).unwrap(), method(&mut a, ds, MUIM_Dataspace_Add, &[d1, 0, 8]).unwrap(), method(&mut a, ds, MUIM_Dataspace_Add, &[d1, 4, 0]).unwrap()), (0, 0, 0));
    assert_eq!(method(&mut a, ds, MUIM_Dataspace_Find, &[8]).unwrap(), 0);
    assert_eq!(method(&mut a, ds, MUIM_Dataspace_Remove, &[8]).unwrap(), 0);
    assert_eq!(method(&mut a, ds, MUIM_Dataspace_Remove, &[7]).unwrap(), 1);
    assert_eq!(method(&mut a, ds, MUIM_Dataspace_Find, &[7]).unwrap(), 0);
    method(&mut a, ds, MUIM_Dataspace_Add, &[d1, 4, 3]).unwrap();
    method(&mut a, ds, MUIM_Dataspace_Clear, &[]).unwrap();
    assert_eq!(method(&mut a, ds, MUIM_Dataspace_Find, &[3]).unwrap(), 0);
    assert!(method(&mut a, ds, MUIM_Dataspace_WriteIFF, &[0, 0, 0]).unwrap_err().0.contains("iffparse.library"));
    dispose(&mut a, ds).unwrap();
}

#[test]
fn application_save_and_load_keep_the_objects_that_have_an_object_id() {
    let mut a = machine();
    let (base, one, two) = (cstr(&mut a, "TestApp"), cstr(&mut a, "one"), cstr(&mut a, "two"));
    let arr = a.heap.alloc(12).unwrap();
    for (i, p) in [one, two, 0].into_iter().enumerate() { a.mem.write_u32(arr + 4 * i as u32, p).unwrap(); }
    let hello = cstr(&mut a, "hello");
    let s = make(&mut a, "String.mui", &[(MUIA_ObjectID, 5), (MUIA_String_Contents, hello)]).unwrap();
    let cy = make(&mut a, "Cycle.mui", &[(MUIA_ObjectID, 6), (MUIA_Cycle_Entries, arr), (MUIA_Cycle_Active, 1)]).unwrap();
    let sl = make(&mut a, "Slider.mui", &[(MUIA_ObjectID, 7), (MUIA_Numeric_Value, 40)]).unwrap();
    let anon = make(&mut a, "Slider.mui", &[(MUIA_Numeric_Value, 11)]).unwrap(); // no ObjectID: not saved
    let grp = make(&mut a, "Group.mui", &[(MUIA_Group_Child, s), (MUIA_Group_Child, cy), (MUIA_Group_Child, sl), (MUIA_Group_Child, anon)]).unwrap();
    let win = make(&mut a, "Window.mui", &[(MUIA_Window_RootObject, grp)]).unwrap();
    let app = make(&mut a, "Application.mui", &[(MUIA_Application_Base, base), (MUIA_Application_Window, win)]).unwrap();
    method(&mut a, app, MUIM_Application_Save, &[0]).unwrap();
    let file = { let n = a.fs.vol.resolve(crate::fs::cur_dir(&a), "ENV:Zune/TestApp.cfg").unwrap(); a.fs.vol.file_data(n).unwrap().to_vec() };
    assert_eq!(store::decode(&file), alloc::vec![(5, b"hello\0".to_vec()), (6, 1u32.to_be_bytes().to_vec()), (7, 40u32.to_be_bytes().to_vec())], "the three identified objects, in tree order");
    // Change everything, load, and see the saved state back.
    let bye = cstr(&mut a, "bye");
    set_attr(&mut a, s, MUIA_String_Contents, bye).unwrap();
    set_attr(&mut a, cy, MUIA_Cycle_Active, 0).unwrap();
    set_attr(&mut a, sl, MUIA_Numeric_Value, 90).unwrap();
    set_attr(&mut a, anon, MUIA_Numeric_Value, 12).unwrap();
    method(&mut a, app, MUIM_Application_Load, &[0]).unwrap();
    assert_eq!((get_attr(&mut a, cy, MUIA_Cycle_Active).unwrap(), get_attr(&mut a, sl, MUIA_Numeric_Value).unwrap()), (1, 40));
    let sp = get_attr(&mut a, s, MUIA_String_Contents).unwrap();
    assert_eq!(a.mem.read_cstr(sp).unwrap(), "hello");
    assert_eq!(get_attr(&mut a, anon, MUIA_Numeric_Value).unwrap(), 12, "NEGATIVE CONTROL: an object without an ObjectID is untouched");
    // NEGATIVE CONTROLS: a missing file imports nothing; an application without MUIA_Application_Base saves and loads nothing.
    set_attr(&mut a, sl, MUIA_Numeric_Value, 90).unwrap();
    method(&mut a, app, MUIM_Application_Load, &[u32::MAX]).unwrap(); // ENVARC:Zune/TestApp.cfg was never saved
    assert_eq!(get_attr(&mut a, sl, MUIA_Numeric_Value).unwrap(), 90);
    let app2 = make(&mut a, "Application.mui", &[]).unwrap();
    method(&mut a, app2, MUIM_Application_Save, &[0]).unwrap();
    method(&mut a, app2, MUIM_Application_Load, &[0]).unwrap();
    assert_eq!(get_attr(&mut a, sl, MUIA_Numeric_Value).unwrap(), 90);
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
use heirloom_ui_abi::{CallbackToken, ChoiceStyle, ControlKind, ControlValue, GuestUiEvent, Intent};

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

#[test]
fn a_control_the_host_refuses_costs_the_control_never_the_window() {
    use crate::mui::win::{degrade, is_open};
    use heirloom_ui_abi::ControlModel;
    let mut a = machine();
    a.ui.refuse_kinds.push("Gauge");
    let g = make(&mut a, "Gauge.mui", &[(MUIA_Gauge_Max, 200), (MUIA_Gauge_Current, 50)]).unwrap();
    let b = made(&mut a, MUIO_Button, &[0]);
    let (_app, win) = open_ui(&mut a, &[g, b]);
    assert!(is_open(&a, win), "the window is open: one refused control did not close it");
    // The refusal is named, the Gauge is a framed label reading its value, and the sibling control is natively there.
    assert!(a.refused.iter().any(|r| r.contains("refused to draw it (Gauge)")), "refused: {:?}", a.refused);
    assert_eq!(kind_of_obj(&a, g), ControlKind::Label { text: "25%".into(), framed: true });
    assert!(matches!(kind_of_obj(&a, b), ControlKind::Button));
    // The value moving re-shows the new label, with no second refusal line.
    let n = a.refused.len();
    set_attr(&mut a, g, MUIA_Gauge_Current, 100).unwrap();
    assert_eq!(kind_of_obj(&a, g), ControlKind::Label { text: "50%".into(), framed: true });
    assert_eq!(a.refused.len(), n);
    assert_eq!(degrade(&ControlModel { kind: ControlKind::Gauge { value: 1, max: 3 }, label: "".into(), rect: heirloom_ui_abi::GuestRect { left: 0, top: 0, width: 1, height: 1 }, enabled: true, token: CallbackToken(0) }).kind, ControlKind::Label { text: "33%".into(), framed: true });
}

// ---- the image classes draw (images.rs) ------------------------------------------------------------------------------------------------

use heirloom_ui_abi::{DrawOp, GuestRect, IconRole, PenRole};

fn bytes(a: &mut Amiga<RecordingGuestUi>, b: &[u8]) -> u32 { let p = a.heap.alloc(b.len() as u32 + 4).unwrap(); a.mem.write_bytes(p, b).unwrap(); p }

/// A window holding `child` in a group, opened; the planes the host was shown.
fn planes_of(a: &mut Amiga<RecordingGuestUi>, child: u32) -> Vec<heirloom_ui_abi::Plane> {
    let grp = make(a, "Group.mui", &[(MUIA_Group_Child, child)]).unwrap();
    let win = make(a, "Window.mui", &[(MUIA_Window_RootObject, grp)]).unwrap();
    let _app = make(a, "Application.mui", &[(MUIA_Application_Window, win)]).unwrap();
    set_attr(a, win, MUIA_Window_Open, 1).unwrap();
    a.ui.calls.iter().filter_map(|c| match c { UiCall::PresentPlane(_, p) => Some(p.clone()), _ => None }).collect()
}

fn pixels(p: &heirloom_ui_abi::Plane) -> Vec<(GuestRect, Vec<u32>)> { p.ops.iter().filter_map(|o| match o { DrawOp::Pixels { rect, rgb } => Some((*rect, rgb.clone())), _ => None }).collect() }

#[test]
fn a_chunky_image_draws_its_pixels_through_its_palette_honouring_modulo_and_the_transparent_pen() {
    let mut a = machine();
    // 3x2 pens in rows of modulo 4 (a pad byte per row); palette: 0 black, 1 red, 2 green.
    let px = bytes(&mut a, &[1, 2, 0, 9, 0, 1, 2, 9]);
    let pal = bytes(&mut a, &[0, 0, 0, 255, 0, 0, 0, 255, 0]);
    let mk = |a: &mut Amiga<RecordingGuestUi>, extra: &[(u32, u32)]| {
        let mut t = alloc::vec![(MUIA_ChunkyImage_Pixels, px), (MUIA_ChunkyImage_Palette, pal), (MUIA_ChunkyImage_NumColors, 3), (MUIA_ChunkyImage_Modulo, 4), (MUIA_Bitmap_Width, 3), (MUIA_Bitmap_Height, 2), (MUIA_FixWidth, 3), (MUIA_FixHeight, 2)];
        t.extend_from_slice(extra);
        make(a, "ChunkyImage.mui", &t).unwrap()
    };
    let img = mk(&mut a, &[]);
    let planes = planes_of(&mut a, img);
    assert_eq!(planes.len(), 1, "one content plane for the image");
    let px_ops = pixels(&planes[0]);
    assert_eq!(px_ops.len(), 1);
    assert_eq!((px_ops[0].0.width, px_ops[0].0.height), (3, 2));
    assert_eq!(px_ops[0].1, alloc::vec![0xFF0000, 0x00FF00, 0, 0, 0xFF0000, 0x00FF00], "pens through the palette; the modulo pad byte is skipped");
    // Transparent pen 0: those pixels are not drawn (runs of opaque pixels only).
    let mut b = machine();
    let (px, pal) = (bytes(&mut b, &[1, 2, 0, 9, 0, 1, 2, 9]), bytes(&mut b, &[0, 0, 0, 255, 0, 0, 0, 255, 0]));
    let t = make(&mut b, "ChunkyImage.mui", &[(MUIA_ChunkyImage_Pixels, px), (MUIA_ChunkyImage_Palette, pal), (MUIA_ChunkyImage_NumColors, 3), (MUIA_ChunkyImage_Modulo, 4), (MUIA_Bitmap_Width, 3), (MUIA_Bitmap_Height, 2), (MUIA_FixWidth, 3), (MUIA_FixHeight, 2), (MUIA_Bitmap_Transparent, 0)]).unwrap();
    let runs = pixels(&planes_of(&mut b, t)[0]);
    assert_eq!(runs.iter().map(|(r, c)| (r.width, r.height, c.clone())).collect::<Vec<_>>(), alloc::vec![(2, 1, alloc::vec![0xFF0000, 0x00FF00]), (2, 1, alloc::vec![0xFF0000, 0x00FF00])], "NEGATIVE CONTROL: the pen-0 pixels are holes");
    // NEGATIVE CONTROL: chunkyimage.c's make_bitmap fails without pixels / with no colours: named, nothing drawn.
    let mut c = machine();
    let bad = make(&mut c, "ChunkyImage.mui", &[(MUIA_Bitmap_Width, 3), (MUIA_Bitmap_Height, 2), (MUIA_ChunkyImage_NumColors, 3)]).unwrap();
    let grp = make(&mut c, "Group.mui", &[(MUIA_Group_Child, bad)]).unwrap();
    let win = make(&mut c, "Window.mui", &[(MUIA_Window_RootObject, grp)]).unwrap();
    let e = set_attr(&mut c, win, MUIA_Window_Open, 1).unwrap_err().0;
    assert!(e.contains("make_bitmap fails"), "{e}");
}

#[test]
fn a_bodychunk_decodes_planar_rows_plain_and_byterun1_the_same() {
    // 8x2, depth 2: row 0 pens 0,1,2,3,0,1,2,3 ; row 1 all 3. plane0 bits, plane1 bits; bpr = 2 (16-bit padded).
    let plane = |bits: [u8; 8]| bits.iter().fold(0u8, |v, b| v << 1 | b);
    let r0 = [plane([0, 1, 0, 1, 0, 1, 0, 1]), 0, plane([0, 0, 1, 1, 0, 0, 1, 1]), 0]; // plane0 (2 bytes), plane1 (2 bytes)
    let r1 = [0xFF, 0, 0xFF, 0];
    let plain: Vec<u8> = r0.iter().chain(r1.iter()).copied().collect();
    // ByteRun1 per row: a literal run of 4 bytes (count 3) for row 0; a repeat for row 1: 0xFF x1, 0 x1, 0xFF x1, 0 x1 as literals too.
    let packed: Vec<u8> = [&[3u8][..], &r0[..], &[3u8][..], &r1[..]].concat();
    let render = |body: &[u8], comp: u32, depth: u32| {
        let mut a = machine();
        let b = bytes(&mut a, body);
        let img = make(&mut a, "Bodychunk.mui", &[(MUIA_Bodychunk_Body, b), (MUIA_Bodychunk_Compression, comp), (MUIA_Bodychunk_Depth, depth), (MUIA_Bitmap_Width, 8), (MUIA_Bitmap_Height, 2), (MUIA_FixWidth, 8), (MUIA_FixHeight, 2)]).unwrap();
        let planes = planes_of(&mut a, img);
        planes.first().map(pixels)
    };
    let pal = |a: &mut Amiga<RecordingGuestUi>| (a.gfx.palette().rgb[0], a.gfx.palette().rgb[1], a.gfx.palette().rgb[2], a.gfx.palette().rgb[3]);
    let mut probe = machine();
    let (c0, c1, c2, c3) = pal(&mut probe);
    let plain_px = render(&plain, 0, 2).unwrap();
    assert_eq!(plain_px.len(), 1);
    assert_eq!(plain_px[0].1, [c0, c1, c2, c3, c0, c1, c2, c3, c3, c3, c3, c3, c3, c3, c3, c3], "pens 0..3 of the screen palette (no SourceColors)");
    assert_eq!(render(&packed, 1, 2).unwrap(), plain_px, "ByteRun1 decodes to the same picture");
    // NEGATIVE CONTROLS: depth 9 and a missing body are bodychunk.c failures; the two bytes of a mask plane are skipped when Masking is 1.
    let mut a = machine();
    let b = bytes(&mut a, &plain);
    let bad = make(&mut a, "Bodychunk.mui", &[(MUIA_Bodychunk_Body, b), (MUIA_Bodychunk_Depth, 9), (MUIA_Bitmap_Width, 8), (MUIA_Bitmap_Height, 2)]).unwrap();
    let grp = make(&mut a, "Group.mui", &[(MUIA_Group_Child, bad)]).unwrap();
    let win = make(&mut a, "Window.mui", &[(MUIA_Window_RootObject, grp)]).unwrap();
    assert!(set_attr(&mut a, win, MUIA_Window_Open, 1).unwrap_err().0.contains("make_bitmap fails"));
}

#[test]
fn a_bitmap_draws_the_program_bitmap_with_its_source_colours_clipped_to_its_box() {
    let mut a = machine();
    let bm = crate::graphics::bitmaps::create(&mut a, 4, 4, 2, None).unwrap();
    { let g = a.gfx.bitmaps.get_mut(&bm).unwrap(); for y in 0..4 { for x in 0..4 { g.bm.set(x, y, ((x + y) % 4) as u8); } } }
    // SourceColors: ULONG red, green, blue per pen (top byte used): pen 0 white, 1 red, 2 green, 3 blue.
    let mut cols = Vec::new();
    for (r, g, b) in [(255u32, 255, 255), (255, 0, 0), (0, 255, 0), (0, 0, 255)] { for v in [r, g, b] { cols.extend((v << 24).to_be_bytes()); } }
    let src = bytes(&mut a, &cols);
    let img = make(&mut a, "Bitmap.mui", &[(MUIA_Bitmap_Bitmap, bm), (MUIA_Bitmap_Width, 4), (MUIA_Bitmap_Height, 4), (MUIA_Bitmap_SourceColors, src), (MUIA_FixWidth, 3), (MUIA_FixHeight, 2)]).unwrap();
    let planes = planes_of(&mut a, img);
    let px = pixels(&planes[0]);
    assert_eq!((px[0].0.width, px[0].0.height), (3, 2), "bitmap.c Draw clips to the object's box (MUIA_FixWidth/Height here)");
    assert_eq!(px[0].1, alloc::vec![0xFFFFFF, 0xFF0000, 0x00FF00, 0xFF0000, 0x00FF00, 0x0000FF], "SourceColors, not the screen palette");
    // NEGATIVE CONTROL: without a BitMap (or a size) bitmap.c draws nothing.
    let mut b = machine();
    let none = make(&mut b, "Bitmap.mui", &[(MUIA_Bitmap_Width, 4), (MUIA_Bitmap_Height, 4)]).unwrap();
    assert!(planes_of(&mut b, none).is_empty());
}

#[test]
fn an_imagedisplay_draws_its_spec_as_theme_roles_glyphs_and_icons_and_refuses_what_it_cannot() {
    let spec_ops = |spec: &str| -> Result<Vec<DrawOp>, String> {
        let mut a = machine();
        let s = cstr(&mut a, spec);
        let img = make(&mut a, "Imagedisplay.mui", &[(MUIA_Imagedisplay_Spec, s), (MUIA_FixWidth, 12), (MUIA_FixHeight, 12)]).unwrap();
        let grp = make(&mut a, "Group.mui", &[(MUIA_Group_Child, img)]).unwrap();
        let win = make(&mut a, "Window.mui", &[(MUIA_Window_RootObject, grp)]).unwrap();
        set_attr(&mut a, win, MUIA_Window_Open, 1).map_err(|e| e.0)?;
        Ok(a.ui.calls.iter().filter_map(|c| match c { UiCall::PresentPlane(_, p) => Some(p.ops.clone()), _ => None }).next().unwrap_or_default())
    };
    assert!(matches!(spec_ops("0:128").unwrap()[..], [DrawOp::Fill { pen_role: PenRole::Background, .. }]), "a standard background is a theme fill");
    assert!(matches!(spec_ops("0:131").unwrap()[..], [DrawOp::Fill { pen_role: PenRole::Fill, .. }]));
    assert!(matches!(spec_ops("2:m5").unwrap()[..], [DrawOp::Fill { pen_role: PenRole::Text, .. }]), "MUI pen 5 is the text pen");
    let up = spec_ops("1:11").unwrap();
    assert!(up.len() >= 6 && up.iter().all(|o| matches!(o, DrawOp::Line { pen_role: PenRole::Text, .. })), "an arrow is lines in the text pen");
    let widths = |ops: &[DrawOp]| ops.iter().map(|o| match o { DrawOp::Line { a, b, .. } => (b.x - a.x).abs(), _ => 0 }).collect::<Vec<_>>();
    assert!(widths(&up).first() < widths(&up).last(), "an up arrow widens downwards");
    assert!(matches!(spec_ops("1:24").unwrap()[..], [DrawOp::Icon { role: IconRole::Disk, .. }]), "MUII_Disk is the host's disk icon");
    assert!(matches!(spec_ops("1:22").unwrap()[..], [DrawOp::Icon { role: IconRole::Drawer, .. }]));
    assert!(!spec_ops("1:15").unwrap().is_empty(), "the check mark is drawn");
    // NEGATIVE CONTROLS: named refusals, never a guessed picture.
    assert!(spec_ops("5:SYS:Prefs/x.png").unwrap_err().contains("type 5"));
    assert!(spec_ops("1:17").unwrap_err().contains("no glyph"), "the cycle image has no glyph here");
    assert!(spec_ops("nonsense").unwrap_err().contains("no type prefix"));
    assert!(spec_ops("2:m9").unwrap_err().contains("outside 0..8"));
}

#[test]
fn a_list_without_a_pool_makes_its_own_and_gives_it_back() {
    let mut a = machine();
    let list = make(&mut a, "List.mui", &[]).unwrap();
    let p = list::pool_of(&mut a, list);
    assert_ne!(p, 0, "the construct hook's A2 is a real pool, not NULL");
    assert!(a.pools.contains_key(&p));
    assert_eq!(list::pool_of(&mut a, list), p, "one pool per list");
    // NEGATIVE CONTROL: a pool the program gave (MUIA_List_Pool) is used as it is and no second one is made.
    let given = a.heap.alloc(16).unwrap();
    let mine = make(&mut a, "List.mui", &[(MUIA_List_Pool, given)]).unwrap();
    assert_eq!(list::pool_of(&mut a, mine), given);
    assert_eq!(a.pools.len(), 1);
    dispose(&mut a, list).unwrap();
    assert!(a.pools.is_empty(), "disposing the list deleted its own pool");
}

#[test]
fn set_cycle_chain_records_the_programs_focus_order_and_refuses_what_is_not_an_object() {
    let mut a = machine();
    let (l1, l2) = (cstr(&mut a, "One"), cstr(&mut a, "Two"));
    let (b1, b2) = (made(&mut a, MUIO_Button, &[l1]), made(&mut a, MUIO_Button, &[l2]));
    let (_app, win) = open_ui(&mut a, &[b1, b2]);
    assert!(a.objects.cycle_chain(win).is_empty(), "control: nothing is recorded until the program sets a chain");
    assert_eq!(method(&mut a, win, MUIM_Window_SetCycleChain, &[b2, b1]).unwrap(), 0);
    assert_eq!(a.objects.cycle_chain(win), [b2, b1], "the program's order, not creation order");
    method(&mut a, win, MUIM_Window_SetCycleChain, &[b1]).unwrap();
    assert_eq!(a.objects.cycle_chain(win), [b1], "a new chain replaces the old one");
    // NEGATIVE CONTROL: a longword that is no MUI object is refused by name and the chain is untouched.
    let e = method(&mut a, win, MUIM_Window_SetCycleChain, &[b2, 0x1234]).unwrap_err().0;
    assert!(e.contains("MUIM_Window_SetCycleChain") && e.contains("0x1234"), "{e}");
    assert_eq!(a.objects.cycle_chain(win), [b1]);
}

#[test]
fn a_listview_passes_the_list_methods_attributes_and_notifications_to_its_list() {
    let mut a = machine();
    let list = make(&mut a, "List.mui", &[]).unwrap();
    let lv = make(&mut a, "Listview.mui", &[(MUIA_Listview_List, list)]).unwrap();
    let (app, _) = open_ui(&mut a, &[lv]);
    let rows: Vec<u32> = ["alpha", "beta", "gamma"].iter().map(|s| cstr(&mut a, s)).collect();
    for &r in &rows { method(&mut a, lv, MUIM_List_InsertSingle, &[r, MUIV_List_Insert_Bottom]).unwrap(); }
    assert!(matches!(kind_of_obj(&a, list), ControlKind::List { ref rows, .. } if rows.len() == 3), "InsertSingle sent to the Listview filled the native list");
    // MUIA_List_Active set on the Listview selects the native row (and a notification asked of the Listview fires on the List's change).
    method(&mut a, lv, MUIM_Notify, &[MUIA_List_Active, MUIV_EveryTime, app, 2, MUIM_Application_ReturnID, 77]).unwrap();
    set_attr(&mut a, lv, MUIA_List_Active, 2).unwrap();
    assert!(matches!(kind_of_obj(&a, list), ControlKind::List { selected: Some(2), .. }));
    assert_eq!((get_attr(&mut a, lv, MUIA_List_Active).unwrap(), get_attr(&mut a, list, MUIA_List_Active).unwrap()), (2, 2));
    assert_eq!(next_id(&mut a, app), 77, "the redirected notification ran");
    method(&mut a, lv, MUIM_List_Clear, &[]).unwrap();
    assert!(matches!(kind_of_obj(&a, list), ControlKind::List { ref rows, selected: None, .. } if rows.is_empty()), "Clear sent to the Listview emptied the List and its selection");
    assert_eq!(get_attr(&mut a, lv, MUIA_List_Entries).unwrap(), 0);
    // NEGATIVE CONTROL: an attribute that is neither the Listview's nor the List's is still refused by name.
    assert!(set_attr(&mut a, lv, MUIA_Slider_Quiet, 1).unwrap_err().0.contains("not implemented"));
}

#[test]
fn set_as_string_formats_the_values_and_sets_the_attribute_to_the_text() {
    let mut a = machine();
    let txt = make(&mut a, "Text.mui", &[]).unwrap();
    let (app, _) = open_ui(&mut a, &[txt]);
    let fmt = zstr(&mut a, "%ld00 KByte");
    assert_eq!(method(&mut a, txt, MUIM_SetAsString, &[MUIA_Text_Contents, fmt, 42]).unwrap(), 1, "TRUE");
    assert_eq!(a.objects.text(txt, MUIA_Text_Contents), "4200 KByte");
    assert!(matches!(kind_of_obj(&a, txt), ControlKind::Label { ref text, .. } if text == "4200 KByte"), "and the native label follows: {:?}", kind_of_obj(&a, txt));
    // A word conversion reads the HIGH word of the long the C varargs pushed (RawDoFmt's data stream), a string conversion a pointer.
    let (f2, name) = (zstr(&mut a, "%s: %d"), zstr(&mut a, "load"));
    method(&mut a, txt, MUIM_SetAsString, &[MUIA_Text_Contents, f2, name, 7 << 16]).unwrap();
    assert_eq!(a.objects.text(txt, MUIA_Text_Contents), "load: 7");
    // Notified like any set.
    method(&mut a, txt, MUIM_Notify, &[MUIA_Text_Contents, MUIV_EveryTime, app, 2, MUIM_Application_ReturnID, 5]).unwrap();
    method(&mut a, txt, MUIM_SetAsString, &[MUIA_Text_Contents, fmt, 1]).unwrap();
    assert_eq!(next_id(&mut a, app), 5);
    // The guest message's length follows the format: attr, format, and one long per 32-bit or padded 16-bit value.
    let msg = a.heap.alloc(32).unwrap();
    a.mem.write_u32(msg + 8, f2).unwrap();
    assert_eq!(methods::arity(&a, MUIM_SetAsString, msg).unwrap(), 2 + 2, "%s (4 bytes) + %d (2 bytes) = 6 bytes = 2 longs");
    a.mem.write_u32(msg + 8, fmt).unwrap();
    assert_eq!(methods::arity(&a, MUIM_SetAsString, msg).unwrap(), 2 + 1);
    let plain = zstr(&mut a, "no values");
    a.mem.write_u32(msg + 8, plain).unwrap();
    assert_eq!(methods::arity(&a, MUIM_SetAsString, msg).unwrap(), 2);
}

#[test]
fn application_input_reports_the_signals_to_wait_on_and_returns_the_next_id() {
    let mut a = machine();
    let l = cstr(&mut a, "Go");
    let button = made(&mut a, MUIO_Button, &[l]);
    let (app, _) = open_ui(&mut a, &[button]);
    let sigs = a.heap.alloc(4).unwrap();
    assert_eq!(method(&mut a, app, MUIM_Application_Input, &[sigs]).unwrap(), 0, "nothing pending");
    let wait = a.mem.read_u32(sigs).unwrap();
    assert_ne!(wait, 0, "the signals the loop must Wait on");
    method(&mut a, app, MUIM_Application_ReturnID, &[9]).unwrap();
    a.mem.write_u32(sigs, 0).unwrap();
    assert_eq!(method(&mut a, app, MUIM_Application_Input, &[sigs]).unwrap(), 9, "the queued ReturnID comes back from Input as from NewInput");
}

/// Open a window over `root` on a machine carrying `metrics`; the controls the host was handed, in order.
fn controls_under(metrics: layout::Metrics, build: impl FnOnce(&mut Amiga<RecordingGuestUi>) -> u32) -> (heirloom_ui_abi::WindowModel, Vec<heirloom_ui_abi::ControlModel>) {
    let mut a = machine();
    a.objects.mui_metrics = metrics;
    let root = build(&mut a);
    let title = cstr(&mut a, "T");
    let win = make(&mut a, "Window.mui", &[(MUIA_Window_Title, title), (MUIA_Window_RootObject, root)]).unwrap();
    set_attr(&mut a, win, MUIA_Window_Open, 1).unwrap();
    let m = a.ui.calls.iter().find_map(|c| if let UiCall::OpenWindow(m) = c { Some(m.clone()) } else { None }).unwrap();
    (m, a.ui.calls.iter().filter_map(|c| if let UiCall::AddControl(_, c) = c { Some(c.clone()) } else { None }).collect())
}

/// Any frame but None makes a Group a framed group (roles::frame_group); the style value does not matter to the layout.
fn text_cell(a: &mut Amiga<RecordingGuestUi>, s: &str) -> u32 {
    let p = zstr(a, s);
    make(a, "Text.mui", &[(MUIA_Text_Contents, p), (MUIA_Frame, MUIV_Frame_Text)]).unwrap()
}

fn titled_group(a: &mut Amiga<RecordingGuestUi>, title: &str, kids: &[u32]) -> u32 {
    let t = zstr(a, title);
    let mut items: Vec<(u32, u32)> = alloc::vec![(MUIA_Frame, MUIV_Frame_Button), (MUIA_FrameTitle, t)];
    items.extend(kids.iter().map(|&k| (MUIA_Group_Child, k)));
    make(a, "Group.mui", &items).unwrap()
}

#[test]
fn a_host_group_keeps_its_contents_below_its_title_band_and_inside_its_padding() {
    let build = |a: &mut Amiga<RecordingGuestUi>| { let t = text_cell(a, "row"); titled_group(a, "Settings", &[t]) };
    let host = layout::Metrics { group_head: 28, group_inset: 12, ..Default::default() };
    let (_, c) = controls_under(host, build);
    let (g, row) = (&c[0], &c[1]);
    assert!(matches!(g.kind, heirloom_ui_abi::ControlKind::Group { .. }));
    assert_eq!(row.rect.top - g.rect.top, 28 + 12, "contents start under the 28 px title band plus the panel's padding");
    assert_eq!(row.rect.left - g.rect.left, 12, "the panel's padding on the left");
    assert_eq!((g.rect.left + g.rect.width) - (row.rect.left + row.rect.width), 12, "and on the right");
    assert_eq!((g.rect.top + g.rect.height) - (row.rect.top + row.rect.height), 12, "and under the contents");
    // NEGATIVE CONTROL: no host profile = Zune's own frame, a one-line title and a 5 px border (the contents sit well above the host's band).
    let (_, z) = controls_under(layout::Metrics::default(), build);
    assert!(z[1].rect.top - z[0].rect.top < 28, "Zune's own title row is not the host's 28 px band: {}", z[1].rect.top - z[0].rect.top);
}

#[test]
fn an_untitled_host_group_has_no_title_band() {
    let host = layout::Metrics { group_head: 28, group_inset: 12, ..Default::default() };
    let (_, c) = controls_under(host, |a| {
        let t = text_cell(a, "row");
        make(a, "Group.mui", &[(MUIA_Frame, MUIV_Frame_Button), (MUIA_Group_Child, t)]).unwrap()
    });
    assert_eq!(c[1].rect.top - c[0].rect.top, 12);
}

fn wide(s: &str) -> Option<i32> { Some(20 * s.chars().count() as i32) }

#[test]
fn the_host_measures_the_text_the_layout_is_sized_by() {
    let build = |a: &mut Amiga<RecordingGuestUi>| text_cell(a, "abc");
    let (m1, c1) = controls_under(layout::Metrics { text_w: Some(wide), ..Default::default() }, build);
    assert_eq!(c1[0].rect.width, 3 * 20 + 8, "the cell is the host's width of the text plus the frame");
    // NEGATIVE CONTROL: with no measurer, or one that cannot measure this text, the screen font decides.
    let (_, unmeasurable) = controls_under(layout::Metrics { text_w: Some(|_| None), ..Default::default() }, build);
    assert_eq!(unmeasurable[0].rect.width, controls_under(layout::Metrics::default(), build).1[0].rect.width);
    let (m0, c0) = controls_under(layout::Metrics::default(), build);
    assert_ne!(c0[0].rect.width, c1[0].rect.width);
    assert_eq!(m1.rect.width - m0.rect.width, c1[0].rect.width - c0[0].rect.width, "the window grows with the cell");
}

#[test]
fn a_label_has_room_for_the_gap_the_host_draws_before_its_control() {
    let build = |a: &mut Amiga<RecordingGuestUi>| {
        let lab = { let p = a.heap.alloc(8).unwrap(); let s = cstr(a, "Agent"); a.mem.write_u32(p, s).unwrap(); a.mem.write_u32(p + 4, 0).unwrap(); make::make(a, MUIO_Label, p).unwrap() };
        let txt = text_cell(a, "hello");
        make(a, "Group.mui", &[(MUIA_Group_Columns, 2), (MUIA_Group_Child, lab), (MUIA_Group_Child, txt)]).unwrap()
    };
    let (_, base) = controls_under(layout::Metrics { text_w: Some(wide), ..Default::default() }, build);
    let (_, gapped) = controls_under(layout::Metrics { text_w: Some(wide), label_gap: 12, ..Default::default() }, build);
    assert_eq!(gapped[0].rect.left - base[0].rect.left, 12, "the label cell is the measured text plus the host's gap");
    assert_eq!(base[0].label, "Agent");
    assert_eq!(gapped[0].label, "Agent", "the label still belongs to its control");
}

#[test]
fn the_host_spacing_and_window_inset_apply_unless_the_guest_names_its_own() {
    let build = |a: &mut Amiga<RecordingGuestUi>| { let (x, y) = (text_cell(a, "a"), text_cell(a, "b")); make(a, "Group.mui", &[(MUIA_Group_Child, x), (MUIA_Group_Child, y)]).unwrap() };
    let (_, zune) = controls_under(layout::Metrics::default(), build);
    assert_eq!(zune[1].rect.top - (zune[0].rect.top + zune[0].rect.height), 3, "Zune's 3 px");
    let host = layout::Metrics { spacing: (12, 8), win_pad: 16, ..Default::default() };
    let (_, h) = controls_under(host, build);
    assert_eq!(h[1].rect.top - (h[0].rect.top + h[0].rect.height), 8);
    assert_eq!(h[0].rect.left - zune[0].rect.left, 16 - layout::WIN_PAD.0, "the window inset");
    let side = |a: &mut Amiga<RecordingGuestUi>| { let (x, y) = (text_cell(a, "a"), text_cell(a, "b")); make(a, "Group.mui", &[(MUIA_Group_Horiz, 1), (MUIA_Group_Child, x), (MUIA_Group_Child, y)]).unwrap() };
    let (_, row) = controls_under(host, side);
    assert_eq!(row[1].rect.left - (row[0].rect.left + row[0].rect.width), 12, "the host's horizontal gap");
    let zrow = controls_under(layout::Metrics::default(), side).1;
    assert_eq!(zrow[1].rect.left - (zrow[0].rect.left + zrow[0].rect.width), 6, "Zune's 6 px");
    // the guest's own spacing wins over the host's default
    let (_, own) = controls_under(host, |a| { let (x, y) = (text_cell(a, "a"), text_cell(a, "b")); make(a, "Group.mui", &[(MUIA_Group_VertSpacing, 1), (MUIA_Group_Child, x), (MUIA_Group_Child, y)]).unwrap() });
    assert_eq!(own[1].rect.top - (own[0].rect.top + own[0].rect.height), 1);
}

#[test]
fn a_text_longer_than_256_bytes_is_stored_whole() {
    // aros-About's credits text is far longer than the bounded string read allowed: its Text object failed with "unterminated within 256 bytes"
    // and took its parent group, window and the application with it.
    let mut a = machine();
    let text = "Zune ".repeat(200); // 1000 bytes
    let p = zstr(&mut a, &text);
    let t = make(&mut a, "Text.mui", &[(MUIA_Text_Contents, p)]).unwrap();
    assert_eq!(get_attr(&mut a, t, MUIA_Text_Contents).map(|v| a.mem.read_cstr_long(v).unwrap()).unwrap(), text, "all 1000 bytes");
    // NEGATIVE CONTROL: a text with no NUL before the end of guest RAM is still a named refusal, not a hang.
    let end = (a.mem.len() as u32) - 8;
    a.mem.write_bytes(end, &[b'x'; 8]).unwrap();
    let e = make(&mut a, "Text.mui", &[(MUIA_Text_Contents, end)]).unwrap_err().0;
    assert!(e.contains("MUI string attribute") && e.contains("unterminated within 8 bytes"), "{e}");
}
