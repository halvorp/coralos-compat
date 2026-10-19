//! The Window class: `MUIA_Window_Open` opens a native window whose controls are the projection of the object tree
//! ([`super::layout`] decides where, [`super::roles`] what), kept in step by the one generic registry
//! ([`retro_translate::Projections`]): a changed attribute pushes only the difference, a changed shape is a new generation, and an
//! intent aimed at a retired control is refused by name.
use alloc::vec::Vec;
use alloc::format;
use super::classes::Kind;
use super::ids_gen::*;
use super::layout::{self, Elem};
use super::{menu, object};
use crate::boopsi::Ext;
use crate::graphics::fonts;
use crate::{machine::Amiga, Refusal};
use guestui_abi::{CallbackToken, ControlKind, ControlModel, GuestRect, GuestUi, UiObjectRef, WindowModel, WindowRole};

/// The chrome window.rs gives an Intuition window with a title bar (borders 4/15/4/4), so a MUI window of the same
/// content is the same size as the GadTools one. Controls are offset by the left/top border.
const BORDER_W: i32 = 8;
const BORDER_H: i32 = 19;
const BORDER_LEFT: i32 = 4;
const BORDER_TOP: i32 = 15;

fn native_of<U: GuestUi>(a: &Amiga<U>, w: u32) -> Option<UiObjectRef> {
    match a.objects.map.get(&w).map(|o| &o.ext) { Some(Ext::Win { native, .. }) => *native, _ => None }
}

pub fn is_open<U: GuestUi>(a: &Amiga<U>, w: u32) -> bool { native_of(a, w).is_some() }

/// The MUI window whose native window is `n`.
pub fn find_by_native<U: GuestUi>(a: &Amiga<U>, n: UiObjectRef) -> Option<u32> {
    a.objects.map.iter().find(|(_, o)| matches!(o.ext, Ext::Win { native: Some(x), .. } if x == n)).map(|(&k, _)| k)
}

pub fn set_open<U: GuestUi>(a: &mut Amiga<U>, w: u32, open: bool) -> Result<(), Refusal> {
    match (open, is_open(a, w)) {
        (true, false) => open_window(a, w),
        (false, true) => close_if_open(a, w),
        _ => Ok(()),
    }
}

fn root_of<U: GuestUi>(a: &Amiga<U>, w: u32) -> Option<u32> { a.objects.children(w).iter().copied().find(|&c| a.objects.kind(c).is_some_and(|k| k.is_area())) }

/// The Layout of the window's tree for a content area of at least `avail`.
fn lay_out<U: GuestUi>(a: &mut Amiga<U>, w: u32, avail: (i32, i32)) -> Result<layout::Layout, Refusal> {
    let root = root_of(a, w).ok_or_else(|| Refusal("MUIA_Window_Open: the window has no MUIA_Window_RootObject".into()))?;
    fonts::install_rom_font(a)?; // a MUI program may never have opened graphics.library: the screen font is installed on first need
    let tf = fonts::default_font(a)?;
    let font = fonts::font_at(a, tf)?;
    Ok(layout::compute(&a.objects, root, &font, avail, a.objects.mui_metrics))
}

fn model_of(e: &Elem) -> ControlModel {
    let rect = GuestRect { left: e.rect.left + BORDER_LEFT, top: e.rect.top + BORDER_TOP, ..e.rect };
    ControlModel { kind: e.kind.clone(), label: e.label.clone(), rect, enabled: e.enabled, token: CallbackToken(e.obj as u64) }
}

/// The title the native window shows: MUIA_Window_Title, and for an untitled window the program's own name (what Intuition's
/// window.rs / intuition.rs do for an untitled window: `gfx.program_name`, set by `start_hunk_as`), so it never shows empty.
fn shown_title<U: GuestUi>(a: &Amiga<U>, w: u32) -> alloc::string::String {
    let t = a.objects.text(w, MUIA_Window_Title);
    if t.is_empty() { a.gfx.program_name.clone() } else { alloc::string::String::from(t) }
}

fn open_window<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    let l = lay_out(a, w, (0, 0))?;
    let title = shown_title(a, w);
    // Placement is the native window manager's job: the engine asks for 0,0.
    let model = WindowModel { title, rect: GuestRect { left: 0, top: 0, width: l.width + BORDER_W, height: l.height + BORDER_H },
        role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(w as u64) };
    let native = a.ui.open_window(model).map_err(Refusal::ui)?;
    if let Some(o) = a.objects.map.get_mut(&w) { o.ext = Ext::Win { native: Some(native), content: (l.width, l.height) }; }
    let shown = project(a, w, native, &l).and_then(|()| menu_strip(a, w, native));
    if let Err(e) = shown { let _ = close_if_open(a, w); return Err(e); }
    Ok(())
}

fn menu_strip<U: GuestUi>(a: &mut Amiga<U>, w: u32, native: UiObjectRef) -> Result<(), Refusal> {
    let own = a.objects.children(w).iter().copied().find(|&c| a.objects.kind(c) == Some(Kind::Menustrip));
    let app = object::ancestor(&a.objects, w, Kind::Application);
    let strip = own.or_else(|| app.and_then(|p| a.objects.children(p).iter().copied().find(|&c| a.objects.kind(c) == Some(Kind::Menustrip))));
    if let Some(s) = strip { let model = menu::model(&a.objects, s); a.ui.set_menu_strip(native, model).map_err(Refusal::ui)?; }
    Ok(())
}

/// The stand-in for a control the host refused to draw: a framed label that NAMES it and shows its value (a Gauge reads "42%").
/// Every host draws a label, so the window keeps all its other controls and the user still sees the state.
pub fn degrade(m: &ControlModel) -> ControlModel {
    let text = match &m.kind {
        ControlKind::Gauge { value, max } => format!("{}%", u64::from(*value) * 100 / u64::from((*max).max(1))),
        k => format!("({} not drawn)", format!("{k:?}").split([' ', '{', '(']).next().unwrap_or("control")),
    };
    ControlModel { kind: ControlKind::Label { text, framed: true }, ..m.clone() }
}

/// Make the host's controls match the layout: new and changed ones are synced, ones no longer wanted are retired. A control the host
/// refuses is NEVER fatal for the window: it is recorded by name in `a.refused`, degraded to its [`degrade`] label and the rest goes on.
fn project<U: GuestUi>(a: &mut Amiga<U>, _w: u32, native: UiObjectRef, l: &layout::Layout) -> Result<(), Refusal> {
    let wanted: Vec<u64> = l.elems.iter().map(|e| e.obj as u64).collect();
    let stale: Vec<u64> = a.objects.proj.iter().filter(|(t, p)| p.window == native && !wanted.contains(t)).map(|(t, _)| t).collect();
    for t in stale { a.objects.degraded.remove(&t); a.objects.proj.retire(&mut a.ui, t).map_err(Refusal::ui)?; }
    for e in &l.elems {
        let token = e.obj as u64;
        let want = model_of(e);
        if a.objects.degraded.contains(&token) {
            let shown = degrade(&want);
            // A label's text is not a pushed state: a changed value is a retire + add.
            if a.objects.proj.get(token).is_some_and(|p| p.model.kind != shown.kind) { let _ = a.objects.proj.retire(&mut a.ui, token); }
            if let Err(err) = a.objects.proj.sync(&mut a.ui, Some(native), token, e.obj, Some(shown)) {
                a.refused.push(format!("MUI control {token:#x}: even its stand-in label was refused: {}", Refusal::ui(err).0));
            }
            continue;
        }
        if let Err(err) = a.objects.proj.sync(&mut a.ui, Some(native), token, e.obj, Some(want.clone())) {
            a.refused.push(format!("MUI control {token:#x}: the host refused to draw it ({}): {}; shown as a label instead",
                format!("{:?}", want.kind).split([' ', '{', '(']).next().unwrap_or("control"), Refusal::ui(err).0));
            a.objects.degraded.insert(token);
            if let Err(err) = a.objects.proj.sync(&mut a.ui, Some(native), token, e.obj, Some(degrade(&want))) {
                a.refused.push(format!("MUI control {token:#x}: even its stand-in label was refused: {}", Refusal::ui(err).0));
            }
        }
    }
    Ok(())
}

/// An attribute of `obj` changed: bring the window it is in (if open) in step.
pub fn refresh<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    let Some(w) = (if a.objects.kind(obj) == Some(Kind::Window) { Some(obj) } else { object::ancestor(&a.objects, obj, Kind::Window) }) else { return Ok(()) };
    let Some(native) = native_of(a, w) else { return Ok(()) };
    let content = match a.objects.map.get(&w).map(|o| &o.ext) { Some(Ext::Win { content, .. }) => *content, _ => (0, 0) };
    let l = lay_out(a, w, content)?;
    if let Some(o) = a.objects.map.get_mut(&w) { o.ext = Ext::Win { native: Some(native), content: (l.width.max(content.0), l.height.max(content.1)) }; }
    project(a, w, native, &l)
}

/// The host resized the window: lay out again for the new content area (the spare space goes to what stretches).
pub fn resized<U: GuestUi>(a: &mut Amiga<U>, w: u32, width: i32, height: i32) -> Result<(), Refusal> {
    let Some(native) = native_of(a, w) else { return Ok(()) };
    let content = (width - BORDER_W, height - BORDER_H);
    let l = lay_out(a, w, content)?;
    if let Some(o) = a.objects.map.get_mut(&w) { o.ext = Ext::Win { native: Some(native), content }; }
    project(a, w, native, &l)
}

/// Close the native window (its controls go with it) if it is open.
pub fn close_if_open<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    let Some(native) = native_of(a, w) else { return Ok(()) };
    if let Some(o) = a.objects.map.get_mut(&w) { o.ext = Ext::Win { native: None, content: (0, 0) }; }
    a.objects.proj.forget_where(|p| p.window == native); // the host retires a closed window's controls itself
    a.ui.close_window(native).map_err(Refusal::ui)
}

/// The window's title changed: tell the host if it is open.
pub fn push_title<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    let Some(native) = native_of(a, w) else { return Ok(()) };
    let title = shown_title(a, w);
    a.ui.set_window_title(native, title).map_err(Refusal::ui)
}

/// The route census of window `w` as the projection holds it: how many native controls, by kind name.
pub fn native_controls<U: GuestUi>(a: &Amiga<U>, w: u32) -> Vec<(u32, alloc::string::String)> {
    let Some(native) = native_of(a, w) else { return Vec::new() };
    a.objects.proj.iter().filter(|(_, p)| p.window == native).map(|(t, p)| (t as u32, format!("{:?}", p.model.kind).split([' ', '{', '(']).next().unwrap_or("").into())).collect()
}
