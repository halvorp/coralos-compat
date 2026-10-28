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
pub mod broker;
pub mod attrs;
pub mod classes;
pub(crate) mod custom;
pub mod disk;
mod dispose;
pub mod drawn;
pub mod handlers;
pub mod hooks;
mod ids_gen;
pub mod images;
mod input;
pub mod layout;
pub mod list;
mod make;
mod menu;
mod methods;
mod request;
mod set_string;
pub mod mirror;
mod names;
mod notify;
mod object;
pub mod pens;
pub mod roles;
pub mod store;
pub mod surface;
pub mod win;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use crate::guest_call;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, run::Delivered, tags, Refusal};
use heirloom_ui_abi::{GuestUi, GuestUiEvent};
use crate::cpu::Cpu;

pub use ids_gen::*;
pub use methods::{method, OM_ADDMEMBER, OM_REMMEMBER};
pub use object::{dispose, get_attr, set_attr, set_attr_quiet};
pub use object::set_list;
pub use win::native_controls;

const LVO_MUI_NEW_OBJECT_A: i32 = -30;
const LVO_MUI_ALLOC_ASL_REQUEST: i32 = -48;
const LVO_MUI_FREE_ASL_REQUEST: i32 = -60;
const LVO_MUI_REQUEST_A: i32 = -42;
const LVO_MUI_REDRAW: i32 = -102;
const LVO_MUI_ADD_CLIP_REGION: i32 = -180;
const LVO_MUI_REMOVE_CLIP_REGION: i32 = -186;
const LVO_MUI_DISPOSE_OBJECT: i32 = -36;
const LVO_MUI_MAKE_OBJECT_A: i32 = -120;
const LVO_MUI_CREATE_CUSTOM_CLASS: i32 = -108;
const LVO_MUI_DELETE_CUSTOM_CLASS: i32 = -114;
const LVO_MUI_OBTAIN_PEN: i32 = -156;
const LVO_MUI_RELEASE_PEN: i32 = -162;

/// muimaster.library with the functions the engine bridges; every other vector stays a named refusal.
pub fn library<U: GuestUi>(base: u32) -> LibraryBuilder<U> {
    LibraryBuilder::new("muimaster.library", base)
        .func(LVO_MUI_NEW_OBJECT_A, "MUI_NewObjectA", mui_new_object_a::<U>)
        .func(LVO_MUI_REQUEST_A, "MUI_RequestA", request::mui_request_a::<U>)
        .func(LVO_MUI_DISPOSE_OBJECT, "MUI_DisposeObject", mui_dispose_object::<U>)
        .func(LVO_MUI_MAKE_OBJECT_A, "MUI_MakeObjectA", mui_make_object_a::<U>)
        // muimaster.library/MUI_AllocAslRequest(type, tags) is asl.library's AllocAslRequest with the same registers (Zune: mui_allocaslrequest.c).
        .func(LVO_MUI_ALLOC_ASL_REQUEST, "MUI_AllocAslRequest", crate::asl::alloc_request::<U>)
        .func(LVO_MUI_FREE_ASL_REQUEST, "MUI_FreeAslRequest", crate::asl::free_request::<U>)
        .func(LVO_MUI_ADD_CLIP_REGION, "MUI_AddClipRegion", surface::add_clip_region::<U>)
        .func(LVO_MUI_REMOVE_CLIP_REGION, "MUI_RemoveClipRegion", surface::remove_clip_region::<U>)
        .func(LVO_MUI_REDRAW, "MUI_Redraw", mui_redraw::<U>)
        .func(LVO_MUI_CREATE_CUSTOM_CLASS, "MUI_CreateCustomClass", custom::create_custom_class::<U>)
        .func(LVO_MUI_DELETE_CUSTOM_CLASS, "MUI_DeleteCustomClass", custom::delete_custom_class::<U>)
        .func(LVO_MUI_OBTAIN_PEN, "MUI_ObtainPen", pens::obtain_pen::<U>)
        .func(LVO_MUI_RELEASE_PEN, "MUI_ReleasePen", pens::release_pen::<U>)
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
    if !classes::is_builtin(&name) {
        // Not built in: a disk class (Zune's ZUNE_GetExternalClass), made by a guest call chain that ends with the object.
        if let Some(found) = disk::resolve(a, cpu, &name)? {
            let tags_at = cpu.areg(1);
            let next = guest_call::then(a, cpu, found, Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, cl: u32| disk::new_instance(a, cl, tags_at)))?;
            guest_call::start(a, cpu, next)?;
            return Ok(format!("[muimaster] MUI_NewObjectA \"{name}\" (disk class)"));
        }
    }
    let obj = new_by_name(a, &name, cpu.areg(1))?;
    hooks::after_call(a, cpu, obj)?; // a List asks its display hook for the column titles
    Ok(format!("[muimaster] MUI_NewObjectA \"{name}\" -> {obj:#x}"))
}

/// MUI_DisposeObject(a0 = object): the whole tree below it goes too.
fn mui_dispose_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    if obj == 0 { return Ok("[muimaster] MUI_DisposeObject NULL".into()); } // "May be NULL, in which case this function has no effect"
    dispose_start(a, cpu, obj)?;
    Ok(format!("[muimaster] MUI_DisposeObject {obj:#x}"))
}

/// DisposeObject on a MUI root: its own dispatcher, then every child's (Zune's family dispose order, [`dispose`]), as guest calls.
pub fn dispose_start<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, obj: u32) -> Result<(), Refusal> {
    match a.objects.map.get(&obj) {
        None => return Err(Refusal(format!("DisposeObject: {obj:#x} is not a MUI object (or already disposed)"))),
        Some(o) if o.parent.is_some() => return Err(Refusal(format!("DisposeObject: {obj:#x} is owned by a parent; dispose the root"))),
        Some(_) => {}
    }
    let next = dispose::dispose_object(a, cpu, obj)?;
    guest_call::start(a, cpu, next)
}

/// MUI_Redraw(a0 = object, d0 = flags): the object is drawn again (MUIM_Draw, as the window does it).
fn mui_redraw<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (obj, flags) = (cpu.areg(0), cpu.dreg(0));
    drawn::redraw(a, obj, flags);
    hooks::after_call(a, cpu, 0)?;
    Ok(format!("[muimaster] MUI_Redraw {obj:#x} flags {flags:#x}"))
}

/// MUI_MakeObjectA(d0 = type, a0 = parameter array): NULL (with the reason) when it cannot be built.
fn mui_make_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, params) = (cpu.dreg(0), cpu.areg(0));
    let obj = make::make(a, ty, params)?;
    hooks::after_call(a, cpu, obj)?;
    Ok(format!("[muimaster] MUI_MakeObjectA type {ty} -> {obj:#x}"))
}

/// The shared class dispatcher's MUI branch: a guest `DoMethod(obj, ...)` with the message at `msg`.
pub fn dispatch<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, cl: u32, obj: u32, msg: u32) -> Result<String, Refusal> {
    let id = a.mem.read_u32(msg).map_err(Refusal::mem("DoMethod message"))?;
    if id == crate::boopsi_ids::OM_DISPOSE && a.objects.map.contains_key(&obj) {
        // The native half of a destructor (a subclass's DoSuperMethod, or a DoMethod on a native object): children first, then the object.
        let next = dispose::native(a, cpu, obj)?;
        guest_call::start(a, cpu, next)?;
        return Ok(format!("[boopsi] {} OM_DISPOSE on {obj:#x}", a.classes.name_of(cl)));
    }
    if let Some(r) = drawn::area_method(a, obj, id, msg)? { hooks::after_call(a, cpu, r)?; return Ok(format!("[boopsi] {} {} on {obj:#x} -> {r:#x}", a.classes.name_of(cl), attrs::method_name(id))); }
    if let Some(r) = custom::om_method(a, cl, obj, id, msg)? { hooks::after_call(a, cpu, r)?; return Ok(format!("[boopsi] {} method {id:#x} on {obj:#x} -> {r:#x}", a.classes.name_of(cl))); }
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
            Some(w) => {
                win::resized(a, w, rect.width, rect.height)?;
                if !a.objects.draw_steps.is_empty() { app::post(a, w, app::Inbound::Redraw)?; } // self-drawing objects are drawn in the program's loop
                Ok(Some(Delivered::Handled))
            }
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
        GuestUiEvent::Pointer { win, x, y, button, phase, .. } => match win::find_by_native(a, *win) {
            Some(w) if handlers::listening(a, w) => { handlers::deliver(a, w, handlers::Event::Pointer { win: w, x: *x, y: *y, button: *button, phase: *phase })?; Ok(Some(Delivered::Handled)) }
            Some(_) => Ok(Some(Delivered::IgnoredUnsubscribed)),
            None => Ok(None),
        },
        GuestUiEvent::Key { win, code, modifiers, down, .. } => match win::find_by_native(a, *win) {
            Some(w) if handlers::listening(a, w) => { a.input.modifiers = *modifiers; handlers::deliver(a, w, handlers::Event::Key { win: w, code: *code, down: *down })?; Ok(Some(Delivered::Handled)) }
            Some(_) => Ok(Some(Delivered::IgnoredUnsubscribed)),
            None => Ok(None),
        },
        GuestUiEvent::RequestAnswered { .. } => Ok(None), // a requester's answer is intuition's (requester.rs)
    }
}

#[cfg(test)]
mod tests;
