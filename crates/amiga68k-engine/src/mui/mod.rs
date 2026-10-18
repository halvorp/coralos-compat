//! muimaster.library (MUI) translated natively onto GuestUi (docs/MUI-NATIVE.md). The object model is host-side
//! ([`crate::boopsi`]); windows and controls become GuestUi calls only when a window opens.
pub mod ids;
mod app;
mod layout;
mod menu;
mod notify;
mod object;
mod win;

use alloc::string::String;
use alloc::format;
use crate::boopsi::{Kind, CLASSES};
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, run::Delivered, tags, Refusal};
use guestui_abi::{GuestUi, GuestUiEvent};
use ids::*;
use retro_m68k::Cpu;

pub use object::{dispose, get_attr, set_attr};

const LVO_MUI_NEW_OBJECT_A: i32 = -30;
const LVO_MUI_DISPOSE_OBJECT: i32 = -36;
const LVO_MUI_MAKE_OBJECT_A: i32 = -120;

/// muimaster.library with the functions the engine bridges; every other vector stays a named refusal.
pub fn library<U: GuestUi>(base: u32) -> LibraryBuilder<U> {
    LibraryBuilder::new("muimaster.library", base)
        .func(LVO_MUI_NEW_OBJECT_A, "MUI_NewObjectA", mui_new_object_a::<U>)
        .func(LVO_MUI_DISPOSE_OBJECT, "MUI_DisposeObject", mui_dispose_object::<U>)
        .func(LVO_MUI_MAKE_OBJECT_A, "MUI_MakeObjectA", mui_make_object_a::<U>)
}

/// Create the object of public class `name` from the tag list at `tags_at`.
pub fn new_by_name<U: GuestUi>(a: &mut Amiga<U>, name: &str, tags_at: u32) -> Result<u32, Refusal> {
    let kind = CLASSES.iter().find(|c| c.0 == name).map(|c| c.1)
        .ok_or_else(|| Refusal(format!("MUI class \"{name}\" is not implemented")))?;
    let t = tags::walk(&a.mem, tags_at)?;
    object::create(a, kind, &t)
}

/// MUI_NewObjectA(a0 = class name, a1 = tags): NULL (with the reason) when it cannot be built.
fn mui_new_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(0)).map_err(Refusal::mem("MUI_NewObjectA class name"))?;
    let obj = new_by_name(a, &name, cpu.areg(1))?;
    cpu.set_d0(obj);
    Ok(format!("[muimaster] MUI_NewObjectA \"{name}\" -> {obj:#x}"))
}

/// MUI_DisposeObject(a0 = object): the whole tree below it goes too.
fn mui_dispose_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    dispose(a, obj)?;
    Ok(format!("[muimaster] MUI_DisposeObject {obj:#x}"))
}

/// MUI_MakeObjectA(d0 = type, a0 = parameter array). Only MUIO_Label so far: a Text object carrying the label.
fn mui_make_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, params) = (cpu.dreg(0), cpu.areg(0));
    if ty != MUIO_LABEL { return Err(Refusal(format!("MUI_MakeObjectA: object type {ty} is not implemented (only MUIO_Label)"))); }
    let text = a.mem.read_cstr(a.mem.read_u32(params).map_err(Refusal::mem("MUIO_Label text"))?).map_err(Refusal::mem("MUIO_Label text"))?;
    let obj = object::create_label(a, text)?;
    cpu.set_d0(obj);
    Ok(format!("[muimaster] MUI_MakeObjectA MUIO_Label -> {obj:#x}"))
}

/// A host or guest method call on `obj`: `args` are the longwords after the method id.
pub fn method<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32, args: &[u32]) -> Result<u32, Refusal> {
    let kind = a.objects.map.get(&obj).ok_or_else(|| Refusal(format!("DoMethod: {obj:#x} is not a MUI object")))?.kind;
    let arg = |i: usize| args.get(i).copied().ok_or_else(|| Refusal(format!("{}: missing argument {i}", method_name(id))));
    match id {
        MUIM_NOTIFY => notify::add(a, obj, args),
        MUIM_SET => { set_attr(a, obj, arg(0)?, arg(1)?)?; Ok(0) }
        MUIM_APP_NEW_INPUT if kind == Kind::Application => app::new_input(a, obj, arg(0)?),
        MUIM_APP_RETURN_ID if kind == Kind::Application => { app::return_id(a, obj, arg(0)?)?; Ok(0) }
        _ => Err(Refusal(format!("MUI {kind:?} class: {} is not implemented", method_name(id)))),
    }
}

/// Offer a native event to the MUI objects. None = not one of ours (the caller keeps looking).
pub fn try_deliver<U: GuestUi>(a: &mut Amiga<U>, ev: &GuestUiEvent) -> Result<Option<Delivered>, Refusal> {
    match ev {
        GuestUiEvent::CloseRequested { win, .. } => match win::find_by_native(a, *win) {
            Some(w) => {
                set_attr(a, w, MUIA_WIN_CLOSE_REQUEST, 1)?;
                if let Some(o) = a.objects.map.get_mut(&w) { o.attrs.insert(MUIA_WIN_CLOSE_REQUEST, 0); } // MUI resets it after notifying
                Ok(Some(Delivered::Handled))
            }
            None => Ok(None),
        },
        GuestUiEvent::MenuCommand { token, .. } => match menu::item_action(a, token.0 as u32) {
            Some((app, user)) => { set_attr(a, app, MUIA_APP_MENU_ACTION, user)?; Ok(Some(Delivered::Handled)) }
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests;
