//! AmigaDOS variables: GetVar / SetVar / DeleteVar / FindVar.
//!
//! LOCAL variables live on the Process's `pr_LocalVars` MinList as real `struct LocalVar` nodes (dos/var.h: a Node
//! with ln_Type = LV_VAR/LV_ALIAS and ln_Name -> the name, then lv_Flags, lv_Value, lv_Len), kept in alphabetical
//! order as the header says. The HOST holds the authoritative copy (name, type, value), so a guest that scribbles on a
//! node cannot change what GetVar answers; the node is a faithful mirror for programs that walk the list or call
//! FindVar and read lv_Value. GLOBAL variables are files in `ENV:` (a RAM-backed directory), with `ENVARC:` as the
//! saved copy (GVF_SAVE_VAR writes both). Offsets: m68k gcc -m68020 -mno-align-int over the AROS dos/dosextens.h and
//! dos/var.h field lists (2026-10-18): Process.pr_LocalVars 208 (a MinList: lh_Head 0, lh_Tail 4, lh_TailPred 8),
//! LocalVar 24 bytes (ln_Type 8, ln_Name 10, lv_Flags 14, lv_Value 16, lv_Len 20).
//!
//! Text variables end at their first newline unless GVF_BINARY_VAR; the value is NUL-terminated unless
//! GVF_DONT_NULL_TERM|GVF_BINARY_VAR. A value that does not fit the buffer is truncated and terminated, GetVar returns
//! -1 and IoErr is ERROR_BUFFER_OVERFLOW (dos.doc as remembered; AROS returns the truncated length instead - not
//! verified against Kickstart). On success IoErr() is the variable's full length.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::dos_err::*;
use crate::fs::*;
use crate::guest_str::{read_bytes_z, write_z};
use crate::libtable::LibraryBuilder;
use crate::{exec_wait, lists, machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const PR_LOCAL_VARS: u32 = 208;
const LV_SIZE: u32 = 24;
const LV_VAR: u8 = 0;
const GVF_GLOBAL_ONLY: u32 = 1 << 8;
const GVF_LOCAL_ONLY: u32 = 1 << 9;
const GVF_BINARY_VAR: u32 = 1 << 10;
const GVF_DONT_NULL_TERM: u32 = 1 << 11;
const GVF_SAVE_VAR: u32 = 1 << 12;
const ERROR_BAD_NUMBER: DosErr = DosErr(115);
const ERROR_BUFFER_OVERFLOW: DosErr = DosErr(303);
const MAX: usize = 0x1_0000;

pub struct LocalVarRec { pub name: Vec<u8>, pub ty: u8, pub value: Vec<u8>, node: u32, block: u32 }

/// The Process's local variables, sorted by name (case-insensitive), then type.
#[derive(Default)]
pub struct Vars { pub locals: Vec<LocalVarRec> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("GetVar", get_var::<U>).named("SetVar", set_var::<U>).named("DeleteVar", delete_var::<U>).named("FindVar", find_var::<U>)
}

fn key(n: &[u8]) -> Vec<u8> { n.to_ascii_lowercase() }

fn list_ready<U: GuestUi>(a: &mut Amiga<U>) -> Result<u32, Refusal> {
    let l = a.sched.cur + PR_LOCAL_VARS;
    if a.mem.read_u32(l).map_err(Refusal::mem("pr_LocalVars"))? == 0 {
        a.mem.write_u32(l, l + 4).map_err(Refusal::mem("pr_LocalVars"))?;
        a.mem.write_u32(l + 4, 0).map_err(Refusal::mem("pr_LocalVars"))?;
        a.mem.write_u32(l + 8, l).map_err(Refusal::mem("pr_LocalVars"))?;
    }
    Ok(l)
}

fn position<U: GuestUi>(a: &Amiga<U>, name: &[u8], ty: u8) -> Result<usize, usize> {
    a.fs.vars.locals.binary_search_by(|r| (key(&r.name), r.ty).cmp(&(key(name), ty)))
}

/// Set (or replace) the local variable. Ok(false) = out of guest memory.
pub fn set_local<U: GuestUi>(a: &mut Amiga<U>, name: &[u8], ty: u8, value: &[u8]) -> Result<bool, Refusal> {
    let list = list_ready(a)?;
    if let Ok(i) = position(a, name, ty) { drop_local(a, i)?; }
    let size = LV_SIZE + name.len() as u32 + 1 + value.len() as u32 + 1;
    let Some(blk) = a.heap.alloc(size) else { return Ok(false) };
    let (name_at, value_at) = (blk + LV_SIZE, blk + LV_SIZE + name.len() as u32 + 1);
    let m = Refusal::mem("LocalVar");
    a.mem.zero(blk, size).map_err(&m)?;
    write_z(&mut a.mem, name_at, name).map_err(&m)?;
    write_z(&mut a.mem, value_at, value).map_err(&m)?;
    a.mem.write_u8(blk + 8, ty).map_err(&m)?;
    a.mem.write_u32(blk + 10, name_at).map_err(&m)?;
    a.mem.write_u32(blk + 16, value_at).map_err(&m)?;
    a.mem.write_u32(blk + 20, value.len() as u32).map_err(&m)?;
    let at = position(a, name, ty).unwrap_or_else(|i| i);
    let pred = if at == 0 { list } else { a.fs.vars.locals[at - 1].node };
    lists::link_after(a, pred, blk)?;
    a.fs.vars.locals.insert(at, LocalVarRec { name: name.to_vec(), ty, value: value.to_vec(), node: blk, block: blk });
    Ok(true)
}

fn drop_local<U: GuestUi>(a: &mut Amiga<U>, i: usize) -> Result<(), Refusal> {
    let r = a.fs.vars.locals.remove(i);
    exec_wait::remove(a, r.node)?;
    let size = LV_SIZE + r.name.len() as u32 + 1 + r.value.len() as u32 + 1;
    a.heap.free(r.block, size).map_err(|e| Refusal(format!("variable node: {e}")))
}

/// Free every local variable of a process that ended (`vars` is its host-side table; the nodes are heap blocks).
pub fn free_locals<U: GuestUi>(a: &mut Amiga<U>, vars: Vars) {
    for r in vars.locals {
        let size = LV_SIZE + r.name.len() as u32 + 1 + r.value.len() as u32 + 1;
        if let Err(e) = a.heap.free(r.block, size) { a.log.push(format!("[dos] variable node {:#x} not freed: {e}", r.block)); }
    }
}

/// The value of a local variable.
pub fn local<'a, U: GuestUi>(a: &'a Amiga<U>, name: &[u8], ty: u8) -> Option<&'a [u8]> {
    position(a, name, ty).ok().map(|i| a.fs.vars.locals[i].value.as_slice())
}

fn global_path(dev: &str, name: &[u8]) -> String { format!("{dev}{}", String::from_utf8_lossy(name)) }

/// The contents of the global variable `name`: `ENV:name` only. (AmigaOS copies ENVARC: into ENV: at boot and never
/// reads it back; AROS's GetVar falls back to ENVARC:, which would resurrect a deleted variable.)
fn global<U: GuestUi>(a: &Amiga<U>, name: &[u8]) -> Option<Vec<u8>> {
    let n = a.fs.vol.resolve(cur_dir(a), &global_path("ENV:", name)).ok()?;
    a.fs.vol.file_data(n).map(<[u8]>::to_vec)
}

fn name_arg<U: GuestUi>(a: &Amiga<U>, p: u32, what: &str) -> Result<Vec<u8>, Refusal> { read_bytes_z(&a.mem, p, MAX).map_err(Refusal::mem(what)) }

/// GetVar(d1 = name, d2 = buffer, d3 = size, d4 = flags) -> length or -1.
fn get_var<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, buf, size, flags) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3), cpu.dreg(4));
    let name = name_arg(a, np, "GetVar.name")?;
    let what = format!("[dos] GetVar \"{}\" size {size} flags {flags:#x}", String::from_utf8_lossy(&name));
    if size == 0 || size > i32::MAX as u32 { return fail(a, cpu, ERROR_BAD_NUMBER, u32::MAX, what); }
    let ty = (flags & 0xFF) as u8;
    let mut value = if flags & GVF_GLOBAL_ONLY == 0 { local(a, &name, ty).map(<[u8]>::to_vec) } else { None };
    if value.is_none() && ty == LV_VAR && flags & GVF_LOCAL_ONLY == 0 { value = global(a, &name); }
    let Some(mut v) = value else { return fail(a, cpu, ERROR_OBJECT_NOT_FOUND, u32::MAX, what) };
    let full = v.len() as u32;
    if flags & GVF_BINARY_VAR == 0 { if let Some(nl) = v.iter().position(|&c| c == b'\n') { v.truncate(nl); } }
    let terminate = flags & GVF_BINARY_VAR == 0 || flags & GVF_DONT_NULL_TERM == 0;
    let room = size as usize - usize::from(terminate);
    let n = v.len().min(room);
    a.mem.write_bytes(buf, &v[..n]).map_err(Refusal::mem("GetVar.buffer"))?;
    if terminate { a.mem.write_u8(buf + n as u32, 0).map_err(Refusal::mem("GetVar.buffer"))?; }
    if n < v.len() { return fail(a, cpu, ERROR_BUFFER_OVERFLOW, u32::MAX, format!("{what} (value is {} bytes: truncated)", v.len())); }
    set_ioerr(&mut a.mem, DosErr(full));
    cpu.set_d0(n as u32);
    Ok(format!("{what} -> {n} bytes"))
}

/// SetVar(d1 = name, d2 = buffer, d3 = size (-1: a C string), d4 = flags) -> DOSTRUE.
fn set_var<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, buf, size, flags) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3), cpu.dreg(4));
    let name = name_arg(a, np, "SetVar.name")?;
    let value = if size == u32::MAX { read_bytes_z(&a.mem, buf, MAX).map_err(Refusal::mem("SetVar.buffer"))? } else {
        if size as usize > MAX { return Err(Refusal(format!("SetVar of {size} bytes is over the {MAX}-byte limit"))); }
        (0..size).map(|i| a.mem.read_u8(buf + i).map_err(Refusal::mem("SetVar.buffer"))).collect::<Result<_, _>>()?
    };
    let what = format!("[dos] SetVar \"{}\" = {} bytes flags {flags:#x}", String::from_utf8_lossy(&name), value.len());
    if name.is_empty() { return fail(a, cpu, ERROR_OBJECT_NOT_FOUND, 0, what); }
    if flags & GVF_GLOBAL_ONLY != 0 {
        if (flags & 0xFF) as u8 != LV_VAR { return Err(Refusal("SetVar: global aliases are not modelled".into())); }
        let mut r = write_file(a, &global_path("ENV:", &name), value.clone());
        if r.is_ok() && flags & GVF_SAVE_VAR != 0 { r = write_file(a, &global_path("ENVARC:", &name), value); }
        return match r { Ok(_) => ok(a, cpu, DOSTRUE, what), Err(e) => fail(a, cpu, e, 0, what) };
    }
    match set_local(a, &name, (flags & 0xFF) as u8, &value)? {
        true => ok(a, cpu, DOSTRUE, what),
        false => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what),
    }
}

/// DeleteVar(d1 = name, d2 = flags) -> DOSTRUE if something was removed.
fn delete_var<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, flags) = (cpu.dreg(1), cpu.dreg(2));
    let name = name_arg(a, np, "DeleteVar.name")?;
    let what = format!("[dos] DeleteVar \"{}\" flags {flags:#x}", String::from_utf8_lossy(&name));
    let ty = (flags & 0xFF) as u8;
    if flags & GVF_GLOBAL_ONLY == 0 {
        if let Ok(i) = position(a, &name, ty) { drop_local(a, i)?; return ok(a, cpu, DOSTRUE, what); }
    }
    if ty == LV_VAR && flags & GVF_LOCAL_ONLY == 0 {
        let cwd = cur_dir(a);
        let devs: &[&str] = if flags & GVF_SAVE_VAR != 0 { &["ENV:", "ENVARC:"] } else { &["ENV:"] };
        let mut r = Err(ERROR_OBJECT_NOT_FOUND);
        for d in devs {
            if let Ok(n) = a.fs.vol.resolve(cwd, &global_path(d, &name)) {
                r = if a.fs.node_in_use(n) { Err(ERROR_OBJECT_IN_USE) } else { a.fs.vol.remove(n) };
            }
        }
        if r != Err(ERROR_OBJECT_NOT_FOUND) { return match r { Ok(()) => ok(a, cpu, DOSTRUE, what), Err(e) => fail(a, cpu, e, 0, what) }; }
    }
    fail(a, cpu, ERROR_OBJECT_NOT_FOUND, 0, what)
}

/// FindVar(d1 = name, d2 = type) -> the local variable's guest LocalVar node, or NULL.
fn find_var<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, ty) = (cpu.dreg(1), cpu.dreg(2));
    let name = name_arg(a, np, "FindVar.name")?;
    let what = format!("[dos] FindVar \"{}\" type {ty}", String::from_utf8_lossy(&name));
    match position(a, &name, (ty & 0xFF) as u8) {
        Ok(i) => { let n = a.fs.vars.locals[i].node; ok(a, cpu, n, format!("{what} -> {n:#x}")) }
        Err(_) => fail(a, cpu, ERROR_OBJECT_NOT_FOUND, 0, what),
    }
}
