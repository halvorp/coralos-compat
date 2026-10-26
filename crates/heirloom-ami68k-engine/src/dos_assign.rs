//! AssignLock / AssignAdd / AssignPath / AssignLate / RemAssignList: define `NAME:` aliases from the guest.
//! AssignLock and AssignAdd CONSUME their lock (dos/dos.h: "the lock is owned by the system afterwards"); here the
//! lock's path is recorded in the volume's assign table and the lock struct is freed.
use alloc::string::String;
use alloc::format;
use crate::fs::*;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

fn take_lock<U: GuestUi>(a: &mut Amiga<U>, b: u32, call: &str) -> Result<Option<String>, Refusal> {
    if b == 0 { return Ok(None); }
    let node = lock_node(a, b).map_err(|e| Refusal(format!("{call}: {:#x} is not a lock (IoErr {})", b << 2, e.0)))?;
    let path = a.fs.vol.full_path(node);
    if a.fs.locks.remove(&(b << 2)).is_some() { a.heap.free(b << 2, LOCK_SIZE).map_err(|e| Refusal(format!("{call}: {e}")))?; }
    Ok(Some(path))
}

/// AssignLock(d1 = name, d2 = lock) -> DOSTRUE. A NULL lock removes the assign.
pub fn assign_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "AssignLock.name")?;
    let name = name.trim_end_matches(':');
    match take_lock(a, cpu.dreg(2), "AssignLock")? {
        Some(p) => { a.fs.vol.assign(name, &p); ok(a, cpu, DOSTRUE, format!("[dos] AssignLock {name}: -> {p}")) }
        None => { let had = a.fs.vol.remove_assign(name); ok(a, cpu, if had { DOSTRUE } else { 0 }, format!("[dos] AssignLock {name}: removed")) }
    }
}

/// AssignAdd(d1 = name, d2 = lock) -> DOSTRUE: another directory for an existing assign.
pub fn assign_add<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "AssignAdd.name")?;
    let name = name.trim_end_matches(':');
    let Some(p) = take_lock(a, cpu.dreg(2), "AssignAdd")? else { return fail(a, cpu, crate::dos_err::ERROR_INVALID_LOCK, 0, format!("[dos] AssignAdd {name}: NULL lock")) };
    a.fs.vol.assign_add(name, &p);
    ok(a, cpu, DOSTRUE, format!("[dos] AssignAdd {name}: + {p}"))
}

/// AssignPath/AssignLate(d1 = name, d2 = path): the assign is just the path (resolved at use, which is "late").
pub fn assign_path<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (name, path) = (str_arg(a, cpu.dreg(1), "AssignPath.name")?, str_arg(a, cpu.dreg(2), "AssignPath.path")?);
    let name = name.trim_end_matches(':');
    a.fs.vol.assign(name, &path);
    ok(a, cpu, DOSTRUE, format!("[dos] AssignPath {name}: -> {path}"))
}

/// RemAssignList(d1 = name, d2 = lock) -> DOSTRUE: drop one directory from a multi-directory assign.
pub fn rem_assign_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "RemAssignList.name")?;
    let name = name.trim_end_matches(':');
    let Some(p) = take_lock(a, cpu.dreg(2), "RemAssignList")? else { return Err(Refusal("RemAssignList: NULL lock".into())) };
    let removed = a.fs.vol.assign_remove_target(name, &p);
    ok(a, cpu, if removed { DOSTRUE } else { 0 }, format!("[dos] RemAssignList {name}: - {p}"))
}
