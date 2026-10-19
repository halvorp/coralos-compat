//! A program's own subclass of a MUI class (`MUI_CreateCustomClass`): a real BOOPSI class whose superclass chain ends in a native MUI
//! class, so the program's dispatcher is guest code and `DoSuperMethod` lands here. An instance's memory is rootclass's (the
//! subclass's instance data lives in it); the MUI state is the usual host record, made when the native part sees `OM_NEW`.
//! What the object IS on screen is the nearest standard ancestor's role ([`super::roles`]): the subclass uses that ancestor's state.
//! Facts: Zune's mui_createcustomclass.c (`MakeClass(id, NULL, super, datasize, 0)`, the dispatcher into `cl_Dispatcher`, the
//! `struct MUI_CustomClass` of mui.h: mcc_UserData 0, four library bases, mcc_Super 20, mcc_Class 24), mui_deletecustomclass.c.
//! NOT bridged: the drawing pipeline (`MUIM_Setup/AskMinMax/Show/Draw`) is never sent, so a subclass that DRAWS itself shows nothing.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use super::classes::Kind;
use super::object;
use crate::boopsi_ids::*;
use crate::boopsi_native;
use crate::guest_call::{self, Next};
use crate::iclass::Native;
use crate::{machine::Amiga, tags, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const MCC_SIZE: u32 = 40;
const MCC_SUPER: u32 = 20;
const MCC_CLASS: u32 = 24;
const H_ENTRY: u32 = 8;
const H_SUB_ENTRY: u32 = 12;
const H_DATA: u32 = 16;

/// MUI_CreateCustomClass(a0 = base, a1 = supername, a2 = supermcc, d0 = datasize, a3 = dispatcher) -> a `struct MUI_CustomClass *`.
/// A superclass that is not built in is a disk class ([`super::disk`]): it is opened first, by guest calls, and the class is made when
/// they are done (the registers of this call are read before any of them runs).
pub fn create_custom_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (base, name_p, supermcc, size, dispatcher) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.dreg(0), cpu.areg(3));
    if name_p == 0 && supermcc == 0 { cpu.set_d0(0); return Ok("[muimaster] MUI_CreateCustomClass: no superclass given -> NULL".into()); }
    if supermcc != 0 {
        let sup = a.mem.read_u32(supermcc + MCC_CLASS).map_err(Refusal::mem("MUI_CustomClass.mcc_Class"))?;
        return make_custom(a, cpu, base, size, dispatcher, sup);
    }
    let name = a.mem.read_cstr(name_p).map_err(Refusal::mem("MUI_CreateCustomClass supername"))?;
    if let Some(sup) = a.classes.find(&name) { return make_custom(a, cpu, base, size, dispatcher, sup); }
    let Some(found) = super::disk::resolve(a, cpu, &name)? else {
        return Err(Refusal(format!("MUI_CreateCustomClass: the superclass \"{name}\" is not implemented")));
    };
    let next = guest_call::then(a, cpu, found, Box::new(move |a: &mut Amiga<U>, cpu: &mut Cpu, sup: u32| {
        let line = make_custom(a, cpu, base, size, dispatcher, sup)?;
        a.log.push(line);
        Ok(Next::Return(cpu.dreg(0)))
    }))?;
    guest_call::start(a, cpu, next)?;
    Ok(format!("[muimaster] MUI_CreateCustomClass over the disk class \"{name}\""))
}

/// The class over superclass `sup`; D0 = the `struct MUI_CustomClass` (or NULL).
fn make_custom<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, base: u32, size: u32, dispatcher: u32, sup: u32) -> Result<String, Refusal> {
    let mcc = a.heap.alloc(MCC_SIZE).ok_or_else(|| Refusal("MUI_CreateCustomClass: no memory (NOMEM)".into()))?;
    a.mem.zero(mcc, MCC_SIZE).map_err(Refusal::mem("MUI_CustomClass"))?;
    // MakeClass(NULL, NULL, super, datasize, 0): the same code the program's own MakeClass runs.
    (cpu.set_areg(0, 0), cpu.set_areg(1, 0), cpu.set_areg(2, sup), cpu.set_dreg(0, size), cpu.set_dreg(1, 0));
    crate::iclass::make_class(a, cpu)?;
    let cl = cpu.dreg(0);
    if cl == 0 { let _ = a.heap.free(mcc, MCC_SIZE); super::disk::release(a, sup); return Ok("[muimaster] MUI_CreateCustomClass -> NULL (MakeClass failed)".into()); }
    let w = |a: &mut Amiga<U>, at: u32, v: u32| a.mem.write_u32(at, v).map_err(Refusal::mem("MUI_CustomClass"));
    w(a, mcc + MCC_SUPER, sup)?;
    w(a, mcc + MCC_CLASS, cl)?;
    (w(a, cl + H_ENTRY, dispatcher)?, w(a, cl + H_SUB_ENTRY, dispatcher)?, w(a, cl + H_DATA, base)?);
    cpu.set_d0(mcc);
    Ok(format!("[muimaster] MUI_CreateCustomClass super={} size={size} -> {mcc:#x} (class {cl:#x})", a.classes.name_of(sup)))
}

/// MUI_DeleteCustomClass(a0 = mcc) -> TRUE when the class could be freed.
pub fn delete_custom_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mcc = cpu.areg(0);
    if mcc == 0 { cpu.set_d0(0); return Ok("[muimaster] MUI_DeleteCustomClass NULL -> FALSE".into()); } // mui_deletecustomclass.c: a NULL class is FALSE
    let cl = a.mem.read_u32(mcc + MCC_CLASS).map_err(Refusal::mem("MUI_CustomClass.mcc_Class"))?;
    let sup = a.mem.read_u32(mcc + MCC_SUPER).map_err(Refusal::mem("MUI_CustomClass.mcc_Super"))?;
    cpu.set_areg(0, cl);
    crate::iclass::free_class(a, cpu)?;
    let freed = cpu.dreg(0) != 0;
    if freed { super::disk::release(a, sup); } // mui_deletecustomclass.c: MUI_FreeClass(mcc_Super)
    if freed { a.heap.free(mcc, MCC_SIZE).map_err(|e| Refusal(format!("MUI_DeleteCustomClass: {e}")))?; }
    Ok(format!("[muimaster] MUI_DeleteCustomClass {mcc:#x} -> {}", if freed { "TRUE" } else { "FALSE" }))
}

/// The native MUI class `cl` (or the class an instance is being made from) as the kind it implements.
fn kind_of_class<U: GuestUi>(a: &Amiga<U>, cl: u32) -> Option<Kind> {
    match a.classes.recs.get(&cl)?.native { Some(Native::Mui(k)) => Some(k), _ => None }
}

/// Tags that are MUI's: its attribute ranges (mui.h 0x8042xxxx, Zune's own 0x9042xxxx) and the generic tag controls. A subclass passes
/// its own attributes down in the same tag list; the native class has nothing to say about them, exactly as in BOOPSI.
fn muis(t: &[(u32, u32)]) -> alloc::vec::Vec<(u32, u32)> { t.iter().copied().filter(|(tag, _)| matches!(tag >> 16, 0x8042 | 0x9042)).collect() }

/// The tags the native class `kind` is to act on. A tag in MUI's ranges that no class of `kind`'s chain defines is another class's attribute
/// (the Palette editor's own class over Group takes `MUIA_Palette_Entries` itself; a program numbers its own attributes in the same range):
/// BOOPSI ignores a tag no class of the chain knows, and so does this, with a line in the log. A plain (native) object keeps the strict
/// rule, a named refusal, in `object::set_attr`.
fn native_tags<U: GuestUi>(a: &mut Amiga<U>, kind: Kind, t: alloc::vec::Vec<(u32, u32)>) -> alloc::vec::Vec<(u32, u32)> {
    let (known, ignored): (alloc::vec::Vec<_>, alloc::vec::Vec<_>) = t.into_iter().partition(|(tag, _)| super::attrs::lookup(kind, *tag).is_some());
    for (tag, _) in ignored { a.log.push(format!("[muimaster] {} class: tag {} is no attribute of its chain: left to the subclass", kind.name(), super::attrs::name(tag))); }
    known
}

/// `OM_NEW` / `OM_DISPOSE` / `OM_SET` / `OM_GET` reaching a native MUI class through `DoSuperMethod`. `Some(result)` = handled.
pub fn om_method<U: GuestUi>(a: &mut Amiga<U>, native_cl: u32, obj: u32, id: u32, msg: u32) -> Result<Option<u32>, Refusal> {
    let kind = kind_of_class(a, native_cl).ok_or_else(|| Refusal(format!("{}: not a native MUI class", a.classes.name_of(native_cl))))?;
    Ok(Some(match id {
        OM_NEW => {
            // `obj` is the class the object is made from (rootclass.c): rootclass allocates the instance, the MUI half follows.
            let o = boopsi_native::dispatch(a, Native::Root, native_cl, obj, msg)?;
            let attrs = a.mem.read_u32(msg + 4).map_err(Refusal::mem("ops_AttrList"))?;
            let t = muis(&tags::walk(&a.mem, attrs)?);
            let t = native_tags(a, kind, t);
            crate::boopsi::adopt_object(a, kind, o);
            if let Err(e) = object::init_adopted(a, o, kind, &t) {
                let _ = object::dispose_adopted(a, o);
                return Err(e);
            }
            o
        }
        OM_SET | OM_UPDATE => {
            let attrs = a.mem.read_u32(msg + 4).map_err(Refusal::mem("ops_AttrList"))?;
            let k = a.objects.kind(obj).unwrap_or(kind);
            let t = muis(&tags::walk(&a.mem, attrs)?);
            for (tag, v) in native_tags(a, k, t) { object::set_attr(a, obj, tag, v)?; }
            0
        }
        OM_GET => {
            let (attr, store) = (a.mem.read_u32(msg + 4).map_err(Refusal::mem("opg_AttrID"))?, a.mem.read_u32(msg + 8).map_err(Refusal::mem("opg_Storage"))?);
            match object::get_attr(a, obj, attr) {
                Ok(v) => { a.mem.write_u32(store, v).map_err(Refusal::mem("opg_Storage"))?; 1 }
                Err(_) => 0, // BOOPSI: an attribute no class knows is simply not answered
            }
        }
        _ => return Ok(None),
    }))
}

/// The program's own (guest) class `obj` was made from, if any: its dispatcher sees every method sent to the object first.
pub fn own_class<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> {
    a.classes.class_of(obj).filter(|c| a.classes.recs.get(c).is_some_and(|r| r.native.is_none()))
}

/// `OM_SET` for an object of a program's own class, through ITS dispatcher (as BOOPSI's `SetAttrsA` is `DoMethod(obj, OM_SET, tags)`): the
/// subclass sees its own attributes first and `DoSuperMethod` reaches the native class for the MUI ones ([`om_method`]). The call is queued
/// and runs before the trapped call returns ([`super::hooks`]).
pub fn queue_om_set<U: GuestUi>(a: &mut Amiga<U>, cl: u32, obj: u32, msg: super::hooks::Msg) {
    a.objects.hooks.push_back(super::hooks::HookCall { hook: cl, obj, msg, after: super::hooks::After::Nothing });
}
