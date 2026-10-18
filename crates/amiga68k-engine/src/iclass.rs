//! BOOPSI classes (intuition/classes.h): a real guest `struct IClass` per class, host-side bookkeeping behind it.
//!
//! A class is 60 guest bytes whose first 20 are its dispatcher `struct Hook`, so a class pointer is also its hook:
//! `h_Entry` is called with A0 = class, A2 = object, A1 = message. The guest may READ the struct (cl_Super, cl_InstOffset,
//! cl_UserData ... are how dispatchers find their instance data) but everything intuition decides from, the registry of
//! classes, who is a subclass of whom, how many objects live, the public names, is host-side in [`Classes`], so a guest
//! that scribbles on cl_ObjectCount cannot make FreeClass free a class with live objects.
//! The built-in ("native") classes sit in a fixed table below LOAD_BASE; their `h_Entry` is the one trap slot of
//! boopsi.rs, so a guest subclass reaches them with the same DoSuperMethod it uses for a guest superclass.
//! Facts: AROS rom/intuition makeclass.c, freeclass.c, addclass.c, removeclass.c, findclass.c, rootclass.c.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::format;
use crate::boopsi::{Kind, BOOPSI_BASE};
use crate::boopsi_ids::*;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

/// The host implementation behind a native class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Native { Root, Image, Gadget, Button, Strg, Prop, Sysi, Frame, Mui(Kind) }

/// (public name, superclass name, implementation, instance size). Supers come first.
pub const NATIVE: [(&str, &str, Native, u16); 15] = [
    ("rootclass", "", Native::Root, 0),
    ("imageclass", "rootclass", Native::Image, IMG_SIZE as u16),
    ("gadgetclass", "rootclass", Native::Gadget, GDATA_SIZE as u16),
    ("buttongclass", "gadgetclass", Native::Button, 0),
    ("strgclass", "gadgetclass", Native::Strg, STRG_DATA as u16),
    ("propgclass", "gadgetclass", Native::Prop, 0),
    ("sysiclass", "imageclass", Native::Sysi, 0),
    ("frameiclass", "imageclass", Native::Frame, 0),
    ("Application.mui", "rootclass", Native::Mui(Kind::Application), 0),
    ("Window.mui", "rootclass", Native::Mui(Kind::Window), 0),
    ("Group.mui", "rootclass", Native::Mui(Kind::Group), 0),
    ("Text.mui", "rootclass", Native::Mui(Kind::Text), 0),
    ("Menustrip.mui", "rootclass", Native::Mui(Kind::Menustrip), 0),
    ("Menu.mui", "rootclass", Native::Mui(Kind::Menu), 0),
    ("Menuitem.mui", "rootclass", Native::Mui(Kind::Menuitem), 0),
];
const CLASS_TABLE: u32 = BOOPSI_BASE + 0x100;
const CLASS_SLOT: u32 = 64;

/// The guest address of a native class.
pub fn native_addr(n: Native) -> u32 {
    CLASS_TABLE + CLASS_SLOT * NATIVE.iter().position(|e| e.2 == n).unwrap_or(0) as u32
}

pub struct ClassRec {
    pub name: Option<String>, pub native: Option<Native>, pub sup: u32,
    pub inst_offset: u32, pub inst_size: u32,
    pub subclasses: u32, pub objects: u32, pub public: bool,
}
impl ClassRec { pub fn object_size(&self) -> u32 { OBJ_HEADER + self.inst_offset + self.inst_size } }

/// A live object made by the BOOPSI core: its class and its heap block.
pub struct ObjRec { pub class: u32, pub block: u32, pub size: u32 }

#[derive(Default)]
pub struct Classes {
    pub recs: BTreeMap<u32, ClassRec>, pub names: BTreeMap<String, u32>, pub objs: BTreeMap<u32, ObjRec>,
    /// strgclass buffers it allocated for a string gadget (freed on OM_DISPOSE): gadget -> (block, size).
    pub strg_bufs: BTreeMap<u32, alloc::vec::Vec<(u32, u32)>>,
    /// sysiclass object -> its SYSIA_Which (boopsi_draw.rs).
    pub sysi_type: BTreeMap<u32, u32>,
}

impl Classes {
    pub fn find(&self, name: &str) -> Option<u32> { self.names.get(name).copied() }
    /// The class of a live object, from host records (not from the guest-writable o_Class).
    pub fn class_of(&self, obj: u32) -> Option<u32> { self.objs.get(&obj).map(|o| o.class) }
    pub fn name_of(&self, cl: u32) -> String {
        self.recs.get(&cl).and_then(|r| r.name.clone()).unwrap_or_else(|| format!("<private class {cl:#x}>"))
    }
}

fn wr(mem: &mut GuestMem, at: u32, v: u32) -> Result<(), Refusal> { mem.write_u32(at, v).map_err(Refusal::mem("IClass")) }

/// Fill a class struct (already zeroed) and register it.
fn fill<U: GuestUi>(a: &mut Amiga<U>, cl: u32, rec: ClassRec) -> Result<(), Refusal> {
    let e = Refusal::mem("IClass");
    a.mem.write_u32(cl + CL_SUPER, rec.sup).map_err(&e)?;
    a.mem.write_u16(cl + CL_INST_OFFSET, rec.inst_offset as u16).map_err(&e)?;
    a.mem.write_u16(cl + CL_INST_SIZE, rec.inst_size as u16).map_err(&e)?;
    // rootclass keeps cl_ObjectSize 0 on a real AROS (oracle: tests/oracle/boopsiprobe.aros); the host record still knows its size
    a.mem.write_u32(cl + CL_OBJECT_SIZE, if rec.sup == 0 { 0 } else { rec.object_size() }).map_err(&e)?;
    if rec.sup != 0 { if let Some(s) = a.classes.recs.get_mut(&rec.sup) { s.subclasses += 1; let n = s.subclasses; wr(&mut a.mem, rec.sup + CL_SUBCLASS_COUNT, n)?; } }
    if rec.public { if let Some(n) = &rec.name { a.classes.names.insert(n.clone(), cl); } }
    a.classes.recs.insert(cl, rec);
    Ok(())
}

/// Build the native classes (called once from machine construction, after boopsi::install made the trap slot).
pub fn init<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    for (i, (name, sup, native, size)) in NATIVE.iter().enumerate() {
        let cl = CLASS_TABLE + CLASS_SLOT * i as u32;
        let sup_addr = if sup.is_empty() { 0 } else { a.classes.find(sup).ok_or_else(|| Refusal(format!("native class {name}: superclass {sup} missing")))? };
        let off = a.classes.recs.get(&sup_addr).map(|s| s.inst_offset + s.inst_size).unwrap_or(0);
        a.mem.zero(cl, CL_SIZE).map_err(Refusal::mem("IClass"))?;
        wr(&mut a.mem, cl + CL_H_ENTRY, crate::boopsi::trap_entry())?;
        wr(&mut a.mem, cl + CL_H_DATA, i as u32)?;
        // cl_ID: the public name as a guest C string (programs read it: DOpus's FindBOOPSIGadget compares cl_Super->cl_ID to "gadgetclass")
        let id = crate::fs::alloc_zeroed_sys(a, name.len() as u32 + 1)?.ok_or_else(|| Refusal("native class names: engine arena full (NOMEM)".into()))?;
        a.mem.write_bytes(id, name.as_bytes()).map_err(Refusal::mem("IClass cl_ID"))?;
        wr(&mut a.mem, cl + CL_ID, id)?;
        a.mem.write_u32(cl + CL_FLAGS, CLF_INLIST).map_err(Refusal::mem("IClass"))?;
        fill(a, cl, ClassRec { name: Some((*name).into()), native: Some(*native), sup: sup_addr, inst_offset: off, inst_size: *size as u32, subclasses: 0, objects: 0, public: true })?;
    }
    Ok(())
}

fn cstr<U: GuestUi>(a: &Amiga<U>, p: u32, what: &'static str) -> Result<Option<String>, Refusal> {
    if p == 0 { return Ok(None); }
    a.mem.read_cstr(p).map(Some).map_err(Refusal::mem(what))
}

/// MakeClass(a0 = classID, a1 = superClassID, a2 = superClassPtr, d0 = instanceSize, d1 = flags) -> IClass or NULL.
pub fn make_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (id_p, sid_p, sptr, size, flags) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.dreg(0) as u16 as u32, cpu.dreg(1));
    let id = cstr(a, id_p, "MakeClass classID")?;
    let sid = cstr(a, sid_p, "MakeClass superClassID")?;
    let shown = id.clone().unwrap_or_else(|| "<private>".into());
    let null = |cpu: &mut Cpu, why: String| { cpu.set_d0(0); Ok(format!("[intuition] MakeClass \"{shown}\" -> NULL ({why})")) };
    if sptr == 0 && sid.is_none() { return null(cpu, "no superclass given".into()); }
    if let Some(n) = &id { if a.classes.find(n).is_some() { return null(cpu, "a public class of that name exists".into()); } }
    let sup = if sptr != 0 {
        if !a.classes.recs.contains_key(&sptr) { return Err(Refusal(format!("MakeClass \"{shown}\": superClassPtr {sptr:#x} is not a class"))); }
        sptr
    } else {
        match a.classes.find(sid.as_deref().unwrap_or("")) { Some(s) => s, None => return null(cpu, format!("superclass \"{}\" not found", sid.unwrap_or_default())) }
    };
    let (soff, ssz) = { let s = &a.classes.recs[&sup]; (s.inst_offset, s.inst_size) };
    let Some(cl) = a.heap.alloc(CL_SIZE) else { return null(cpu, "NOMEM".into()) };
    a.mem.zero(cl, CL_SIZE).map_err(Refusal::mem("IClass"))?;
    wr(&mut a.mem, cl + CL_ID, id_p)?;
    fill(a, cl, ClassRec { name: id, native: None, sup, inst_offset: soff + ssz, inst_size: size, subclasses: 0, objects: 0, public: false })?;
    cpu.set_d0(cl);
    Ok(format!("[intuition] MakeClass \"{shown}\" super={} size={size} flags={flags:#x} -> {cl:#x}", a.classes.name_of(sup)))
}

/// AddClass(a0 = class): make a class public.
pub fn add_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let cl = cpu.areg(0);
    let rec = a.classes.recs.get_mut(&cl).ok_or_else(|| Refusal(format!("AddClass: {cl:#x} is not a class")))?;
    let Some(name) = rec.name.clone() else { return Err(Refusal(format!("AddClass: {cl:#x} was made without a classID (private)"))) };
    if rec.native.is_some() { return Err(Refusal(format!("AddClass: {name} is a built-in class"))); }
    rec.public = true;
    a.classes.names.insert(name.clone(), cl);
    let f = a.mem.read_u32(cl + CL_FLAGS).map_err(Refusal::mem("cl_Flags"))? | CLF_INLIST;
    wr(&mut a.mem, cl + CL_FLAGS, f)?;
    Ok(format!("[intuition] AddClass \"{name}\" {cl:#x}"))
}

fn unlist<U: GuestUi>(a: &mut Amiga<U>, cl: u32) -> Result<(), Refusal> {
    if let Some(rec) = a.classes.recs.get_mut(&cl) {
        if rec.public { rec.public = false; if let Some(n) = &rec.name { a.classes.names.remove(n); } }
        let f = a.mem.read_u32(cl + CL_FLAGS).map_err(Refusal::mem("cl_Flags"))? & !CLF_INLIST;
        wr(&mut a.mem, cl + CL_FLAGS, f)?;
    }
    Ok(())
}

/// RemoveClass(a0 = class or NULL): take it off the public list (a no-op when it is not there).
pub fn remove_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let cl = cpu.areg(0);
    if cl != 0 && a.classes.recs.get(&cl).is_some_and(|r| r.native.is_some()) { return Err(Refusal(format!("RemoveClass: {} is a built-in class", a.classes.name_of(cl)))); }
    unlist(a, cl)?;
    Ok(format!("[intuition] RemoveClass {cl:#x}"))
}

/// FreeClass(a0 = class) -> TRUE when freed; FALSE when objects or subclasses still exist (it is still removed from
/// the public list, as AROS's FreeClass always calls RemoveClass).
pub fn free_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let cl = cpu.areg(0);
    let rec = a.classes.recs.get(&cl).ok_or_else(|| Refusal(format!("FreeClass: {cl:#x} is not a class")))?;
    if rec.native.is_some() { return Err(Refusal(format!("FreeClass: {} is a built-in class", a.classes.name_of(cl)))); }
    let (subs, objs, sup) = (rec.subclasses, rec.objects, rec.sup);
    unlist(a, cl)?;
    if subs != 0 || objs != 0 {
        cpu.set_d0(0);
        return Ok(format!("[intuition] FreeClass {} -> FALSE ({subs} subclasses, {objs} objects)", a.classes.name_of(cl)));
    }
    let name = a.classes.name_of(cl);
    a.classes.recs.remove(&cl);
    if let Some(s) = a.classes.recs.get_mut(&sup) { s.subclasses -= 1; let n = s.subclasses; wr(&mut a.mem, sup + CL_SUBCLASS_COUNT, n)?; }
    a.heap.free(cl, CL_SIZE).map_err(|e| Refusal(format!("FreeClass: {e}")))?;
    cpu.set_d0(1);
    Ok(format!("[intuition] FreeClass {name} -> TRUE"))
}

/// FindClass(a0 = classID) -> the public class of that name, or NULL.
pub fn find_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(0)).map_err(Refusal::mem("FindClass classID"))?;
    let cl = a.classes.find(&name).unwrap_or(0);
    cpu.set_d0(cl);
    Ok(format!("[intuition] FindClass \"{name}\" -> {cl:#x}"))
}

/// Adjust a class's live-object count (host record and the guest-visible mirror).
pub fn count_object<U: GuestUi>(a: &mut Amiga<U>, cl: u32, up: bool) -> Result<(), Refusal> {
    let rec = a.classes.recs.get_mut(&cl).ok_or_else(|| Refusal(format!("object of unknown class {cl:#x}")))?;
    rec.objects = if up { rec.objects + 1 } else { rec.objects.saturating_sub(1) };
    let n = rec.objects;
    wr(&mut a.mem, cl + CL_OBJECT_COUNT, n)
}
