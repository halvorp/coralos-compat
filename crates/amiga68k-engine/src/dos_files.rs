//! dos.library on the [`Volume`](crate::volume::Volume): locks, directory scans, deletion, renaming, names.
//! (Open/Read/Write/Seek are in dos_io.rs, path-string utilities in dos_path.rs, LoadSeg in loadseg.rs.)
//!
//! `struct FileInfoBlock` offsets: m68k-linux-gnu-gcc -m68020 -mno-align-int over the AROS dos/dos.h
//! FileInfoBlock32 field list (2026-10-18): fib_DiskKey 0, fib_DirEntryType 4, fib_FileName 8 (108 bytes),
//! fib_Protection 116, fib_EntryType 120, fib_Size 124, fib_NumBlocks 128, fib_Date 132 (3 longs), fib_Comment 144
//! (80 bytes), fib_OwnerUID 224, fib_OwnerGID 226, 260 bytes in all.
use alloc::string::String;
use alloc::format;
use crate::dos_err::*;
use crate::fs::*;
use crate::volume::{Kind, NodeId};
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const FIB_DIR_ENTRY_TYPE: u32 = 4;
const FIB_FILE_NAME: u32 = 8;
const FIB_PROTECTION: u32 = 116;
const FIB_ENTRY_TYPE: u32 = 120;
const FIB_SIZE_F: u32 = 124;
const FIB_NUM_BLOCKS: u32 = 128;
const FIB_DATE: u32 = 132;
const FIB_COMMENT: u32 = 144;
const NAME_MAX: usize = 108;
const COMMENT_MAX: usize = 80;
/// dos/dosextens.h ST_ROOT, ST_USERDIR, ST_FILE.
pub(crate) const ST_ROOT: i32 = 1;
pub(crate) const ST_USERDIR: i32 = 2;
pub(crate) const ST_FILE: i32 = -3;
/// dos/dos.h EXCLUSIVE_LOCK (-1); ACCESS_READ / SHARED_LOCK is -2.
const EXCLUSIVE_LOCK: i32 = -1;
const FIBF_DELETE: u32 = 1;
pub(crate) const DOS_FILEHANDLE: u32 = 0;
pub(crate) const DOS_EXALLCONTROL: u32 = 1;
pub(crate) const EXALLCONTROL_SIZE: u32 = 16;
const DOS_FIB: u32 = 2;
const BLOCK: usize = 512;

/// Write `s` as a C string into a `max`-byte field at `at` (truncated, NUL-terminated).
fn put_cstr<U: GuestUi>(a: &mut Amiga<U>, at: u32, s: &str, max: usize, what: &str) -> Result<(), Refusal> {
    let b = &s.as_bytes()[..s.len().min(max - 1)];
    a.mem.write_bytes(at, b).map_err(Refusal::mem(what))?;
    a.mem.write_u8(at + b.len() as u32, 0).map_err(Refusal::mem(what))
}

pub(crate) fn fill_fib<U: GuestUi>(a: &mut Amiga<U>, fib: u32, node: NodeId) -> Result<(), Refusal> {
    if fib & 3 != 0 { return Err(Refusal(format!("FileInfoBlock {fib:#x} is not longword aligned"))); }
    let m = Refusal::mem;
    a.mem.zero(fib, FIB_SIZE).map_err(m("FileInfoBlock"))?;
    let v = &a.fs.vol;
    let (ty, size) = match &v.get(node).kind {
        Kind::Dir(_) => (if v.is_device(node) { ST_ROOT } else { ST_USERDIR }, 0),
        Kind::File(d) => (ST_FILE, d.len()),
    };
    let (name, comment, prot, date) = (String::from(v.name(node)), v.get(node).comment.clone(), v.get(node).protection, v.get(node).date);
    a.mem.write_u32(fib, node as u32).map_err(m("fib_DiskKey"))?;
    a.mem.write_u32(fib + FIB_DIR_ENTRY_TYPE, ty as u32).map_err(m("fib_DirEntryType"))?;
    put_cstr(a, fib + FIB_FILE_NAME, &name, NAME_MAX, "fib_FileName")?;
    a.mem.write_u32(fib + FIB_PROTECTION, prot).map_err(m("fib_Protection"))?;
    a.mem.write_u32(fib + FIB_ENTRY_TYPE, ty as u32).map_err(m("fib_EntryType"))?;
    a.mem.write_u32(fib + FIB_SIZE_F, size as u32).map_err(m("fib_Size"))?;
    a.mem.write_u32(fib + FIB_NUM_BLOCKS, size.div_ceil(BLOCK) as u32).map_err(m("fib_NumBlocks"))?;
    for (i, d) in date.iter().enumerate() { a.mem.write_u32(fib + FIB_DATE + 4 * i as u32, *d as u32).map_err(m("fib_Date"))?; }
    put_cstr(a, fib + FIB_COMMENT, &comment, COMMENT_MAX, "fib_Comment")
}

/// Lock(d1 = name, d2 = accessMode) -> BPTR lock or 0.
pub fn lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "Lock.name")?;
    let exclusive = cpu.dreg(2) as i32 == EXCLUSIVE_LOCK;
    let what = format!("[dos] Lock \"{name}\" {}", if exclusive { "exclusive" } else { "shared" });
    let node = match a.fs.vol.resolve(cur_dir(a), &name) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    let clash = a.fs.locks.values().any(|l| l.node == node && (l.exclusive || exclusive)) || (exclusive && a.fs.node_in_use(node));
    if clash { return fail(a, cpu, ERROR_OBJECT_IN_USE, 0, what); }
    match new_lock(a, node, exclusive)? {
        Some(p) => ok(a, cpu, p >> 2, format!("{what} -> {p:#x}")),
        None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what),
    }
}

pub(crate) fn free_lock<U: GuestUi>(a: &mut Amiga<U>, bptr: u32, call: &str) -> Result<(), Refusal> {
    if bptr == 0 { return Ok(()); }
    if a.fs.locks.remove(&(bptr << 2)).is_none() { return Err(Refusal(format!("{call}: {:#x} is not a live lock", bptr << 2))); }
    free_block(a, bptr << 2, LOCK_SIZE).map_err(|e| Refusal(format!("{call}: {e}")))
}

/// UnLock(d1 = lock). The current directory's own lock cannot be released under the program.
pub fn unlock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let cur = a.mem.read_u32(crate::process::PROCESS + 152).map_err(Refusal::mem("pr_CurrentDir"))?;
    if b != 0 && b == cur { return Err(Refusal(format!("UnLock {:#x}: it is the Process's current directory", b << 2))); }
    free_lock(a, b, "UnLock")?;
    Ok(format!("[dos] UnLock {:#x}", b << 2))
}

/// DupLock(d1 = lock) -> a new shared lock on the same object (an exclusive lock cannot be duplicated).
pub fn dup_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let what = format!("[dos] DupLock {:#x}", b << 2);
    if b == 0 { return ok(a, cpu, 0, what); }
    let node = match lock_node(a, b) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    if a.fs.locks[&(b << 2)].exclusive { return fail(a, cpu, ERROR_OBJECT_IN_USE, 0, what); }
    match new_lock(a, node, false)? {
        Some(p) => ok(a, cpu, p >> 2, format!("{what} -> {p:#x}")),
        None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what),
    }
}

/// ParentDir(d1 = lock) -> a lock on the containing directory, or 0 with IoErr 0 at a device root.
pub fn parent_dir<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let what = format!("[dos] ParentDir {:#x}", b << 2);
    let node = match lock_node(a, b) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    match a.fs.vol.parent_of(node) {
        Err(_) => ok(a, cpu, 0, format!("{what} -> 0 (root)")),
        Ok(p) => match new_lock(a, p, false)? {
            Some(l) => ok(a, cpu, l >> 2, format!("{what} -> {l:#x}")),
            None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what),
        },
    }
}

/// CurrentDir(d1 = lock) -> the previous current directory lock.
pub fn current_dir<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    if let Err(e) = lock_node(a, b) { return Err(Refusal(format!("CurrentDir {:#x}: not a lock (IoErr {})", b << 2, e.0))); }
    let old = set_cur_dir(a, b)?;
    cpu.set_d0(old);
    Ok(format!("[dos] CurrentDir {:#x} (was {:#x})", b << 2, old << 2))
}

/// Examine(d1 = lock, d2 = fib) -> DOSTRUE. Starts a scan if the lock is a directory.
pub fn examine<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, fib) = (cpu.dreg(1), cpu.dreg(2));
    let what = format!("[dos] Examine {:#x}", b << 2);
    let node = match lock_node(a, b) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    fill_fib(a, fib, node)?;
    a.fs.scans.remove(&fib);
    if a.fs.vol.is_dir(node) { a.fs.scans.insert(fib, (node, 0)); }
    let shown = a.mem.read_cstr(fib + 8).unwrap_or_default();
    ok(a, cpu, DOSTRUE, format!("{what} -> \"{shown}\""))
}

/// ExNext(d1 = lock, d2 = fib) -> DOSTRUE with the next entry, or FALSE with ERROR_NO_MORE_ENTRIES.
pub fn ex_next<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, fib) = (cpu.dreg(1), cpu.dreg(2));
    let what = format!("[dos] ExNext {:#x}", b << 2);
    let node = match lock_node(a, b) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    let Some(&(dir, idx)) = a.fs.scans.get(&fib) else {
        return Err(Refusal(format!("ExNext: FileInfoBlock {fib:#x} was not set up by Examine on a directory")));
    };
    if dir != node { return fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what); }
    let Some(&child) = a.fs.vol.children(dir).get(idx) else { return fail(a, cpu, ERROR_NO_MORE_ENTRIES, 0, what) };
    fill_fib(a, fib, child)?;
    a.fs.scans.insert(fib, (dir, idx + 1));
    let shown = a.mem.read_cstr(fib + 8).unwrap_or_default();
    ok(a, cpu, DOSTRUE, format!("{what} -> \"{shown}\""))
}

/// DeleteFile(d1 = name) -> DOSTRUE.
pub fn delete_file<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "DeleteFile.name")?;
    let what = format!("[dos] DeleteFile \"{name}\"");
    let node = match a.fs.vol.resolve(cur_dir(a), &name) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    if a.fs.node_in_use(node) { return fail(a, cpu, ERROR_OBJECT_IN_USE, 0, what); }
    if a.fs.vol.get(node).protection & FIBF_DELETE != 0 { return fail(a, cpu, ERROR_DELETE_PROTECTED, 0, what); }
    match a.fs.vol.remove(node) { Ok(()) => ok(a, cpu, DOSTRUE, what), Err(e) => fail(a, cpu, e, 0, what) }
}

/// CreateDir(d1 = name) -> an exclusive lock on the new directory.
pub fn create_dir<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "CreateDir.name")?;
    let what = format!("[dos] CreateDir \"{name}\"");
    let made = a.fs.vol.resolve_parent(cur_dir(a), &name).and_then(|(d, leaf)| a.fs.vol.create_dir(d, &leaf));
    match made {
        Err(e) => fail(a, cpu, e, 0, what),
        Ok(n) => { let d = crate::dos_misc::now(a); a.fs.vol.get_mut(n).date = d; match new_lock(a, n, true)? { Some(p) => ok(a, cpu, p >> 2, what), None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) } }
    }
}

/// Rename(d1 = old, d2 = new) -> DOSTRUE.
pub fn rename<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (old, new) = (str_arg(a, cpu.dreg(1), "Rename.old")?, str_arg(a, cpu.dreg(2), "Rename.new")?);
    let what = format!("[dos] Rename \"{old}\" \"{new}\"");
    let cwd = cur_dir(a);
    let r = a.fs.vol.resolve(cwd, &old).and_then(|n| {
        if a.fs.node_in_use(n) { return Err(ERROR_OBJECT_IN_USE); }
        let (d, leaf) = a.fs.vol.resolve_parent(cwd, &new)?;
        a.fs.vol.rename(n, d, &leaf)
    });
    match r { Ok(()) => ok(a, cpu, DOSTRUE, what), Err(e) => fail(a, cpu, e, 0, what) }
}

/// SetProtection(d1 = name, d2 = mask) -> DOSTRUE.
pub fn set_protection<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "SetProtection.name")?;
    let mask = cpu.dreg(2);
    let what = format!("[dos] SetProtection \"{name}\" {mask:#x}");
    match a.fs.vol.resolve(cur_dir(a), &name) {
        Err(e) => fail(a, cpu, e, 0, what),
        Ok(n) if !a.fs.vol.is_writable(n) => fail(a, cpu, ERROR_DISK_WRITE_PROTECTED, 0, what),
        Ok(n) => { a.fs.vol.get_mut(n).protection = mask; ok(a, cpu, DOSTRUE, what) }
    }
}

/// SetComment(d1 = name, d2 = comment) -> DOSTRUE.
pub fn set_comment<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (name, c) = (str_arg(a, cpu.dreg(1), "SetComment.name")?, str_arg(a, cpu.dreg(2), "SetComment.comment")?);
    let what = format!("[dos] SetComment \"{name}\"");
    if c.len() >= COMMENT_MAX { return fail(a, cpu, ERROR_COMMENT_TOO_BIG, 0, what); }
    match a.fs.vol.resolve(cur_dir(a), &name) {
        Err(e) => fail(a, cpu, e, 0, what),
        Ok(n) if !a.fs.vol.is_writable(n) => fail(a, cpu, ERROR_DISK_WRITE_PROTECTED, 0, what),
        Ok(n) => { a.fs.vol.get_mut(n).comment = c; ok(a, cpu, DOSTRUE, what) }
    }
}

/// Copy `s` plus a NUL into the guest buffer `buf` of `len` bytes, or fail with ERROR_LINE_TOO_LONG.
pub(crate) fn copy_out<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, buf: u32, len: u32, s: &str, what: String) -> Result<String, Refusal> {
    if s.len() as u64 + 1 > len as u64 { return fail(a, cpu, ERROR_LINE_TOO_LONG, 0, what); }
    a.mem.write_bytes(buf, s.as_bytes()).map_err(Refusal::mem("path buffer"))?;
    a.mem.write_u8(buf + s.len() as u32, 0).map_err(Refusal::mem("path buffer"))?;
    ok(a, cpu, DOSTRUE, format!("{what} -> \"{s}\""))
}

/// NameFromLock(d1 = lock, d2 = buffer, d3 = length) -> DOSTRUE.
pub fn name_from_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, len) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let what = format!("[dos] NameFromLock {:#x}", b << 2);
    match lock_node(a, b) {
        Err(e) => fail(a, cpu, e, 0, what),
        Ok(n) => { let p = a.fs.vol.full_path(n); copy_out(a, cpu, buf, len, &p, what) }
    }
}

/// GetCurrentDirName(d1 = buffer, d2 = length) -> DOSTRUE. The name is derived from the current lock.
pub fn get_current_dir_name<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (buf, len) = (cpu.dreg(1), cpu.dreg(2));
    let p = a.fs.vol.full_path(cur_dir(a));
    copy_out(a, cpu, buf, len, &p, "[dos] GetCurrentDirName".into())
}

/// SameLock(d1, d2) -> LOCK_SAME (0), LOCK_SAME_VOLUME (1) or LOCK_DIFFERENT (-1).
pub fn same_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (x, y) = (cpu.dreg(1), cpu.dreg(2));
    let (Ok(nx), Ok(ny)) = (lock_node(a, x), lock_node(a, y)) else { return Err(Refusal("SameLock: not a lock".into())) };
    let r: i32 = if nx == ny { 0 } else if a.fs.vol.device_of(nx) == a.fs.vol.device_of(ny) { 1 } else { -1 };
    cpu.set_d0(r as u32);
    Ok(format!("[dos] SameLock -> {r}"))
}

/// AllocDosObject(d1 = type, d2 = tags): DOS_FIB and DOS_FILEHANDLE are modelled; the rest are named refusals.
pub fn alloc_dos_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ty = cpu.dreg(1);
    let size = match ty { DOS_FIB => FIB_SIZE, DOS_FILEHANDLE => FH_SIZE, DOS_EXALLCONTROL => EXALLCONTROL_SIZE, t => return Err(Refusal(format!("AllocDosObject type {t} not modelled"))) };
    let p = alloc_zeroed(a, size)?.unwrap_or(0);
    cpu.set_d0(p);
    Ok(format!("[dos] AllocDosObject {ty} -> {p:#x}"))
}

/// FreeDosObject(d1 = type, d2 = object).
pub fn free_dos_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, p) = (cpu.dreg(1), cpu.dreg(2));
    let size = match ty { DOS_FIB => FIB_SIZE, DOS_FILEHANDLE => FH_SIZE, DOS_EXALLCONTROL => EXALLCONTROL_SIZE, t => return Err(Refusal(format!("FreeDosObject type {t} not modelled"))) };
    if p != 0 { a.heap.free(p, size).map_err(|e| Refusal(format!("FreeDosObject: {e}")))?; a.fs.scans.remove(&p); a.fs.exall.remove(&p); }
    Ok(format!("[dos] FreeDosObject {ty} {p:#x}"))
}


/// `Process.pr_HomeDir`: field order of dos/dosextens.h after pr_CLI (172): pr_ReturnAddr 176, pr_PktWait 180,
/// pr_WindowPtr 184, pr_HomeDir 188.
const PR_HOME_DIR: u32 = 188;

/// GetProgramDir() -> the lock on the directory the program was started from (what `PROGDIR:` names), made on first use.
pub fn get_program_dir<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mut home = a.mem.read_u32(crate::process::PROCESS + PR_HOME_DIR).map_err(Refusal::mem("pr_HomeDir"))?;
    if home == 0 {
        if let Ok(n) = a.fs.vol.resolve(a.fs.vol.boot_device(), "PROGDIR:") {
            if let Some(p) = new_lock(a, n, false)? {
                home = p >> 2;
                a.mem.write_u32(crate::process::PROCESS + PR_HOME_DIR, home).map_err(Refusal::mem("pr_HomeDir"))?;
            }
        }
    }
    ok(a, cpu, home, format!("[dos] GetProgramDir -> {:#x}", home << 2))
}

/// SetProgramDir(d1 = lock) -> the previous program directory lock.
pub fn set_program_dir<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    if let Err(e) = lock_node(a, b) { return Err(Refusal(format!("SetProgramDir {:#x}: not a lock (IoErr {})", b << 2, e.0))); }
    let old = a.mem.read_u32(crate::process::PROCESS + PR_HOME_DIR).map_err(Refusal::mem("pr_HomeDir"))?;
    a.mem.write_u32(crate::process::PROCESS + PR_HOME_DIR, b).map_err(Refusal::mem("pr_HomeDir"))?;
    cpu.set_d0(old);
    Ok(format!("[dos] SetProgramDir {:#x}", b << 2))
}
