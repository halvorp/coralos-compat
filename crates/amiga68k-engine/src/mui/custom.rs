//! A program's own subclass of a MUI class (`MUI_CreateCustomClass`): a real BOOPSI class whose superclass chain ends in a native MUI
//! class, so the program's dispatcher is guest code and `DoSuperMethod` lands here. An instance's memory is rootclass's (the
//! subclass's instance data lives in it); the MUI state is the usual host record, made when the native part sees `OM_NEW`.
//! What the object IS on screen is the nearest standard ancestor's role ([`super::roles`]): the subclass uses that ancestor's state.
//! Facts: Zune's mui_createcustomclass.c (`MakeClass(id, NULL, super, datasize, 0)`, the dispatcher into `cl_Dispatcher`, the
//! `struct MUI_CustomClass` of mui.h: mcc_UserData 0, four library bases, mcc_Super 20, mcc_Class 24), mui_deletecustomclass.c.
//! NOT bridged: the drawing pipeline (`MUIM_Setup/AskMinMax/Show/Draw`) is never sent, so a subclass that DRAWS itself shows nothing.
use alloc::format;
use alloc::string::String;
use super::classes::Kind;
use super::object;
use crate::boopsi_ids::*;
use crate::boopsi_native;
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
pub fn create_custom_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (base, name_p, supermcc, size, dispatcher) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.dreg(0), cpu.areg(3));
    if name_p == 0 && supermcc == 0 { cpu.set_d0(0); return Ok("[muimaster] MUI_CreateCustomClass: no superclass given -> NULL".into()); }
    let sup = if supermcc != 0 { a.mem.read_u32(supermcc + MCC_CLASS).map_err(Refusal::mem("MUI_CustomClass.mcc_Class"))? } else {
        let name = a.mem.read_cstr(name_p).map_err(Refusal::mem("MUI_CreateCustomClass supername"))?;
        a.classes.find(&name).ok_or_else(|| Refusal(format!("MUI_CreateCustomClass: the superclass \"{name}\" is not implemented")))?
    };
    let mcc = a.heap.alloc(MCC_SIZE).ok_or_else(|| Refusal("MUI_CreateCustomClass: no memory (NOMEM)".into()))?;
    a.mem.zero(mcc, MCC_SIZE).map_err(Refusal::mem("MUI_CustomClass"))?;
    // MakeClass(NULL, NULL, super, datasize, 0): the same code the program's own MakeClass runs.
    (cpu.set_areg(0, 0), cpu.set_areg(1, 0), cpu.set_areg(2, sup), cpu.set_dreg(1, 0));
    crate::iclass::make_class(a, cpu)?;
    let cl = cpu.dreg(0);
    if cl == 0 { let _ = a.heap.free(mcc, MCC_SIZE); return Ok("[muimaster] MUI_CreateCustomClass -> NULL (MakeClass failed)".into()); }
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
    cpu.set_areg(0, cl);
    crate::iclass::free_class(a, cpu)?;
    let freed = cpu.dreg(0) != 0;
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

/// `OM_NEW` / `OM_DISPOSE` / `OM_SET` / `OM_GET` reaching a native MUI class through `DoSuperMethod`. `Some(result)` = handled.
pub fn om_method<U: GuestUi>(a: &mut Amiga<U>, native_cl: u32, obj: u32, id: u32, msg: u32) -> Result<Option<u32>, Refusal> {
    let kind = kind_of_class(a, native_cl).ok_or_else(|| Refusal(format!("{}: not a native MUI class", a.classes.name_of(native_cl))))?;
    Ok(Some(match id {
        OM_NEW => {
            // `obj` is the class the object is made from (rootclass.c): rootclass allocates the instance, the MUI half follows.
            let o = boopsi_native::dispatch(a, Native::Root, native_cl, obj, msg)?;
            let attrs = a.mem.read_u32(msg + 4).map_err(Refusal::mem("ops_AttrList"))?;
            let t = muis(&tags::walk(&a.mem, attrs)?);
            crate::boopsi::adopt_object(a, kind, o);
            if let Err(e) = object::init_adopted(a, o, kind, &t) {
                let _ = object::dispose_adopted(a, o);
                return Err(e);
            }
            o
        }
        OM_DISPOSE => { object::dispose_adopted(a, obj)?; 0 }
        OM_SET | OM_UPDATE => {
            let attrs = a.mem.read_u32(msg + 4).map_err(Refusal::mem("ops_AttrList"))?;
            for (tag, v) in muis(&tags::walk(&a.mem, attrs)?) { object::set_attr(a, obj, tag, v)?; }
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
