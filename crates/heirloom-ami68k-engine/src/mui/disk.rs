//! Disk MUI classes (`.mcc` / `.mcp`): a class that is not built in is a real disk library. `MUI_NewObjectA("Foo.mcc")` and
//! `MUI_CreateCustomClass(.., "Foo.mcc", ..)` open it through the engine's disklib (its init and Open vectors run in the guest), call
//! `MCC_Query(0)` (then `MCC_Query(1)`, the preferences class) as a guest call, and use `mcc_Class` of the `struct MUI_CustomClass` it
//! returns. The class is then an ordinary guest `struct IClass`: its dispatcher is guest code and its superclass chain reaches the
//! native MUI classes through the one dispatch path ([`crate::boopsi`]), so an instance's role is its nearest standard ancestor's.
//! Facts: Zune's support_classes.c `ZUNE_GetExternalClass` (the search order and the Query sequence; `mcc_Module` is set to the
//! library), mui_newobject.c (`NewObjectA` on the class), mui_disposeobject.c / mui_freeclass.c (the class is released when its object
//! goes), mui.h (`struct MUI_CustomClass`: mcc_Class 24, mcc_Module 28), the MCC header of the corpus classes (`case 0` returns the
//! `MUI_CustomClass *`, the library vector is the first after Open/Close/Expunge/Reserved = LVO -30).
//! Search order (a name is looked up as `OpenLibrary` does, LIBS: then PROGDIR:): Zune's `Zune/X`, `Classes/Zune/X`, `MUI/X`, then
//! MUI's own `mui/X`, then the explicit volumes `MUI:Libs/mui/X` and `SYS:Classes/Zune/X` (LIBS: names only SYS:Libs on this machine).
//! A loaded class stays loaded for the life of the machine (no Expunge): `users` counts the objects and subclasses that hold it.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::boopsi_ids::{CL_H_ENTRY, OM_NEW};
use crate::boopsi_send::{make_msg, MSG_BLOCK};
use crate::guest_call::{self, Cont, Next};
use crate::{disklib, machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const MCC_CLASS: u32 = 24;
const MCC_MODULE: u32 = 28;
/// The vector after Open, Close, Expunge and Reserved.
const LVO_MCC_QUERY: u32 = 30;

/// One class opened from disk.
#[derive(Debug, Clone)]
pub struct DiskClass { pub lib: u32, pub mcc: u32, pub class: u32, pub users: u32 }

/// Where Zune and MUI look for class `name`, in order.
pub fn search(name: &str) -> Vec<String> {
    ["Zune/", "Classes/Zune/", "MUI/", "mui/"].iter().map(|p| format!("{p}{name}"))
        .chain([format!("MUI:Libs/mui/{name}"), format!("SYS:Classes/Zune/{name}")]).collect()
}

/// Does `cl` belong to a class opened from disk?
pub fn is_disk_class<U: GuestUi>(a: &Amiga<U>, cl: u32) -> bool { a.objects.disk_classes.values().any(|d| d.class == cl) }

/// Give back one use of `cl` (an object of it was disposed, or a subclass of it deleted). Not a disk class: nothing.
pub fn release<U: GuestUi>(a: &mut Amiga<U>, cl: u32) {
    if let Some(d) = a.objects.disk_classes.values_mut().find(|d| d.class == cl) { d.users = d.users.saturating_sub(1); }
}

/// The chain that yields the class pointer of the disk class `name` (one more use of it), or NULL-with-a-reason as an `Err`.
/// `Ok(None)`: no such file anywhere on the search path, so the caller keeps its own "not implemented" answer.
pub fn resolve<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str) -> Result<Option<Next<U>>, Refusal> {
    if let Some(d) = a.objects.disk_classes.get_mut(name) { d.users += 1; return Ok(Some(Next::Return(d.class))); }
    let mut opened = None;
    for p in search(name) { if let Some(x) = disklib::open_chain(a, &p, 0)? { opened = Some((p, x)); break; } }
    let Some((path, (open, line))) = opened else { return Ok(None) };
    a.log.push(line);
    let n = String::from(name);
    // The Open vector's D0 is the library base; then MCC_Query(0), and MCC_Query(1) when it answers nothing (an MCP).
    let chain = guest_call::then(a, cpu, open, Box::new(move |_a: &mut Amiga<U>, _cpu: &mut Cpu, base: u32| {
        if base == 0 { return Err(Refusal(format!("MUI class \"{n}\": OpenLibrary of {path} failed (Open returned NULL)"))); }
        let n2 = n.clone();
        let prefs: Cont<U> = Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, mcc: u32| {
            if mcc == 0 { return Err(Refusal(format!("MUI class \"{n2}\": MCC_Query(0) and MCC_Query(1) both answered NULL"))); }
            finish(a, &n2, base, mcc)
        });
        let main: Cont<U> = Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, mcc: u32| {
            if mcc != 0 { return finish(a, &n, base, mcc); }
            Ok(query(base, 1, prefs))
        });
        Ok(query(base, 0, main))
    }))?;
    Ok(Some(chain))
}

fn query<U: GuestUi>(base: u32, which: u32, then: Cont<U>) -> Next<U> {
    Next::Call { target: base - LVO_MCC_QUERY, d0: which, aregs: vec![(6, base)], then }
}

fn finish<U: GuestUi>(a: &mut Amiga<U>, name: &str, base: u32, mcc: u32) -> Result<Next<U>, Refusal> {
    let class = a.mem.read_u32(mcc + MCC_CLASS).map_err(Refusal::mem("MUI_CustomClass.mcc_Class"))?;
    if class == 0 { return Err(Refusal(format!("MUI class \"{name}\": MCC_Query returned a MUI_CustomClass {mcc:#x} with no mcc_Class"))); }
    if !a.classes.recs.contains_key(&class) { return Err(Refusal(format!("MUI class \"{name}\": mcc_Class {class:#x} is not a BOOPSI class"))); }
    a.mem.write_u32(mcc + MCC_MODULE, base).map_err(Refusal::mem("MUI_CustomClass.mcc_Module"))?;
    a.log.push(format!("[muimaster] disk class \"{name}\" = {class:#x} (library {base:#x}, mcc {mcc:#x})"));
    a.objects.disk_classes.insert(name.into(), DiskClass { lib: base, mcc, class, users: 1 });
    Ok(Next::Return(class))
}

/// `OM_NEW` on `cl` with the tag list at `tags` (what `NewObjectA(cl, ..)` does): the object, or NULL when the class refuses.
pub fn new_instance<U: GuestUi>(a: &mut Amiga<U>, cl: u32, tags: u32) -> Result<Next<U>, Refusal> {
    let entry = a.mem.read_u32(cl + CL_H_ENTRY).map_err(Refusal::mem("cl_Dispatcher.h_Entry"))?;
    if entry == 0 { return Err(Refusal(format!("class {} has no dispatcher (cl_Dispatcher.h_Entry is NULL)", a.classes.name_of(cl)))); }
    let msg = make_msg(a, &[OM_NEW, tags, 0])?;
    Ok(Next::Call { target: entry, d0: 0, aregs: vec![(0, cl), (1, msg), (2, cl)], then: Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, obj: u32| {
        a.heap.free(msg, MSG_BLOCK).map_err(|e| Refusal(format!("BOOPSI message: {e}")))?;
        if obj == 0 { release(a, cl); } // Zune: MUI_NewObjectA frees the class when the object could not be made
        super::hooks::run(a, obj)
    }) })
}
