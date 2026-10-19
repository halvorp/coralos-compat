//! dos.library file I/O on the [`Volume`](crate::volume::Volume): Open, Close, Read, Write, Seek, SetFileSize and the
//! calls that go between locks and handles. The console streams stay in dos.rs; Read/Write/FPuts there forward any
//! other handle here. A handle is a real 44-byte guest FileHandle; its state (node, position, access) is host-side.
use crate::volume::AmigaDate;
use alloc::string::String;
use alloc::format;
use crate::dos_err::*;
use crate::dos_files::{copy_out, fill_fib};
use crate::fs::*;
use crate::volume::{Kind, NodeId};
use crate::{dos, machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// dos/dos.h MODE_*; OFFSET_*.
const MODE_READWRITE: u32 = 1004;
const MODE_OLDFILE: u32 = 1005;
const MODE_NEWFILE: u32 = 1006;
const OFFSET_BEGINNING: i32 = -1;
const OFFSET_CURRENT: i32 = 0;
const OFFSET_END: i32 = 1;
/// A -1 result in D0 (Read/Write/Seek failures).
const MINUS_ONE: u32 = 0xFFFF_FFFF;

fn is_console(name: &str) -> bool { name == "*" || name.eq_ignore_ascii_case("CONSOLE:") }

fn file_size<U: GuestUi>(a: &Amiga<U>, n: NodeId) -> u32 { a.fs.vol.file_data(n).map_or(0, |d| d.len() as u32) }

/// Open(d1 = name, d2 = mode) -> BPTR FileHandle or 0.
pub fn open<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "Open.name")?;
    let mode = cpu.dreg(2);
    let what = format!("[dos] Open \"{name}\" mode={mode}");
    if is_console(&name) { return ok(a, cpu, dos::STDOUT_FH >> 2, format!("{what} -> console")); }
    if !matches!(mode, MODE_OLDFILE | MODE_NEWFILE | MODE_READWRITE) { return fail(a, cpu, ERROR_ACTION_NOT_KNOWN, 0, what); }
    if name.eq_ignore_ascii_case("NIL:") {
        // dos.library's NIL: device: every mode opens; reads hit EOF at once, writes succeed and go nowhere.
        let Some(fh) = alloc_zeroed(a, FH_SIZE)? else { return fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) };
        a.fs.files.insert(fh, OpenFile { node: crate::volume::ROOT, pos: 0, read: true, write: true, exclusive: false, nil: true });
        return ok(a, cpu, fh >> 2, format!("{what} -> {fh:#x} (NIL:)"));
    }
    let cwd = cur_dir(a);
    let existing = a.fs.vol.resolve(cwd, &name);
    let node = match (existing, mode) {
        (Ok(n), _) if a.fs.vol.is_dir(n) => return fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what),
        (Err(e), MODE_OLDFILE) => return fail(a, cpu, e, 0, what),
        (Ok(n), _) => n,
        (Err(_), _) => {
            let made = a.fs.vol.resolve_parent(cwd, &name).and_then(|(d, leaf)| a.fs.vol.create_file(d, &leaf));
            match made { Ok(n) => { let d = crate::dos_misc::now(a); a.fs.vol.get_mut(n).set_date(d); n } Err(e) => return fail(a, cpu, e, 0, what) }
        }
    };
    let excl = mode == MODE_NEWFILE;
    let clash = (excl && a.fs.node_in_use(node)) || a.fs.files.values().any(|f| f.node == node && f.exclusive);
    if clash { return fail(a, cpu, ERROR_OBJECT_IN_USE, 0, what); }
    let write = mode != MODE_OLDFILE;
    if write && !a.fs.vol.is_writable(node) { return fail(a, cpu, ERROR_DISK_WRITE_PROTECTED, 0, what); }
    let Some(fh) = alloc_zeroed(a, FH_SIZE)? else { return fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) };
    if mode == MODE_NEWFILE { if let Some(d) = a.fs.vol.file_data_mut(node) { d.clear(); } }
    a.fs.files.insert(fh, OpenFile { node, pos: 0, read: true, write, exclusive: excl, nil: false });
    ok(a, cpu, fh >> 2, format!("{what} -> {fh:#x}"))
}

/// Close a file handle BPTR: the console streams are accepted and do nothing.
pub(crate) fn close_fh<U: GuestUi>(a: &mut Amiga<U>, b: u32) -> Result<(), Refusal> {
    // Close(0) is DOSTRUE on AmigaOS (dos.library ignores a NULL handle); the stdcio/posixc cleanup of 26 corpus programs does it.
    if b == 0 || b == dos::STDIN_FH >> 2 || b == dos::STDOUT_FH >> 2 { return Ok(()); }
    if a.fs.files.remove(&(b << 2)).is_none() { return Err(Refusal(format!("Close {:#x}: not an open file", b << 2))); }
    a.heap.free(b << 2, FH_SIZE).map_err(|e| Refusal(format!("Close: {e}")))
}

/// Close(d1 = file) -> DOSTRUE.
pub fn close<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    close_fh(a, b)?;
    ok(a, cpu, DOSTRUE, format!("[dos] Close {:#x}", b << 2))
}

pub(crate) fn handle<'a, U: GuestUi>(a: &'a mut Amiga<U>, b: u32, call: &str) -> Result<&'a mut OpenFile, Refusal> {
    a.fs.files.get_mut(&(b << 2)).ok_or_else(|| Refusal(format!("{call} on {:#x}: not an open file", b << 2)))
}

/// Read(d1 = file, d2 = buffer, d3 = length) for a volume file -> bytes read (0 = EOF), or -1.
pub fn read<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, len) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let (node, pos, readable) = { let f = handle(a, b, "Read")?; (f.node, f.pos, f.read) };
    let what = format!("[dos] Read {:#x} len={len}", b << 2);
    if !readable { return fail(a, cpu, ERROR_READ_PROTECTED, MINUS_ONE, what); }
    let data = a.fs.vol.file_data(node).unwrap_or(&[]);
    let start = (pos as usize).min(data.len());
    let end = start.saturating_add(len as usize).min(data.len());
    let chunk = data[start..end].to_vec();
    a.mem.write_bytes(buf, &chunk).map_err(Refusal::mem("Read.buffer"))?;
    handle(a, b, "Read")?.pos = end as u32;
    ok(a, cpu, chunk.len() as u32, format!("{what} -> {}", chunk.len()))
}

/// Write(d1 = file, d2 = buffer, d3 = length) for a volume file -> bytes written, or -1.
pub fn write<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, len) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let what = format!("[dos] Write {:#x} len={len}", b << 2);
    let bytes: alloc::vec::Vec<u8> = (0..len).map(|i| a.mem.read_u8(buf + i).map_err(Refusal::mem("Write.buffer"))).collect::<Result<_, _>>()?;
    if write_bytes_to(a, b, &bytes)? { ok(a, cpu, len, what) } else { let e = DosErr(a.mem.read_u32(a.sched.cur + PR_RESULT2).unwrap_or(0)); fail(a, cpu, e, MINUS_ONE, what) }
}

/// FPuts(d1 = file, d2 = string) for a volume file -> 0 on success, -1 on a write error.
pub fn fputs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, s) = (cpu.dreg(1), str_arg(a, cpu.dreg(2), "FPuts.str")?);
    let done = write_bytes_to(a, b, s.as_bytes())?;
    cpu.set_d0(if done { 0 } else { MINUS_ONE });
    Ok(format!("[dos] FPuts {:#x} \"{}\"", b << 2, s.trim_end_matches('\n')))
}

/// Store `bytes` at the handle's position; Ok(false) = refused with IoErr set (read-only handle).
pub(crate) fn write_bytes_to<U: GuestUi>(a: &mut Amiga<U>, b: u32, bytes: &[u8]) -> Result<bool, Refusal> {
    let (node, pos, writable, nil) = { let f = handle(a, b, "Write")?; (f.node, f.pos, f.write, f.nil) };
    if nil { return Ok(true); }
    if !writable { set_ioerr(&mut a.mem, ERROR_WRITE_PROTECTED); return Ok(false); }
    let data = a.fs.vol.file_data_mut(node).ok_or_else(|| Refusal("Write: the file's node is gone".into()))?;
    let at = pos as usize;
    if data.len() < at + bytes.len() { data.resize(at + bytes.len(), 0); }
    data[at..at + bytes.len()].copy_from_slice(bytes);
    handle(a, b, "Write")?.pos = (at + bytes.len()) as u32;
    let d = crate::dos_misc::now(a);
    a.fs.vol.get_mut(node).set_date(d);
    Ok(true)
}

/// Seek(d1 = file, d2 = position, d3 = mode) -> the OLD position, or -1 (ERROR_SEEK_ERROR).
pub fn seek<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, off, mode) = (cpu.dreg(1), cpu.dreg(2) as i32, cpu.dreg(3) as i32);
    let (node, pos) = { let f = handle(a, b, "Seek")?; (f.node, f.pos) };
    let what = format!("[dos] Seek {:#x} {off} mode={mode}", b << 2);
    let base = match mode { OFFSET_BEGINNING => 0, OFFSET_CURRENT => pos as i64, OFFSET_END => file_size(a, node) as i64, _ => return fail(a, cpu, ERROR_SEEK_ERROR, MINUS_ONE, what) };
    let new = base + off as i64;
    if new < 0 || new > file_size(a, node) as i64 { return fail(a, cpu, ERROR_SEEK_ERROR, MINUS_ONE, what); }
    handle(a, b, "Seek")?.pos = new as u32;
    ok(a, cpu, pos, what)
}

/// SetFileSize(d1 = file, d2 = offset, d3 = mode) -> the new size, or -1.
pub fn set_file_size<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, off, mode) = (cpu.dreg(1), cpu.dreg(2) as i32, cpu.dreg(3) as i32);
    let (node, pos, writable) = { let f = handle(a, b, "SetFileSize")?; (f.node, f.pos, f.write) };
    let what = format!("[dos] SetFileSize {:#x} {off} mode={mode}", b << 2);
    if !writable { return fail(a, cpu, ERROR_WRITE_PROTECTED, MINUS_ONE, what); }
    let base = match mode { OFFSET_BEGINNING => 0, OFFSET_CURRENT => pos as i64, OFFSET_END => file_size(a, node) as i64, _ => return fail(a, cpu, ERROR_SEEK_ERROR, MINUS_ONE, what) };
    let new = base + off as i64;
    if new < 0 { return fail(a, cpu, ERROR_SEEK_ERROR, MINUS_ONE, what); }
    if let Some(d) = a.fs.vol.file_data_mut(node) { d.resize(new as usize, 0); }
    for f in a.fs.files.values_mut().filter(|f| f.node == node) { f.pos = f.pos.min(new as u32); }
    ok(a, cpu, new as u32, what)
}

/// NameFromFH(d1 = file, d2 = buffer, d3 = length).
pub fn name_from_fh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, len) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let node = handle(a, b, "NameFromFH")?.node;
    let p = a.fs.vol.full_path(node);
    copy_out(a, cpu, buf, len, &p, format!("[dos] NameFromFH {:#x}", b << 2))
}

/// ExamineFH(d1 = file, d2 = fib) -> DOSTRUE.
pub fn examine_fh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, fib) = (cpu.dreg(1), cpu.dreg(2));
    let node = handle(a, b, "ExamineFH")?.node;
    fill_fib(a, fib, node)?;
    ok(a, cpu, DOSTRUE, format!("[dos] ExamineFH {:#x}", b << 2))
}

/// The console streams (Input() / Output()) are interactive handles of the console handler, which knows no ACTION_COPY_DIR_FH or
/// ACTION_PARENT_FH: dos.library's DupLockFromFH / ParentOfFH send the packet and get DOSFALSE with ERROR_ACTION_NOT_KNOWN (AROS
/// rom/dos/duplockfromfh.c passes the packet to fh_Type; 26 corpus programs duplicate their Output() lock).
pub(crate) fn console_refuses_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, b: u32, call: &str) -> Option<Result<String, Refusal>> {
    if b != dos::STDIN_FH >> 2 && b != dos::STDOUT_FH >> 2 { return None; }
    Some(fail(a, cpu, ERROR_ACTION_NOT_KNOWN, 0, format!("[dos] {call} {:#x} (the console handler has no directory)", b << 2)))
}

/// DupLockFromFH(d1 = file) -> a shared lock on the file.
pub fn dup_lock_from_fh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    if let Some(r) = console_refuses_lock(a, cpu, b, "DupLockFromFH") { return r; }
    let node = handle(a, b, "DupLockFromFH")?.node;
    let what = format!("[dos] DupLockFromFH {:#x}", b << 2);
    match new_lock(a, node, false)? { Some(p) => ok(a, cpu, p >> 2, what), None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) }
}

/// OpenFromLock(d1 = lock) -> a read/write handle on the locked file; the lock is consumed on success.
pub fn open_from_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let what = format!("[dos] OpenFromLock {:#x}", b << 2);
    let node = match lock_node(a, b) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    if !matches!(a.fs.vol.get(node).kind, Kind::File(_)) { return fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what); }
    let Some(fh) = alloc_zeroed(a, FH_SIZE)? else { return fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) };
    let write = a.fs.vol.is_writable(node);
    a.fs.files.insert(fh, OpenFile { node, pos: 0, read: true, write, exclusive: false, nil: false });
    a.fs.locks.remove(&(b << 2));
    a.heap.free(b << 2, LOCK_SIZE).map_err(|e| Refusal(format!("OpenFromLock: {e}")))?;
    ok(a, cpu, fh >> 2, what)
}

