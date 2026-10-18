//! The Window class: MUIA_Window_Open opens a native window with the laid-out controls and the application's menu.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use super::{ids::*, layout, menu, notify::root};
use crate::boopsi::{Ext, Kind};
use crate::{machine::Amiga, Refusal};
use guestui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestRect, GuestUi, UiObjectRef, WindowModel, WindowRole};

/// The chrome window.rs gives an Intuition window with a title bar (borders 4/15/4/4), so a MUI window of the same
/// content is the same size as the GadTools one. Controls are offset by the left/top border.
const BORDER_W: i32 = 8;
const BORDER_H: i32 = 19;
const BORDER_LEFT: i32 = 4;
const BORDER_TOP: i32 = 15;

fn native_of<U: GuestUi>(a: &Amiga<U>, w: u32) -> Option<UiObjectRef> {
    match a.objects.map.get(&w).map(|o| &o.ext) { Some(Ext::Win { native }) => *native, _ => None }
}

pub fn is_open<U: GuestUi>(a: &Amiga<U>, w: u32) -> bool { native_of(a, w).is_some() }

/// The MUI window whose native window is `n`.
pub fn find_by_native<U: GuestUi>(a: &Amiga<U>, n: UiObjectRef) -> Option<u32> {
    a.objects.map.iter().find(|(_, o)| matches!(o.ext, Ext::Win { native: Some(x) } if x == n)).map(|(&k, _)| k)
}

pub fn set_open<U: GuestUi>(a: &mut Amiga<U>, w: u32, open: bool) -> Result<(), Refusal> {
    match (open, is_open(a, w)) {
        (true, false) => open_window(a, w),
        (false, true) => close_if_open(a, w),
        _ => Ok(()),
    }
}

fn open_window<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    let root_obj = a.objects.map.get(&w).and_then(|o| o.children.first().copied())
        .ok_or_else(|| Refusal("MUIA_Window_Open: the window has no MUIA_Window_RootObject".into()))?;
    let (iw, ih, placed) = layout::compute(&a.objects, root_obj);
    let title = a.objects.map.get(&w).and_then(|o| o.strs.get(&MUIA_WIN_TITLE).cloned()).unwrap_or_default();
    // Placement is the native window manager's job: the engine asks for 0,0.
    let model = WindowModel { title, rect: GuestRect { left: 0, top: 0, width: iw + BORDER_W, height: ih + BORDER_H },
        role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(w as u64) };
    let native = a.ui.open_window(model).map_err(Refusal::ui)?;
    match fill(a, w, native, &placed) {
        Ok(()) => { if let Some(o) = a.objects.map.get_mut(&w) { o.ext = Ext::Win { native: Some(native) }; } Ok(()) }
        Err(e) => { let _ = a.ui.close_window(native); Err(e) }
    }
}

fn fill<U: GuestUi>(a: &mut Amiga<U>, w: u32, native: UiObjectRef, placed: &[layout::Placed]) -> Result<(), Refusal> {
    let mut made: Vec<(u32, UiObjectRef)> = Vec::new();
    for p in placed {
        let o = a.objects.map.get(&p.obj).ok_or_else(|| Refusal("layout names a vanished object".into()))?;
        let is_label = matches!(o.ext, Ext::Text { label: true, .. });
        let text = o.strs.get(&MUIA_TEXT_CONTENTS).cloned().unwrap_or_default();
        let framed = !is_label && o.attrs.get(&MUIA_FRAME).copied().unwrap_or(0) != 0;
        let rect = GuestRect { left: p.rect.left + BORDER_LEFT, top: p.rect.top + BORDER_TOP, ..p.rect };
        let ctl = a.ui.add_control(native, ControlModel { kind: ControlKind::Label { text, framed }, label: p.label.clone(), rect, enabled: true, token: CallbackToken(p.obj as u64) }).map_err(Refusal::ui)?;
        made.push((p.obj, ctl));
    }
    for (obj, ctl) in made { if let Some(o) = a.objects.map.get_mut(&obj) { o.ext = Ext::Text { ctl: Some((native, ctl)), label: matches!(o.ext, Ext::Text { label: true, .. }) }; } }
    if let Some(strip) = a.objects.map.get(&root(a, w)).and_then(|app| app.children.iter().copied().find(|c| a.objects.map.get(c).is_some_and(|o| o.kind == Kind::Menustrip))) {
        let model = menu::model(&a.objects, strip);
        a.ui.set_menu_strip(native, model).map_err(Refusal::ui)?;
    }
    Ok(())
}

/// Close the native window (its controls go with it) if it is open.
pub fn close_if_open<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    let Some(native) = native_of(a, w) else { return Ok(()) };
    if let Some(o) = a.objects.map.get_mut(&w) { o.ext = Ext::Win { native: None }; }
    let mut stack: Vec<u32> = a.objects.map.get(&w).map(|o| o.children.clone()).unwrap_or_default();
    while let Some(c) = stack.pop() {
        if let Some(o) = a.objects.map.get_mut(&c) {
            if let Ext::Text { ctl, .. } = &mut o.ext { *ctl = None; }
            stack.extend(o.children.iter().copied());
        }
    }
    a.ui.close_window(native).map_err(Refusal::ui)
}

/// A Text object's contents changed: update its native control, if the window is open.
pub fn push_text<U: GuestUi>(a: &mut Amiga<U>, t: u32) -> Result<(), Refusal> {
    let Some(o) = a.objects.map.get(&t) else { return Err(Refusal(format!("{t:#x} is not a MUI object"))) };
    let Ext::Text { ctl: Some((native, ctl)), .. } = o.ext else { return Ok(()) };
    let text: String = o.strs.get(&MUIA_TEXT_CONTENTS).cloned().unwrap_or_default();
    a.ui.set_control(native, ctl, ControlValue::Text(text)).map_err(Refusal::ui)
}
