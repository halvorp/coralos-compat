//! commodities.library: the object graph (brokers, filters, senders, translators ...) without an input stream.
//!
//! CxBroker / CreateCxObj / AttachCxObj / EnqueueCxObj / InsertCxObj / RemoveCxObj / DeleteCxObj / DeleteCxObjAll /
//! ActivateCxObj / SetCxObjPri / CxObjType / CxObjError / ClearCxObjError / SetTranslate / SetFilter / SetFilterIX keep
//! the object tree host-side, with each object a small zeroed block in the engine's arena so the handle is a unique valid
//! address whose ln_Type (offset 8) is the CX_ type, as in AROS workbench/libs/commodities (createcxobj.c, cxbroker.c,
//! attachcxobj.c, deletecxobjall.c ...). Semantics kept: new objects are ACTIVE, brokers are created inactive; a broker with
//! NBU_UNIQUE and the name of an existing one fails with CBERR_DUP; a wrong nb_Version gives CBERR_VERSION; attaching NULL
//! sets COERR_NULLATTACH on the head, and attaching to a NULL head deletes the object.
//!
//! What is NOT modelled: there is no input.device stream, so no InputEvent ever reaches a filter and a hot key never
//! fires; the CxMsg calls (CxMsgType/ID/Data, DivertCxMsg ...), MatchIX and the
//! broker list calls (CopyBrokerList, BrokerCommand) stay named refusals. SetFilter stores the description text unparsed (ParseIX itself is commodities_parse.rs).
//! Offsets (m68k-linux-gnu-gcc -m68020 -mno-align-int over AROS libraries/commodities.h, 2026-10-18): struct NewBroker is 26
//! bytes: nb_Version 0, nb_Name 2, nb_Title 6, nb_Descr 10, nb_Unique 14, nb_Flags 16, nb_Pri 18, nb_Port 20; struct IX 12.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const CX_FILTER: u32 = 1;
const CX_TYPEFILTER: u32 = 2;
const CX_SEND: u32 = 3;
const CX_SIGNAL: u32 = 4;
const CX_TRANSLATE: u32 = 5;
const CX_BROKER: u32 = 6;
const CX_DEBUG: u32 = 7;
const CX_CUSTOM: u32 = 8;
const CBERR_OK: u32 = 0;
const CBERR_DUP: u32 = 2;
const CBERR_VERSION: u32 = 3;
const NB_VERSION: u8 = 5;
const NBU_UNIQUE: u16 = 1;
const COERR_NULLATTACH: u8 = 1;
const OBJ_SIZE: u32 = 32;
const IX_SIZE: usize = 12;

struct Obj { ty: u32, parent: u32, kids: Vec<u32>, active: bool, pri: i8, err: u8, name: String, ix: Option<[u8; IX_SIZE]>, filter_text: Option<String> }

#[derive(Default)]
pub struct State { objs: BTreeMap<u32, Obj>, brokers: Vec<u32> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("CreateCxObj", create::<U>).named("CxBroker", broker::<U>).named("ActivateCxObj", activate::<U>)
        .named("DeleteCxObj", delete_one::<U>).named("DeleteCxObjAll", delete_all::<U>).named("CxObjType", obj_type::<U>)
        .named("CxObjError", obj_error::<U>).named("ClearCxObjError", clear_error::<U>).named("SetCxObjPri", set_pri::<U>)
        .named("AttachCxObj", attach::<U>).named("EnqueueCxObj", enqueue::<U>).named("InsertCxObj", insert::<U>)
        .named("RemoveCxObj", remove::<U>).named("SetTranslate", set_translate::<U>).named("SetFilter", set_filter::<U>)
        .named("SetFilterIX", set_filter_ix::<U>).with(crate::commodities_parse::register::<U>)
}

fn new_obj<U: GuestUi>(a: &mut Amiga<U>, ty: u32, active: bool, pri: i8, name: String) -> Result<u32, Refusal> {
    let Some(p) = crate::fs::alloc_zeroed_sys(a, OBJ_SIZE)? else { return Err(Refusal("commodities: no memory for a CxObj (NOMEM)".into())) };
    a.mem.write_u8(p + 8, ty as u8).map_err(Refusal::mem("CxObj.ln_Type"))?;
    a.mem.write_u8(p + 9, pri as u8).map_err(Refusal::mem("CxObj.ln_Pri"))?;
    a.cx.objs.insert(p, Obj { ty, parent: 0, kids: Vec::new(), active, pri, err: 0, name, ix: None, filter_text: None });
    Ok(p)
}

fn obj<'a, U: GuestUi>(a: &'a mut Amiga<U>, co: u32, call: &str) -> Result<&'a mut Obj, Refusal> {
    a.cx.objs.get_mut(&co).ok_or_else(|| Refusal(format!("{call}: {co:#x} is not a CxObj")))
}

/// CreateCxObj(d0 = type, a0 = arg1, a1 = arg2).
fn create<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, arg1) = (cpu.dreg(0), cpu.areg(0));
    if !matches!(ty, CX_FILTER | CX_TYPEFILTER | CX_SEND | CX_SIGNAL | CX_TRANSLATE | CX_DEBUG | CX_CUSTOM) {
        cpu.set_d0(0);
        return Ok(format!("[commodities] CreateCxObj type {ty}: not a creatable type -> NULL (CxBroker makes brokers)"));
    }
    let co = new_obj(a, ty, true, 0, String::new())?;
    if ty == CX_FILTER && arg1 != 0 { let t = a.mem.read_cstr(arg1).map_err(Refusal::mem("CxFilter description"))?; obj(a, co, "CreateCxObj")?.filter_text = Some(t); }
    cpu.set_d0(co);
    Ok(format!("[commodities] CreateCxObj type {ty} -> {co:#x}"))
}

/// CxBroker(a0 = NewBroker, d0 = &error or NULL) -> the broker object (inactive), or NULL with *error.
fn broker<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (nb, errp) = (cpu.areg(0), cpu.dreg(0));
    let m = Refusal::mem("NewBroker");
    let version = a.mem.read_u8(nb).map_err(&m)?;
    let name_p = a.mem.read_u32(nb + 2).map_err(&m)?;
    let name = if name_p == 0 { String::new() } else { a.mem.read_cstr(name_p).map_err(&m)? };
    let (unique, pri) = (a.mem.read_u16(nb + 14).map_err(&m)?, a.mem.read_u8(nb + 18).map_err(&m)? as i8);
    let (co, err) = if version != NB_VERSION { (0, CBERR_VERSION) }
        else if unique & NBU_UNIQUE != 0 && a.cx.brokers.iter().any(|b| a.cx.objs.get(b).map_or(false, |o| o.name == name)) { (0, CBERR_DUP) }
        else {
            let co = new_obj(a, CX_BROKER, false, pri, name.clone())?;
            // Enqueue by priority, FIFO among equals.
            let at = a.cx.brokers.iter().position(|b| a.cx.objs.get(b).map_or(false, |o| o.pri < pri)).unwrap_or(a.cx.brokers.len());
            a.cx.brokers.insert(at, co);
            (co, CBERR_OK)
        };
    if errp != 0 { a.mem.write_u32(errp, err).map_err(Refusal::mem("CxBroker error"))?; }
    cpu.set_d0(co);
    Ok(format!("[commodities] CxBroker \"{name}\" -> {co:#x} (error {err})"))
}

/// ActivateCxObj(a0 = object, d0 = on/off) -> whether it was active.
fn activate<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (co, on) = (cpu.areg(0), cpu.dreg(0) != 0);
    if co == 0 { cpu.set_d0(0); return Ok("[commodities] ActivateCxObj NULL".into()); }
    let o = obj(a, co, "ActivateCxObj")?;
    let was = o.active;
    o.active = on;
    cpu.set_d0(was as u32);
    Ok(format!("[commodities] ActivateCxObj {co:#x} {on} (was {was})"))
}

fn detach<U: GuestUi>(a: &mut Amiga<U>, co: u32) {
    let parent = a.cx.objs.get(&co).map_or(0, |o| o.parent);
    if parent != 0 { if let Some(p) = a.cx.objs.get_mut(&parent) { p.kids.retain(|&k| k != co); } }
    if let Some(o) = a.cx.objs.get_mut(&co) { o.parent = 0; }
}

fn free_obj<U: GuestUi>(a: &mut Amiga<U>, co: u32) -> Result<(), Refusal> {
    detach(a, co);
    a.cx.brokers.retain(|&b| b != co);
    a.cx.objs.remove(&co);
    crate::fs::free_block(a, co, OBJ_SIZE).map_err(|e| Refusal(format!("commodities: {e}")))
}

fn free_tree<U: GuestUi>(a: &mut Amiga<U>, co: u32) -> Result<(), Refusal> {
    let kids = a.cx.objs.get(&co).map(|o| o.kids.clone()).unwrap_or_default();
    for k in kids { free_tree(a, k)?; }
    free_obj(a, co)
}

/// DeleteCxObj(a0): this object only (its children are orphaned, as in AROS).
fn delete_one<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let co = cpu.areg(0);
    if co == 0 { return Ok("[commodities] DeleteCxObj NULL".into()); }
    obj(a, co, "DeleteCxObj")?;
    let kids = a.cx.objs.get(&co).map(|o| o.kids.clone()).unwrap_or_default();
    for k in kids { if let Some(o) = a.cx.objs.get_mut(&k) { o.parent = 0; } }
    free_obj(a, co)?;
    Ok(format!("[commodities] DeleteCxObj {co:#x}"))
}

/// DeleteCxObjAll(a0): the object and everything attached to it.
fn delete_all<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let co = cpu.areg(0);
    if co == 0 { return Ok("[commodities] DeleteCxObjAll NULL".into()); }
    obj(a, co, "DeleteCxObjAll")?;
    free_tree(a, co)?;
    Ok(format!("[commodities] DeleteCxObjAll {co:#x}"))
}

fn obj_type<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let co = cpu.areg(0);
    let ty = if co == 0 { 0 } else { obj(a, co, "CxObjType")?.ty };
    cpu.set_d0(ty);
    Ok(format!("[commodities] CxObjType {co:#x} -> {ty}"))
}

fn obj_error<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let co = cpu.areg(0);
    let e = if co == 0 { 0 } else { obj(a, co, "CxObjError")?.err };
    cpu.set_d0(e as u32);
    Ok(format!("[commodities] CxObjError {co:#x} -> {e}"))
}

fn clear_error<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let co = cpu.areg(0);
    if co != 0 { obj(a, co, "ClearCxObjError")?.err = 0; }
    Ok(format!("[commodities] ClearCxObjError {co:#x}"))
}

/// SetCxObjPri(a0, d0) -> the old priority.
fn set_pri<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (co, pri) = (cpu.areg(0), cpu.dreg(0) as i8);
    if co == 0 { cpu.set_d0(0); return Ok("[commodities] SetCxObjPri NULL".into()); }
    let o = obj(a, co, "SetCxObjPri")?;
    let old = o.pri;
    o.pri = pri;
    a.mem.write_u8(co + 9, pri as u8).map_err(Refusal::mem("CxObj.ln_Pri"))?;
    cpu.set_d0(old as u8 as u32);
    Ok(format!("[commodities] SetCxObjPri {co:#x} {old} -> {pri}"))
}

fn link<U: GuestUi>(a: &mut Amiga<U>, head: u32, co: u32, at: usize) {
    detach(a, co);
    if let Some(h) = a.cx.objs.get_mut(&head) { let at = at.min(h.kids.len()); h.kids.insert(at, co); }
    if let Some(o) = a.cx.objs.get_mut(&co) { o.parent = head; }
}

/// AttachCxObj(a0 = head, a1 = object): at the end of head's list. NULL head deletes the object; NULL object flags the head.
fn attach<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (head, co) = (cpu.areg(0), cpu.areg(1));
    if head == 0 { if co != 0 { obj(a, co, "AttachCxObj")?; free_tree(a, co)?; } return Ok("[commodities] AttachCxObj to NULL: the object is deleted".into()); }
    obj(a, head, "AttachCxObj")?;
    if co == 0 { obj(a, head, "AttachCxObj")?.err |= COERR_NULLATTACH; return Ok(format!("[commodities] AttachCxObj {head:#x} NULL: COERR_NULLATTACH")); }
    obj(a, co, "AttachCxObj")?;
    let n = a.cx.objs[&head].kids.len();
    link(a, head, co, n);
    Ok(format!("[commodities] AttachCxObj {head:#x} <- {co:#x}"))
}

/// EnqueueCxObj(a0 = head, a1 = object): before the first child of lower priority.
fn enqueue<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (head, co) = (cpu.areg(0), cpu.areg(1));
    let pri = obj(a, co, "EnqueueCxObj")?.pri;
    let kids = obj(a, head, "EnqueueCxObj")?.kids.clone();
    let at = kids.iter().position(|k| a.cx.objs.get(k).map_or(false, |o| o.pri < pri)).unwrap_or(kids.len());
    link(a, head, co, at);
    Ok(format!("[commodities] EnqueueCxObj {head:#x} <- {co:#x}"))
}

/// InsertCxObj(a0 = head, a1 = object, a2 = predecessor or NULL = first).
fn insert<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (head, co, pred) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    obj(a, co, "InsertCxObj")?;
    let kids = obj(a, head, "InsertCxObj")?.kids.clone();
    let at = if pred == 0 { 0 } else { kids.iter().position(|&k| k == pred).ok_or_else(|| Refusal(format!("InsertCxObj: {pred:#x} is not attached to {head:#x}")))? + 1 };
    link(a, head, co, at);
    Ok(format!("[commodities] InsertCxObj {head:#x} <- {co:#x} after {pred:#x}"))
}

fn remove<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let co = cpu.areg(0);
    if co == 0 { return Ok("[commodities] RemoveCxObj NULL".into()); }
    obj(a, co, "RemoveCxObj")?;
    detach(a, co);
    Ok(format!("[commodities] RemoveCxObj {co:#x}"))
}

/// SetTranslate(a0 = translator, a1 = InputEvent chain).
fn set_translate<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (co, ie) = (cpu.areg(0), cpu.areg(1));
    if co != 0 { let o = obj(a, co, "SetTranslate")?; if o.ty != CX_TRANSLATE { return Err(Refusal(format!("SetTranslate: {co:#x} is not a translator (type {})", o.ty))); } }
    Ok(format!("[commodities] SetTranslate {co:#x} ie={ie:#x} (never fires: no input events)"))
}

/// SetFilter(a0 = filter, a1 = description): stored unparsed.
fn set_filter<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (co, text) = (cpu.areg(0), cpu.areg(1));
    if co == 0 { return Ok("[commodities] SetFilter NULL".into()); }
    let t = if text == 0 { None } else { Some(a.mem.read_cstr(text).map_err(Refusal::mem("SetFilter.description"))?) };
    let o = obj(a, co, "SetFilter")?;
    if o.ty != CX_FILTER { return Err(Refusal(format!("SetFilter: {co:#x} is not a filter (type {})", o.ty))); }
    o.filter_text = t;
    Ok(format!("[commodities] SetFilter {co:#x} (description stored, not parsed)"))
}

/// SetFilterIX(a0 = filter, a1 = IX): copies the 12-byte expression.
fn set_filter_ix<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (co, ix) = (cpu.areg(0), cpu.areg(1));
    if co == 0 { return Ok("[commodities] SetFilterIX NULL".into()); }
    let mut b = [0u8; IX_SIZE];
    for (i, x) in b.iter_mut().enumerate() { *x = a.mem.read_u8(ix + i as u32).map_err(Refusal::mem("IX"))?; }
    let o = obj(a, co, "SetFilterIX")?;
    if o.ty != CX_FILTER { return Err(Refusal(format!("SetFilterIX: {co:#x} is not a filter (type {})", o.ty))); }
    o.ix = Some(b);
    Ok(format!("[commodities] SetFilterIX {co:#x} class {} code {:#x}", b[1], u16::from_be_bytes([b[2], b[3]])))
}
