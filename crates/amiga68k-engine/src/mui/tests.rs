//! MUI object model: refusals are named, a failed build leaks nothing, labels pair with their text.
use super::{ids::*, layout, new_by_name, dispose, set_attr, get_attr, method};
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
    assert_eq!(make(&mut a, "Rectangle.mui", &[]).unwrap_err().0, "MUI class \"Rectangle.mui\" is not implemented");
    let before = a.heap.live_blocks();
    let e = make(&mut a, "Text.mui", &[(MUIA_TEXT_CONTENTS, 0), (0x8042_0001, 7)]).unwrap_err().0;
    assert_eq!(e, "MUI Text class: attribute 0x80420001 is not implemented");
    assert_eq!(a.heap.live_blocks(), before + 1, "the half-built Text was freed (only the tag list remains)");
    assert!(a.objects.map.is_empty());
    // CONTROL: the same build without the bad tag succeeds.
    assert!(make(&mut a, "Text.mui", &[(MUIA_TEXT_CONTENTS, 0)]).is_ok());
}

#[test]
fn a_null_child_fails_the_parent_and_disposes_its_siblings() {
    let mut a = machine();
    let good = make(&mut a, "Text.mui", &[]).unwrap();
    let e = make(&mut a, "Group.mui", &[(MUIA_GROUP_CHILD, good), (MUIA_GROUP_CHILD, 0)]).unwrap_err().0;
    assert!(e.contains("MUIA_Group_Child: the child object is NULL"), "{e}");
    assert!(a.objects.map.is_empty(), "the good child went too: {:?}", a.objects.map.keys().collect::<Vec<_>>());
    // A kind that does not belong is refused by name too.
    let t = make(&mut a, "Text.mui", &[]).unwrap();
    let e = make(&mut a, "Application.mui", &[(MUIA_APP_WINDOW, t)]).unwrap_err().0;
    assert!(e.contains("a Text object cannot be a child of a Application"), "{e}");
}

#[test]
fn set_and_get_store_values_notify_fires_and_dispose_needs_the_root() {
    let mut a = machine();
    let win = make(&mut a, "Window.mui", &[]).unwrap();
    let app = make(&mut a, "Application.mui", &[(MUIA_APP_WINDOW, win)]).unwrap();
    // Notify: setting CloseRequest=TRUE queues ReturnID(Quit) on the app.
    method(&mut a, win, MUIM_NOTIFY, &[MUIA_WIN_CLOSE_REQUEST, 1, app, 2, MUIM_APP_RETURN_ID, RETURN_ID_QUIT]).unwrap();
    set_attr(&mut a, win, MUIA_WIN_CLOSE_REQUEST, 0).unwrap(); // NEGATIVE CONTROL: trigger value 1 does not match 0
    assert_eq!(method(&mut a, app, MUIM_APP_NEW_INPUT, &[0]).unwrap(), 0);
    set_attr(&mut a, win, MUIA_WIN_CLOSE_REQUEST, 1).unwrap();
    assert_eq!(get_attr(&mut a, win, MUIA_WIN_CLOSE_REQUEST).unwrap(), 1);
    assert_eq!(method(&mut a, app, MUIM_APP_NEW_INPUT, &[0]).unwrap(), RETURN_ID_QUIT);
    assert!(dispose(&mut a, win).unwrap_err().0.contains("owned by a parent"));
    let e = method(&mut a, app, MUIM_APP_OPEN_CONFIG_WINDOW, &[0]).unwrap_err().0;
    assert_eq!(e, "MUI Application class: MUIM_Application_OpenConfigWindow is not implemented");
    dispose(&mut a, app).unwrap();
    assert!(a.objects.map.is_empty());
    assert!(dispose(&mut a, app).unwrap_err().0.contains("not a MUI object"), "double dispose is refused");
}

#[test]
fn a_label_before_a_text_is_that_texts_label_and_a_window_opens_with_the_sized_content() {
    let mut a = machine();
    let lab = super::object::create_label(&mut a, "Agent".into()).unwrap();
    let hello = cstr(&mut a, "hello");
    let txt = make(&mut a, "Text.mui", &[(MUIA_TEXT_CONTENTS, hello), (MUIA_FRAME, 3)]).unwrap();
    let grp = make(&mut a, "Group.mui", &[(MUIA_GROUP_COLUMNS, 2), (MUIA_GROUP_CHILD, lab), (MUIA_GROUP_CHILD, txt)]).unwrap();
    let title = cstr(&mut a, "T");
    let win = make(&mut a, "Window.mui", &[(MUIA_WIN_TITLE, title), (MUIA_WIN_ROOT_OBJECT, grp)]).unwrap();
    let (w, h, placed) = layout::compute(&a.objects, grp);
    assert_eq!(placed.len(), 1, "the label is not a control of its own");
    assert_eq!((placed[0].label.as_str(), placed[0].obj), ("Agent", txt));
    assert_eq!((w, h), (4 + 48 + 4 + 48 + 4, 22), "label cell 6*8, text cell 6*8, one 14px row, 4px margins");
    set_attr(&mut a, win, MUIA_WIN_OPEN, 1).unwrap();
    assert!(matches!(&a.ui.calls[0], UiCall::OpenWindow(m) if m.title == "T"));
    // A title change while open has no GuestUi verb: refused by name, the title is untouched.
    assert!(set_attr(&mut a, win, MUIA_WIN_TITLE, title).unwrap_err().0.contains("no GuestUi verb"));
    // A window with no root object cannot open.
    let bare = make(&mut a, "Window.mui", &[]).unwrap();
    assert!(set_attr(&mut a, bare, MUIA_WIN_OPEN, 1).unwrap_err().0.contains("no MUIA_Window_RootObject"));
    assert_eq!(get_attr(&mut a, bare, MUIA_WIN_OPEN).unwrap(), 0);
}
