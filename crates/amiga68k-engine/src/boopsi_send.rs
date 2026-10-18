//! Sending methods to objects, the intuition way: NewObjectA, DisposeObject, SetAttrsA, GetAttr, SetGadgetAttrsA,
//! NextObject. Every one builds the message in guest RAM and calls the object's CLASS DISPATCHER (`cl_Dispatcher.h_Entry`
//! with A0 = class, A2 = object, A1 = message, the hook convention) through guest_call.rs, because a guest class's
//! dispatcher is guest code; a native class's `h_Entry` is the boopsi.rs trap slot, so the same one path serves both.
//! The guest's own amiga.lib DoMethodA/DoSuperMethodA/CoerceMethodA need nothing from here: they read `o_Class` /
//! `cl_Super` and jump to `h_Entry` themselves, which is why those structs are real.
//! MUI objects (host-side, boopsi.rs/mui) are told apart by their record in `Amiga::objects` and sent down the old path.
//! Facts: AROS rom/intuition newobjecta.c, disposeobject.c, setattrsa.c, getattr.c, setgadgetattrsa.c, nextobject.c.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::format;
use crate::boopsi_ids::*;
use crate::guest_call::{self, Cont, Next};
use crate::iclass::Native;
use crate::{machine::Amiga, memory, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const MSG_BLOCK: u32 = 32;

/// Start a guest call of `class`'s dispatcher on `obj` with `msg`; `then` receives the dispatcher's D0.
pub fn send<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, class: u32, obj: u32, msg: u32, then: Cont<U>) -> Result<(), Refusal> {
    let entry = a.mem.read_u32(class + CL_H_ENTRY).map_err(Refusal::mem("cl_Dispatcher.h_Entry"))?;
    if entry == 0 { return Err(Refusal(format!("class {} has no dispatcher (cl_Dispatcher.h_Entry is NULL)", a.classes.name_of(class)))); }
    guest_call::start(a, cpu, Next::Call { target: entry, d0: 0, aregs: vec![(0, class), (1, msg), (2, obj)], then })
}

/// A scratch message of up to 32 bytes, longwords from `words`.
pub fn make_msg<U: GuestUi>(a: &mut Amiga<U>, words: &[u32]) -> Result<u32, Refusal> {
    let m = memory::alloc_zeroed(a, MSG_BLOCK, "BOOPSI message")?;
    for (i, w) in words.iter().enumerate() { a.mem.write_u32(m + 4 * i as u32, *w).map_err(Refusal::mem("BOOPSI message"))?; }
    Ok(m)
}

/// The continuation that frees `msg` and answers with the dispatcher's D0 (`post` may adjust it).
pub fn finish<U: GuestUi>(msg: u32) -> Cont<U> { finish_with(msg, 0) }

/// Like [`finish`], and frees the GadgetInfo `gi` too (0 = none).
pub fn finish_with<U: GuestUi>(msg: u32, gi: u32) -> Cont<U> {
    Box::new(move |a, _cpu, d0| {
        a.heap.free(msg, MSG_BLOCK).map_err(|e| Refusal(format!("BOOPSI message: {e}")))?;
        crate::boopsi_gadget::free_ginfo(a, gi)?;
        Ok(Next::Return(d0))
    })
}

fn is_mui<U: GuestUi>(a: &Amiga<U>, obj: u32) -> bool { a.objects.map.contains_key(&obj) }

fn live_class<U: GuestUi>(a: &Amiga<U>, obj: u32, what: &str) -> Result<u32, Refusal> {
    a.classes.class_of(obj).ok_or_else(|| Refusal(format!("{what}: {obj:#x} is not a BOOPSI object")))
}

/// NewObjectA(a0 = class pointer or NULL, a1 = public class name, a2 = tags) -> the object or NULL.
pub fn new_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (cp, idp, tags) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let (cl, shown) = if cp != 0 { (cp, a.classes.name_of(cp)) } else {
        let name = a.mem.read_cstr(idp).map_err(Refusal::mem("NewObjectA class name"))?;
        match a.classes.find(&name) {
            Some(c) => (c, name),
            None => { cpu.set_d0(0); return Ok(format!("[intuition] NewObjectA \"{name}\" -> NULL (no such public class)")); }
        }
    };
    let Some(rec) = a.classes.recs.get(&cl) else { return Err(Refusal(format!("NewObjectA: {cl:#x} is not a class"))) };
    if matches!(rec.native, Some(Native::Mui(_))) { cpu.set_areg(0, 0); cpu.set_areg(1, idp); return crate::boopsi::new_object_a(a, cpu); }
    let msg = make_msg(a, &[OM_NEW, tags, 0])?;
    send(a, cpu, cl, cl, msg, finish(msg))?;
    Ok(format!("[intuition] NewObjectA \"{shown}\" tags={tags:#x}"))
}

/// DisposeObject(a0 = object or NULL): OM_DISPOSE to its class.
pub fn dispose_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    if obj == 0 { return Ok("[intuition] DisposeObject NULL".into()); }
    if is_mui(a, obj) { return crate::boopsi::dispose_object(a, cpu); }
    let cl = live_class(a, obj, "DisposeObject")?;
    let msg = make_msg(a, &[OM_DISPOSE])?;
    send(a, cpu, cl, obj, msg, finish(msg))?;
    Ok(format!("[intuition] DisposeObject {obj:#x} ({})", a.classes.name_of(cl)))
}

/// SetAttrsA(a0 = object, a1 = tags) -> what OM_SET returns.
pub fn set_attrs_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (obj, tags) = (cpu.areg(0), cpu.areg(1));
    if is_mui(a, obj) { return crate::boopsi::set_attrs_a(a, cpu); }
    let cl = live_class(a, obj, "SetAttrsA")?;
    let msg = make_msg(a, &[OM_SET, tags, 0])?;
    send(a, cpu, cl, obj, msg, finish(msg))?;
    Ok(format!("[intuition] SetAttrsA {obj:#x} ({})", a.classes.name_of(cl)))
}

/// GetAttr(d0 = attribute, a0 = object, a1 = storage) -> what OM_GET returns (TRUE when the attribute is known).
pub fn get_attr<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (attr, obj, store) = (cpu.dreg(0), cpu.areg(0), cpu.areg(1));
    if is_mui(a, obj) { return crate::boopsi::get_attr(a, cpu); }
    let cl = live_class(a, obj, "GetAttr")?;
    let msg = make_msg(a, &[OM_GET, attr, store])?;
    send(a, cpu, cl, obj, msg, finish(msg))?;
    Ok(format!("[intuition] GetAttr {obj:#x} {attr:#x} ({})", a.classes.name_of(cl)))
}

/// SetGadgetAttrsA(a0 = gadget, a1 = window, a2 = requester, a3 = tags): OM_SET with the window's GadgetInfo.
pub fn set_gadget_attrs_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (g, win, req, tags) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if is_mui(a, g) { return Err(Refusal("SetGadgetAttrsA on a MUI object".into())); }
    let cl = live_class(a, g, "SetGadgetAttrsA")?;
    let gi = if win != 0 || req != 0 { crate::boopsi_gadget::make_ginfo(a, win, req, g)? } else { 0 };
    let msg = make_msg(a, &[OM_SET, tags, gi])?;
    send(a, cpu, cl, g, msg, finish_with(msg, gi))?;
    Ok(format!("[intuition] SetGadgetAttrsA {g:#x} ({}) win={win:#x}", a.classes.name_of(cl)))
}

/// NextObject(a0 = &state) -> the current member, advancing `state` to the next list node (NULL at the end).
pub fn next_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let pp = cpu.areg(0);
    let node = a.mem.read_u32(pp).map_err(Refusal::mem("NextObject state"))?;
    let succ = a.mem.read_u32(node).map_err(Refusal::mem("NextObject node"))?;
    let r = if succ != 0 { a.mem.write_u32(pp, succ).map_err(Refusal::mem("NextObject state"))?; node + OBJ_HEADER } else { 0 };
    cpu.set_d0(r);
    Ok(format!("[intuition] NextObject {node:#x} -> {r:#x}"))
}

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("NewObjectA", new_object_a::<U>).named("DisposeObject", dispose_object::<U>).named("SetAttrsA", set_attrs_a::<U>)
        .named("GetAttr", get_attr::<U>).named("SetGadgetAttrsA", set_gadget_attrs_a::<U>).named("NextObject", next_object::<U>)
        .named("MakeClass", crate::iclass::make_class::<U>).named("AddClass", crate::iclass::add_class::<U>)
        .named("RemoveClass", crate::iclass::remove_class::<U>).named("FreeClass", crate::iclass::free_class::<U>)
        .named("FindClass", crate::iclass::find_class::<U>)
}

