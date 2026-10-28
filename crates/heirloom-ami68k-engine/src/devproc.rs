//! Resolving a name to its device, and editing the DOS list: GetDeviceProc / FreeDeviceProc / IsFileSystem and
//! MakeDosEntry / AddDosEntry / RemDosEntry / FreeDosEntry (doslist.rs owns the list itself).
//!
//! `struct DevProc` is 16 bytes (dvp_Port 0, dvp_Lock 4, dvp_Flags 8, dvp_DevNode 12; m68k gcc, AROS dosextens.h).
//! Only ASSIGNS can be added or removed by the guest: a device or volume node needs a filesystem handler task, and
//! handler packets are not modelled, so those are named refusals rather than a node that looks mounted and is not.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use crate::dos_err::*;
use crate::doslist::*;
use crate::fs::*;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const DEVPROC_SIZE: u32 = 16;
const DVPF_ASSIGN: u32 = 1 << 1;
const DL_LOCK: u32 = 12;
const DL_MISC: u32 = 16;
const DL_NAME: u32 = 40;
/// Handler names that are not file systems (dos.doc IsFileSystem): the console, the null device, raw and pipe.
const NOT_FILESYSTEMS: [&str; 5] = ["NIL", "CON", "RAW", "PIPE", "*"];

/// State: live DevProcs (address -> (name, index of the directory it points at)) and DosList nodes MakeDosEntry made.
#[derive(Default)]
pub struct DevState { procs: BTreeMap<u32, (String, usize)>, made: BTreeMap<u32, u32> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("GetDeviceProc", get_device_proc::<U>).named("FreeDeviceProc", free_device_proc::<U>).named("IsFileSystem", is_file_system::<U>)
        .named("MakeDosEntry", make_dos_entry::<U>).named("FreeDosEntry", free_dos_entry::<U>).named("AddDosEntry", add_dos_entry::<U>).named("RemDosEntry", rem_dos_entry::<U>)
}

/// The locks of assign `name` in order: the dos entry's own FileLocks (BPTRs).
fn assign_locks<U: GuestUi>(a: &Amiga<U>, name: &str) -> Option<(u32, alloc::vec::Vec<u32>)> {
    find(a, name, LDF_ASSIGNS).map(|e| (e.addr, e.lock_bptrs()))
}

/// GetDeviceProc(d1 = name, d2 = previous DevProc or 0) -> DevProc, NULL when there is no (more) device.
fn get_device_proc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, prev) = (cpu.dreg(1), cpu.dreg(2));
    let name = str_arg(a, np, "GetDeviceProc.name")?;
    let what = format!("[dos] GetDeviceProc \"{name}\"");
    sync(a)?;
    let (dev, idx) = match a.fs.devs.procs.get(&prev) {
        Some((d, i)) if prev != 0 => (d.clone(), *i + 1),
        _ if prev != 0 => return Err(Refusal(format!("GetDeviceProc: {prev:#x} is not a DevProc this call made"))),
        _ => (name.split_once(':').map(|(d, _)| String::from(d)).unwrap_or_default(), 0),
    };
    let (port, lock, flags, node) = if name.contains(':') {
        if let Some((node, locks)) = assign_locks(a, &dev) {
            let Some(&l) = locks.get(idx) else { return finish_none(a, cpu, prev, what, if idx == 0 { ERROR_DEVICE_NOT_MOUNTED } else { ERROR_NO_MORE_ENTRIES }) };
            (a.mem.read_u32(node + 8).map_err(Refusal::mem("dol_Task"))?, l, DVPF_ASSIGN, node)
        } else if let Some(e) = find(a, &dev, LDF_DEVICES) {
            if idx > 0 { return finish_none(a, cpu, prev, what, ERROR_NO_MORE_ENTRIES); }
            let node = e.addr;
            (a.mem.read_u32(node + 8).map_err(Refusal::mem("dol_Task"))?, 0, 0, node)
        } else { return finish_none(a, cpu, prev, what, ERROR_DEVICE_NOT_MOUNTED) }
    } else {
        // A relative name belongs to the current directory's device, which is the lock the Process already holds.
        if idx > 0 { return finish_none(a, cpu, prev, what, ERROR_NO_MORE_ENTRIES); }
        let dev = a.fs.vol.device_of(cur_dir(a));
        let dname = String::from(a.fs.vol.name(dev));
        let node = find(a, &dname, LDF_DEVICES).map_or(0, |e| e.addr);
        let cur = a.mem.read_u32(a.sched.cur + 152).map_err(Refusal::mem("pr_CurrentDir"))?;
        (device_port(a, &dname)?, cur, 0, node)
    };
    let dp = if prev != 0 { prev } else {
        let Some(p) = alloc_zeroed(a, DEVPROC_SIZE)? else { return fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) };
        p
    };
    let m = Refusal::mem("DevProc");
    for (off, v) in [(0, port), (4, lock), (8, flags), (12, node)] { a.mem.write_u32(dp + off, v).map_err(&m)?; }
    a.fs.devs.procs.insert(dp, (dev, idx));
    ok(a, cpu, dp, format!("{what} -> {dp:#x} (directory {idx}, flags {flags:#x})"))
}

fn finish_none<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, prev: u32, what: String, e: DosErr) -> Result<String, Refusal> {
    if prev != 0 { a.fs.devs.procs.remove(&prev); a.heap.free(prev, DEVPROC_SIZE).map_err(|x| Refusal(format!("GetDeviceProc: {x}")))?; }
    fail(a, cpu, e, 0, what)
}

/// FreeDeviceProc(d1 = DevProc).
fn free_device_proc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let dp = cpu.dreg(1);
    if dp == 0 { return Ok("[dos] FreeDeviceProc NULL".into()); }
    if a.fs.devs.procs.remove(&dp).is_none() { return Err(Refusal(format!("FreeDeviceProc {dp:#x}: not a live DevProc"))); }
    a.heap.free(dp, DEVPROC_SIZE).map_err(|x| Refusal(format!("FreeDeviceProc: {x}")))?;
    Ok(format!("[dos] FreeDeviceProc {dp:#x}"))
}

/// IsFileSystem(d1 = name) -> DOSTRUE if the named device or assign is a filesystem.
fn is_file_system<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "IsFileSystem.name")?;
    let what = format!("[dos] IsFileSystem \"{name}\"");
    let dev = name.split(':').next().unwrap_or("");
    if NOT_FILESYSTEMS.iter().any(|n| n.eq_ignore_ascii_case(dev)) { return ok(a, cpu, 0, what); }
    sync(a)?;
    if !name.contains(':') || find(a, dev, LDF_DEVICES | LDF_VOLUMES | LDF_ASSIGNS).is_some() { return ok(a, cpu, DOSTRUE, what); }
    fail(a, cpu, ERROR_DEVICE_NOT_MOUNTED, 0, what)
}

/// MakeDosEntry(d1 = name, d2 = type) -> a fresh DosList node (not in the list until AddDosEntry).
fn make_dos_entry<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (name, ty) = (str_arg(a, cpu.dreg(1), "MakeDosEntry.name")?, cpu.dreg(2));
    let what = format!("[dos] MakeDosEntry \"{name}\" type {ty}");
    let size = DL_NAME + 4 + (1 + name.len().min(255) as u32 + 3) / 4 * 4;
    let Some(p) = alloc_zeroed(a, size)? else { return fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) };
    let m = Refusal::mem("DosList");
    a.mem.write_u32(p + 4, ty).map_err(&m)?;
    a.mem.write_u32(p + DL_NAME, (p + DL_NAME + 4) >> 2).map_err(&m)?; // the BSTR follows the 44-byte node
    a.mem.write_u8(p + DL_NAME + 4, name.len().min(255) as u8).map_err(&m)?;
    a.mem.write_bytes(p + DL_NAME + 5, &name.as_bytes()[..name.len().min(255)]).map_err(&m)?;
    a.fs.devs.made.insert(p, size);
    ok(a, cpu, p, format!("{what} -> {p:#x}"))
}

/// FreeDosEntry(d1 = dlist): only nodes MakeDosEntry made.
fn free_dos_entry<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.dreg(1);
    if p == 0 { return Ok("[dos] FreeDosEntry NULL".into()); }
    let Some(size) = a.fs.devs.made.remove(&p) else { return Err(Refusal(format!("FreeDosEntry {p:#x}: not a node MakeDosEntry made (list entries belong to the system)"))) };
    a.heap.free(p, size).map_err(|x| Refusal(format!("FreeDosEntry: {x}")))?;
    Ok(format!("[dos] FreeDosEntry {p:#x}"))
}

/// AddDosEntry(d1 = dlist) -> DOSTRUE. An assign node (DLT_DIRECTORY with dol_Lock, or DLT_LATE / DLT_NONBINDING with
/// dol_AssignName) becomes an assign of the volume; a device or volume node is a named refusal.
fn add_dos_entry<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let e = cpu.dreg(1);
    let m = Refusal::mem("DosList");
    let (ty, name) = (a.mem.read_u32(e + 4).map_err(&m)?, entry_name(a, e));
    let what = format!("[dos] AddDosEntry \"{name}\" type {ty}");
    let target = match ty {
        DLT_DIRECTORY => {
            let l = a.mem.read_u32(e + DL_LOCK).map_err(&m)?;
            match lock_node(a, l) { Ok(n) => a.fs.vol.full_path(n), Err(x) => return fail(a, cpu, x, 0, what) }
        }
        DLT_LATE | DLT_NONBINDING => {
            let p = a.mem.read_u32(e + DL_MISC).map_err(&m)?;
            str_arg(a, p, "dol_AssignName")?
        }
        t => return Err(Refusal(format!("AddDosEntry of a type {t} (device/volume) node: filesystem handlers are not modelled; only assigns can be added"))),
    };
    sync(a)?;
    if find(a, &name, LDF_DEVICES | LDF_VOLUMES | LDF_ASSIGNS).is_some() || a.fs.vol.device(&name).is_some() { return fail(a, cpu, ERROR_OBJECT_EXISTS, 0, what); }
    a.fs.vol.assign(&name, &target);
    sync(a)?;
    ok(a, cpu, DOSTRUE, format!("{what} = {target}"))
}

/// RemDosEntry(d1 = dlist) -> DOSTRUE: removes an assign. A device or volume cannot be unmounted.
fn rem_dos_entry<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let e = cpu.dreg(1);
    let ty = a.mem.read_u32(e + 4).map_err(Refusal::mem("DosList"))?;
    let name = entry_name(a, e);
    let what = format!("[dos] RemDosEntry \"{name}\" type {ty}");
    if !matches!(ty, DLT_DIRECTORY | DLT_LATE | DLT_NONBINDING) { return Err(Refusal(format!("RemDosEntry of a type {ty} (device/volume) node: devices cannot be unmounted here"))); }
    if !a.fs.vol.remove_assign(&name) { return fail(a, cpu, ERROR_OBJECT_NOT_FOUND, 0, what); }
    sync(a)?;
    ok(a, cpu, DOSTRUE, what)
}
