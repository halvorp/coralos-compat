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
