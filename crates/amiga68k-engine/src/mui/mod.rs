//! muimaster.library (MUI) translated natively onto GuestUi (docs/MUI-NATIVE.md, ruling 17). The object model is host-side
//! ([`crate::boopsi`]); a window's tree is projected as native controls through one table ([`crate::translate::mui`]) and the
//! generic registry, and a control's intent comes back as the `set` calls MUI's own input handling makes.
//!
//! * [`classes`] / [`attrs`] — the class tree and what each class accepts (unknown ones are named refusals).
//! * [`object`] / [`methods`] / [`notify`] / [`hooks`] / [`app`] — the object model: attributes, methods, MUIM_Notify, MUIM_CallHook
//!   as guest calls, the Application's input loop.
//! * [`roles`] / [`layout`] / [`win`] — what an object is on screen, where, and the window that holds it.
//! * [`input`] — the intents back; [`make`] — `MUI_MakeObjectA`; [`list`] — the List class; [`menu`] — menu strips.
// MUI's identifiers keep their own spelling (MUIA_Window_Open): they are interface names from mui.h, not Rust names.
#![allow(non_upper_case_globals)]
pub mod app;
pub mod attrs;
pub mod classes;
pub mod hooks;
mod ids_gen;
mod input;
pub mod layout;
pub mod list;
mod make;
mod menu;
mod methods;
mod names;
mod notify;
mod object;
pub mod roles;
pub mod win;

use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, run::Delivered, tags, Refusal};
use guestui_abi::{GuestUi, GuestUiEvent};
use retro_m68k::Cpu;

pub use ids_gen::*;
pub use methods::{method, OM_ADDMEMBER, OM_GET, OM_REMMEMBER};
pub use object::{dispose, get_attr, set_attr, set_attr_quiet};
pub use win::native_controls;

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
    let kind = classes::by_name(name).map_err(Refusal)?;
    let t = tags::walk(&a.mem, tags_at)?;
    object::create(a, kind, &t)
}

/// MUI_NewObjectA(a0 = class name, a1 = tags): NULL (with the reason) when it cannot be built.
fn mui_new_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(0)).map_err(Refusal::mem("MUI_NewObjectA class name"))?;
    let obj = new_by_name(a, &name, cpu.areg(1))?;
    hooks::after_call(a, cpu, obj)?; // a List asks its display hook for the column titles
    Ok(format!("[muimaster] MUI_NewObjectA \"{name}\" -> {obj:#x}"))
}

/// MUI_DisposeObject(a0 = object): the whole tree below it goes too.
fn mui_dispose_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    dispose(a, obj)?;
    Ok(format!("[muimaster] MUI_DisposeObject {obj:#x}"))
}

/// MUI_MakeObjectA(d0 = type, a0 = parameter array): NULL (with the reason) when it cannot be built.
fn mui_make_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, params) = (cpu.dreg(0), cpu.areg(0));
    let obj = make::make(a, ty, params)?;
    hooks::after_call(a, cpu, obj)?;
    Ok(format!("[muimaster] MUI_MakeObjectA type {ty} -> {obj:#x}"))
}

/// The shared class dispatcher's MUI branch: a guest `DoMethod(obj, ...)` with the message at `msg`.
pub fn dispatch<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, obj: u32, msg: u32) -> Result<String, Refusal> {
    let id = a.mem.read_u32(msg).map_err(Refusal::mem("DoMethod message"))?;
    if id == MUIM_Application_Execute && a.objects.kind(obj) == Some(classes::Kind::Application) {
        let next = app::execute(a, obj)?;
        crate::guest_call::start(a, cpu, next)?;
        return Ok(format!("[boopsi] DoMethod {obj:#x} MUIM_Application_Execute (the program loop)"));
    }
    let r = if id == MUIM_CallHook {
        // The hook is called with the parameters where the guest put them, right after the hook pointer.
        let hook = a.mem.read_u32(msg + 4).map_err(Refusal::mem("MUIM_CallHook hook"))?;
        a.objects.hooks.push_back(hooks::HookCall { hook, obj, msg: hooks::Msg::Addr(msg + 8), after: hooks::After::Nothing });
        0
    } else {
        let n = methods::arity(a, id, msg)?;
        let mut args = alloc::vec::Vec::new();
        for i in 0..n { args.push(a.mem.read_u32(msg + 4 + 4 * i).map_err(Refusal::mem("DoMethod message"))?); }
        method(a, obj, id, &args)?
    };
    hooks::after_call(a, cpu, r)?;
    Ok(format!("[boopsi] DoMethod {obj:#x} {} -> {r:#x}", attrs::method_name(id)))
}

/// A guest `SetAttrs` on a MUI object ran its notifications: call the hooks they queued before returning.
pub fn after_set<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<(), Refusal> { hooks::after_call(a, cpu, 0) }

/// Offer a native event to the MUI objects. None = not one of ours (the caller keeps looking).
pub fn try_deliver<U: GuestUi>(a: &mut Amiga<U>, ev: &GuestUiEvent) -> Result<Option<Delivered>, Refusal> {
    match ev {
        GuestUiEvent::CloseRequested { win, .. } => match win::find_by_native(a, *win) {
            Some(w) => { app::post(a, w, app::Inbound::Close(w))?; Ok(Some(Delivered::Handled)) }
            None => Ok(None),
        },
        GuestUiEvent::MenuCommand { win, token } => match menu::item_action(a, token.0 as u32) {
            Some(_) => { let w = win::find_by_native(a, *win).unwrap_or(0); app::post(a, token.0 as u32, app::Inbound::Menu { item: token.0 as u32, win: w })?; Ok(Some(Delivered::Handled)) }
            None => Ok(None),
        },
        GuestUiEvent::Resized { win, rect } => match win::find_by_native(a, *win) {
            Some(w) => { win::resized(a, w, rect.width, rect.height)?; Ok(Some(Delivered::Handled)) }
            None => Ok(None),
        },
        GuestUiEvent::Intent { win, control, token, generation, intent } => {
            if win::find_by_native(a, *win).is_none() { return Ok(None); }
            a.objects.proj.resolve(token.0, *win, *control, *generation).map_err(Refusal)?;
            let obj = token.0 as u32;
            input::validate(a, obj, intent)?;
            app::post(a, obj, app::Inbound::Intent { obj, intent: intent.clone() })?;
            Ok(Some(Delivered::Handled))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests;
