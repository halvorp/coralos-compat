//! MUI object lifecycle and attributes: creation from a tag list (children adopted), SetAttrs / GetAttr, disposal.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use super::{ids::*, notify, win};
use crate::boopsi::{self, Ext, Kind};
use crate::{machine::Amiga, ports, Refusal};
use guestui_abi::GuestUi;

fn kind_of<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Result<Kind, Refusal> {
    a.objects.map.get(&obj).map(|o| o.kind).ok_or_else(|| Refusal(format!("{obj:#x} is not a MUI object")))
}

/// A Text object that is a MUIO_Label: the label text is its contents.
pub fn create_label<U: GuestUi>(a: &mut Amiga<U>, text: String) -> Result<u32, Refusal> {
    let obj = boopsi::alloc_object(a, Kind::Text)?;
    let o = a.objects.map.get_mut(&obj).ok_or_else(|| Refusal("label vanished".into()))?;
    o.strs.insert(MUIA_TEXT_CONTENTS, text);
    o.ext = Ext::Text { ctl: None, label: true };
    Ok(obj)
}

/// Build an object of `kind` from `tags`. On any failure the half-built tree AND the not-yet-adopted children named
/// in the tags are disposed (MUI semantics: a NULL child makes the parent NULL, nothing leaks).
pub fn create<U: GuestUi>(a: &mut Amiga<U>, kind: Kind, tags: &[(u32, u32)]) -> Result<u32, Refusal> {
    let obj = boopsi::alloc_object(a, kind)?;
    match init(a, obj, kind, tags) {
        Ok(()) => Ok(obj),
        Err(e) => {
            let _ = dispose_tree(a, obj);
            for &(tag, data) in tags {
                if super::ids::lookup(kind, tag).is_some_and(|d| d.ty == Ty::Adopt) && a.objects.map.get(&data).is_some_and(|c| c.parent.is_none()) {
                    let _ = dispose_tree(a, data);
                }
            }
            Err(e)
        }
    }
}

fn init<U: GuestUi>(a: &mut Amiga<U>, obj: u32, kind: Kind, tags: &[(u32, u32)]) -> Result<(), Refusal> {
    if kind == Kind::Application {
        let bit = ports::alloc_signal_bit(a, -1)?.ok_or_else(|| Refusal("MUI Application: no free signal bit".into()))?;
        if let Some(o) = a.objects.map.get_mut(&obj) { o.ext = Ext::App { bit, pending: Default::default() }; }
        a.sched.app_task.insert(obj, a.sched.cur); // the task a ReturnID signals: the one that created the Application
    }
    for &(tag, data) in tags {
        let def = lookup(kind, tag).ok_or_else(|| Refusal(format!("MUI {kind:?} class: attribute {} is not implemented", attr_name(tag))))?;
        match def.ty {
            Ty::Num => { store_num(a, obj, tag, data); }
            Ty::Str => store_str(a, obj, tag, data)?,
            Ty::Adopt => adopt(a, obj, tag, data)?,
        }
    }
    Ok(())
}

fn store_num<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, v: u32) { if let Some(o) = a.objects.map.get_mut(&obj) { o.attrs.insert(attr, v); } }

fn store_str<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, ptr: u32) -> Result<(), Refusal> {
    let s = if ptr == 0 { String::new() } else { a.mem.read_cstr(ptr).map_err(Refusal::mem("MUI string attribute"))? };
    if let Some(o) = a.objects.map.get_mut(&obj) { o.strs.insert(attr, s); o.attrs.insert(attr, ptr); }
    Ok(())
}

/// The parent takes ownership of `child` (tag `tag`), if a child of that kind belongs there.
fn adopt<U: GuestUi>(a: &mut Amiga<U>, parent: u32, tag: u32, child: u32) -> Result<(), Refusal> {
    let pk = kind_of(a, parent)?;
    let name = attr_name(tag);
    if child == 0 { return Err(Refusal(format!("{name}: the child object is NULL (its creation failed)"))); }
    let (ck, owned) = a.objects.map.get(&child).map(|c| (c.kind, c.parent.is_some()))
        .ok_or_else(|| Refusal(format!("{name}: {child:#x} is not a MUI object")))?;
    if owned { return Err(Refusal(format!("{name}: {child:#x} already has a parent"))); }
    let ok = match (tag, pk, ck) {
        (MUIA_APP_WINDOW, _, c) => c == Kind::Window,
        (MUIA_APP_MENUSTRIP, _, c) => c == Kind::Menustrip,
        (MUIA_WIN_ROOT_OBJECT | MUIA_GROUP_CHILD, _, c) => matches!(c, Kind::Group | Kind::Text),
        (MUIA_FAMILY_CHILD, Kind::Menustrip, c) => c == Kind::Menu,
        (MUIA_FAMILY_CHILD, Kind::Menu | Kind::Menuitem, c) => c == Kind::Menuitem,
        _ => false,
    };
    if !ok { return Err(Refusal(format!("{name}: a {ck:?} object cannot be a child of a {pk:?}"))); }
    if let Some(c) = a.objects.map.get_mut(&child) { c.parent = Some(parent); }
    if let Some(p) = a.objects.map.get_mut(&parent) { p.children.push(child); }
    Ok(())
}

/// SetAttrs on a live object: store, apply the side effect on the native UI, then fire the notifications.
pub fn set_attr<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, val: u32) -> Result<(), Refusal> {
    let kind = kind_of(a, obj)?;
    let def = lookup(kind, attr).ok_or_else(|| Refusal(format!("MUI {kind:?} class: attribute {} is not implemented", attr_name(attr))))?;
    match def.ty {
        Ty::Adopt => return Err(Refusal(format!("{}: can be given only when the object is created", def.name))),
        Ty::Str => {
            if kind == Kind::Window && win::is_open(a, obj) { return Err(Refusal(format!("{}: changing it while the window is open has no GuestUi verb", def.name))); }
            store_str(a, obj, attr, val)?;
            if (kind, attr) == (Kind::Text, MUIA_TEXT_CONTENTS) { win::push_text(a, obj)?; }
        }
        Ty::Num if (kind, attr) == (Kind::Window, MUIA_WIN_OPEN) => {
            win::set_open(a, obj, val != 0)?;
            store_num(a, obj, attr, (val != 0) as u32);
        }
        Ty::Num => store_num(a, obj, attr, val),
    }
    notify::trigger(a, obj, attr, val)
}

/// GetAttr: the stored value (0 when never set).
pub fn get_attr<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32) -> Result<u32, Refusal> {
    let kind = kind_of(a, obj)?;
    lookup(kind, attr).ok_or_else(|| Refusal(format!("MUI {kind:?} class: attribute {} is not implemented", attr_name(attr))))?;
    Ok(a.objects.map.get(&obj).and_then(|o| o.attrs.get(&attr).copied()).unwrap_or(0))
}

/// MUI_DisposeObject / DisposeObject: only a root; the tree goes with it.
pub fn dispose<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    match a.objects.map.get(&obj) {
        None => Err(Refusal(format!("DisposeObject: {obj:#x} is not a MUI object (or already disposed)"))),
        Some(o) if o.parent.is_some() => Err(Refusal(format!("DisposeObject: {obj:#x} is owned by a parent; dispose the root"))),
        Some(_) => dispose_tree(a, obj),
    }
}

fn dispose_tree<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    let Some(o) = a.objects.map.get(&obj) else { return Ok(()) };
    let (kind, children): (Kind, Vec<u32>) = (o.kind, o.children.clone());
    let bit = if let Ext::App { bit, .. } = o.ext { Some(bit) } else { None };
    let mut first: Result<(), Refusal> = Ok(());
    if kind == Kind::Window { first = win::close_if_open(a, obj); }
    for c in children { let r = dispose_tree(a, c); if first.is_ok() { first = r; } }
    if let Some(b) = bit { let r = ports::release_signal_bit(a, b as i32); if first.is_ok() { first = r; } }
    let r = boopsi::free_object(a, obj);
    if first.is_ok() { r } else { first }
}
