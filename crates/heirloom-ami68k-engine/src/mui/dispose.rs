//! Disposing a MUI tree the way Zune does (family.c `Family__OM_DISPOSE`, group.c, window.c): an object's OM_DISPOSE disposes each
//! child through `MUI_DisposeObject(child)`, which is `DisposeObject` on the child's OWN class and then `MUI_FreeClass` of that class,
//! and only then lets the superclass free the object. So the destructor of a program's (or a disk class's) subclass runs for every
//! child that has one, children first, in the order they were added; a native child is torn down by the engine as before. The
//! guest calls are a chain ([`crate::guest_call`]): each step starts when the one before returned.
use alloc::format;
use alloc::vec::Vec;
use alloc::boxed::Box;
use super::classes::Kind;
use super::{disk, object, win};
use crate::boopsi_ids::OM_DISPOSE;
use crate::boopsi_send::{make_msg, MSG_BLOCK};
use crate::guest_call::{self, Next};
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// The class of `obj` when it is an instance of a guest class (a program's or a disk class), whose dispatcher is guest code.
fn guest_class<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> {
    let cl = a.classes.class_of(obj)?;
    a.classes.recs.get(&cl).filter(|r| r.native.is_none()).map(|_| cl)
}

/// `DisposeObject(obj)` then `MUI_FreeClass`: the object's own dispatcher gets OM_DISPOSE (it reaches the native half through
/// `DoSuperMethod`, which disposes the children and frees the object), or a native object is torn down directly.
pub fn dispose_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, obj: u32) -> Result<Next<U>, Refusal> {
    match guest_class(a, obj) {
        Some(cl) => {
            let entry = a.mem.read_u32(cl + crate::boopsi_ids::CL_H_ENTRY).map_err(Refusal::mem("cl_Dispatcher.h_Entry"))?;
            if entry == 0 { return Err(Refusal(format!("class {} has no dispatcher (cl_Dispatcher.h_Entry is NULL)", a.classes.name_of(cl)))); }
            let msg = make_msg(a, &[OM_DISPOSE])?;
            Ok(Next::Call { target: entry, d0: 0, aregs: alloc::vec![(0, cl), (1, msg), (2, obj)], then: Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, _| {
                a.heap.free(msg, MSG_BLOCK).map_err(|e| Refusal(format!("BOOPSI message: {e}")))?;
                disk::release(a, cl);
                Ok(Next::Return(0))
            }) })
        }
        None => native(a, cpu, obj),
    }
}

/// The native half of OM_DISPOSE: the children first (each through [`dispose_object`]), then the object itself.
pub fn native<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, obj: u32) -> Result<Next<U>, Refusal> {
    if let Some(p) = a.objects.map.get(&obj).and_then(|o| o.parent) { if let Some(po) = a.objects.map.get_mut(&p) { po.children.retain(|&c| c != obj); } }
    if a.objects.kind(obj) == Some(Kind::Window) { win::close_if_open(a, obj)?; } // the window goes before the objects in it
    let kids: Vec<u32> = a.objects.children(obj).to_vec();
    children(a, cpu, obj, kids)
}

fn children<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, obj: u32, mut kids: Vec<u32>) -> Result<Next<U>, Refusal> {
    while !kids.is_empty() {
        let c = kids.remove(0);
        let Some(co) = a.objects.map.get_mut(&c) else { continue };
        co.parent = None; // it leaves the family before its own destructor runs
        if let Some(o) = a.objects.map.get_mut(&obj) { o.children.retain(|&x| x != c); }
        if guest_class(a, c).is_none() {
            // A native child: its own subtree is the same walk (a custom grandchild still gets its destructor), then the next sibling.
            let next = native(a, cpu, c)?;
            if matches!(next, Next::Return(_)) { continue; }
            return guest_call::then(a, cpu, next, Box::new(move |a: &mut Amiga<U>, cpu: &mut Cpu, _| children(a, cpu, obj, kids)));
        }
        let next = dispose_object(a, cpu, c)?;
        return guest_call::then(a, cpu, next, Box::new(move |a: &mut Amiga<U>, cpu: &mut Cpu, _| children(a, cpu, obj, kids)));
    }
    object::dispose_remaining(a, obj)?;
    Ok(Next::Return(0))
}
