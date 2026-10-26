//! The DOS device list: `struct DosList` entries for the volume's devices, volumes and assigns, reachable the way
//! AmigaOS has it: `DosLibrary.dl_Root -> RootNode.rn_Info -> DosInfo.di_DevInfo`, and through LockDosList /
//! NextDosEntry / FindDosEntry.
//!
//! The HOST volume (`Volume`) stays the one source of truth. [`sync`] makes the guest-visible list match it whenever
//! the guest asks (and at program start): an unchanged entry keeps its address, a changed or vanished one is freed.
//! Each volume device appears as a DLT_DEVICE node and a DLT_VOLUME node of the same name (a fixed disk is both);
//! each assign as DLT_DIRECTORY with a real FileLock in dol_Lock (more directories of a multi-directory assign hang
//! off dol_assign.dol_List as AssignList nodes), or DLT_LATE with its path in dol_AssignName when the target does not
//! exist. Devices answer with a private MsgPort (dol_Task) that nothing serves: handler packets are not modelled, so
//! DoPkt and friends stay named refusals.
//!
//! Offsets: m68k gcc -m68020 -mno-align-int over the AROS dos/dosextens.h field lists (2026-10-18): DosLibrary 70
//! (dl_Root 34), RootNode 102 (rn_Time 8, rn_Info 24, rn_Flags 52, rn_RootLock 56), DosInfo 158 (di_DevInfo 4, three
//! SignalSemaphores of 46 at 20/66/112), DosList 44 (dol_Next 0, dol_Type 4, dol_Task 8, dol_Lock 12, dol_misc 16,
//! dol_Name 40), AssignList 8.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::dos_err::*;
use crate::fs::*;
use crate::libtable::LibraryBuilder;
use crate::volume::ROOT;
use crate::{machine::Amiga, machine::DOS_BASE, semaphores, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const DOS_ROOT: u32 = 0x5080;
pub const DOS_INFO: u32 = 0x5100;
const DL_TYPE: u32 = 4;
const DL_TASK: u32 = 8;
const DL_LOCK: u32 = 12;
const DL_MISC: u32 = 16;
const DL_NAME: u32 = 40;
const DL_SIZE: u32 = 44;
pub const DLT_DEVICE: u32 = 0;
pub const DLT_DIRECTORY: u32 = 1;
pub const DLT_VOLUME: u32 = 2;
pub const DLT_LATE: u32 = 3;
pub const DLT_NONBINDING: u32 = 4;
pub const LDF_WRITE: u32 = 1 << 1;
pub const LDF_DEVICES: u32 = 1 << 2;
pub const LDF_VOLUMES: u32 = 1 << 3;
pub const LDF_ASSIGNS: u32 = 1 << 4;
const ID_DOS_DISK: u32 = 0x444F_5300;
const NT_MSGPORT: u8 = 4;
const PORT_SIZE: u32 = 40;
const MAX_WALK: u32 = 1 << 16;

pub struct Entry { pub name: String, pub ty: u32, sig: Vec<String>, pub addr: u32, size: u32, locks: Vec<u32> }

#[derive(Default)]
pub struct DosListState {
    pub entries: Vec<Entry>,
    /// One private MsgPort per device, by upper-cased name.
    ports: BTreeMap<String, u32>,
    readers: u32,
    writer: bool,
}

impl DosListState {
    /// (read locks held, write lock held).
    pub fn lock_counts(&self) -> (u32, bool) { (self.readers, self.writer) }
    /// FileLocks the engine itself holds for the assigns it publishes (not the program's).
    pub fn engine_locks(&self) -> usize { self.entries.iter().map(|e| e.locks.len()).sum() }
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("LockDosList", lock_dos_list::<U>).named("AttemptLockDosList", attempt_lock_dos_list::<U>)
        .named("UnLockDosList", unlock_dos_list::<U>).named("NextDosEntry", next_dos_entry::<U>).named("FindDosEntry", find_dos_entry::<U>)
}

/// Lay out DosLibrary.dl_Root, the RootNode and the DosInfo (with its three semaphores) in their fixed places.
pub fn init<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    let m = Refusal::mem("DosInfo");
    a.mem.write_u32(DOS_BASE + 34, DOS_ROOT).map_err(&m)?;
    a.mem.write_u32(DOS_ROOT + 24, DOS_INFO >> 2).map_err(&m)?; // rn_Info
    for ss in [DOS_ROOT + 56, DOS_INFO + 20, DOS_INFO + 66, DOS_INFO + 112] { semaphores::init_struct(a, ss)?; }
    Ok(())
}

fn pad4(n: u32) -> u32 { (n + 3) & !3 }

fn port<U: GuestUi>(a: &mut Amiga<U>, dev: &str) -> Result<u32, Refusal> {
    let k = dev.to_ascii_uppercase();
    if let Some(&p) = a.fs.dos.ports.get(&k) { return Ok(p); }
    let p = a.sys_heap.alloc(PORT_SIZE).ok_or_else(|| Refusal("DOS list: out of guest memory (NOMEM)".into()))?;
    a.mem.zero(p, PORT_SIZE).map_err(Refusal::mem("device port"))?;
    a.mem.write_u8(p + 8, NT_MSGPORT).map_err(Refusal::mem("device port"))?;
    a.fs.dos.ports.insert(k, p);
    Ok(p)
}

/// The MsgPort of the device `dev` (a name or `NAME:`).
pub fn device_port<U: GuestUi>(a: &mut Amiga<U>, dev: &str) -> Result<u32, Refusal> { port(a, dev.trim_end_matches(':')) }

/// The device whose private MsgPort (from GetDeviceProc / DosList nodes) is `port`, if any.
pub fn device_of_port<U: GuestUi>(a: &Amiga<U>, port: u32) -> Option<String> {
    a.fs.dos.ports.iter().find(|(_, &p)| p == port).map(|(k, _)| k.clone())
}

struct Want { name: String, ty: u32, sig: Vec<String> }

fn wanted<U: GuestUi>(a: &Amiga<U>) -> Vec<Want> {
    let v = &a.fs.vol;
    let devs: Vec<String> = v.children(ROOT).iter().map(|&d| String::from(v.name(d))).collect();
    let mut w: Vec<Want> = devs.iter().map(|n| Want { name: n.clone(), ty: DLT_DEVICE, sig: vec![] }).collect();
    w.extend(devs.iter().map(|n| Want { name: n.clone(), ty: DLT_VOLUME, sig: vec![] }));
    for (name, targets) in v.assign_list() {
        let live = targets.iter().any(|t| v.resolve(ROOT, t).is_ok());
        w.push(Want { name, ty: if live { DLT_DIRECTORY } else { DLT_LATE }, sig: targets });
    }
    w
}

fn build<U: GuestUi>(a: &mut Amiga<U>, w: &Want) -> Result<Entry, Refusal> {
    let nomem = || Refusal("DOS list: out of guest memory (NOMEM)".into());
    let m = Refusal::mem("DosList");
    let targets: Vec<crate::volume::NodeId> = if w.ty == DLT_DIRECTORY { w.sig.iter().filter_map(|t| a.fs.vol.resolve(ROOT, t).ok()).collect() } else { vec![] };
    let late = if w.ty == DLT_LATE { w.sig.first().cloned().unwrap_or_default() } else { String::new() };
    let name_at = DL_SIZE;
    let late_at = name_at + pad4(1 + w.name.len() as u32);
    let list_at = late_at + if w.ty == DLT_LATE { pad4(late.len() as u32 + 1) } else { 0 };
    let size = list_at + 8 * targets.len().saturating_sub(1) as u32;
    let addr = a.sys_heap.alloc(size).ok_or_else(nomem)?;
    a.mem.zero(addr, size).map_err(&m)?;
    a.mem.write_u32(addr + DL_TYPE, w.ty).map_err(&m)?;
    a.mem.write_u8(addr + name_at, w.name.len().min(255) as u8).map_err(&m)?;
    a.mem.write_bytes(addr + name_at + 1, &w.name.as_bytes()[..w.name.len().min(255)]).map_err(&m)?;
    a.mem.write_u32(addr + DL_NAME, (addr + name_at) >> 2).map_err(&m)?;
    let mut locks = Vec::new();
    match w.ty {
        DLT_DEVICE | DLT_VOLUME => {
            let p = port(a, &w.name)?;
            a.mem.write_u32(addr + DL_TASK, p).map_err(&m)?;
            if w.ty == DLT_VOLUME {
                for (i, d) in crate::dos_misc::now(a).iter().enumerate() { a.mem.write_u32(addr + DL_MISC + 4 * i as u32, *d as u32).map_err(&m)?; }
                a.mem.write_u32(addr + DL_MISC + 16, ID_DOS_DISK).map_err(&m)?; // dol_DiskType
            }
        }
        DLT_DIRECTORY => {
            let mut prev_next_field = None;
            for (i, &n) in targets.iter().enumerate() {
                let l = new_lock_in(a, n, false, true)?.ok_or_else(nomem)?;
                locks.push(l);
                if i == 0 {
                    a.mem.write_u32(addr + DL_LOCK, l >> 2).map_err(&m)?;
                    let dev = a.fs.vol.device_of(n);
                    let p = port(a, &String::from(a.fs.vol.name(dev)))?;
                    a.mem.write_u32(addr + DL_TASK, p).map_err(&m)?;
                } else {
                    let node = addr + list_at + 8 * (i as u32 - 1);
                    a.mem.write_u32(node + 4, l >> 2).map_err(&m)?;
                    let link_at = prev_next_field.unwrap_or(addr + DL_MISC + 4); // dol_assign.dol_List, then each al_Next
                    a.mem.write_u32(link_at, node).map_err(&m)?;
                    prev_next_field = Some(node);
                }
            }
        }
        _ => {
            a.mem.write_bytes(addr + late_at, late.as_bytes()).map_err(&m)?;
            a.mem.write_u32(addr + DL_MISC, addr + late_at).map_err(&m)?; // dol_AssignName
        }
    }
    Ok(Entry { name: w.name.clone(), ty: w.ty, sig: w.sig.clone(), addr, size, locks })
}

fn release<U: GuestUi>(a: &mut Amiga<U>, e: Entry) -> Result<(), Refusal> {
    for l in e.locks { if a.fs.locks.contains_key(&l) { crate::dos_files::free_lock(a, l >> 2, "DOS list")?; } }
    a.sys_heap.free(e.addr, e.size).map_err(|x| Refusal(format!("DOS list entry: {x}")))
}

/// Make the guest-visible list match the volume (see the module doc).
pub fn sync<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    let want = wanted(a);
    let mut old = core::mem::take(&mut a.fs.dos.entries);
    let mut new: Vec<Entry> = Vec::new();
    for w in &want {
        match old.iter().position(|e| e.ty == w.ty && e.name == w.name && e.sig == w.sig) {
            Some(i) => new.push(old.remove(i)),
            None => new.push(build(a, w)?),
        }
    }
    for e in old { release(a, e)?; }
    let m = Refusal::mem("DosInfo");
    for i in 0..new.len() {
        let next = new.get(i + 1).map_or(0, |n| n.addr >> 2);
        a.mem.write_u32(new[i].addr, next).map_err(&m)?;
    }
    a.mem.write_u32(DOS_INFO + 4, new.first().map_or(0, |e| e.addr >> 2)).map_err(&m)?; // di_DevInfo
    a.fs.dos.entries = new;
    Ok(())
}

/// The DLT_VOLUME node of device `dev`, if any (what InfoData.id_VolumeNode points at).
pub fn volume_node<U: GuestUi>(a: &Amiga<U>, dev: &str) -> Option<u32> {
    a.fs.dos.entries.iter().find(|e| e.ty == DLT_VOLUME && e.name.eq_ignore_ascii_case(dev)).map(|e| e.addr)
}

fn conflicts<U: GuestUi>(a: &Amiga<U>, flags: u32) -> bool { a.fs.dos.writer || (flags & LDF_WRITE != 0 && a.fs.dos.readers > 0) }

fn take<U: GuestUi>(a: &mut Amiga<U>, flags: u32) {
    if flags & LDF_WRITE != 0 { a.fs.dos.writer = true } else { a.fs.dos.readers += 1 }
}

/// LockDosList(d1 = flags) -> the list head (the address of DosInfo.di_DevInfo, whose first longword is the first
/// entry's BPTR, so NextDosEntry(head) reads dol_Next at offset 0 exactly as for a real entry).
fn lock_dos_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let flags = cpu.dreg(1);
    if conflicts(a, flags) { return Err(Refusal(format!("LockDosList({flags:#x}): this task already holds the list; it would wait for itself forever"))); }
    sync(a)?;
    take(a, flags);
    cpu.set_d0(DOS_INFO + 4);
    Ok(format!("[dos] LockDosList {flags:#x} -> head {:#x} ({} entries)", DOS_INFO + 4, a.fs.dos.entries.len()))
}

/// AttemptLockDosList(d1 = flags): like LockDosList, but NULL instead of waiting.
fn attempt_lock_dos_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let flags = cpu.dreg(1);
    if conflicts(a, flags) { cpu.set_d0(0); return Ok(format!("[dos] AttemptLockDosList {flags:#x} -> NULL (held)")); }
    sync(a)?;
    take(a, flags);
    cpu.set_d0(DOS_INFO + 4);
    Ok(format!("[dos] AttemptLockDosList {flags:#x} -> head"))
}

/// UnLockDosList(d1 = flags): the flags of the matching LockDosList.
fn unlock_dos_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let flags = cpu.dreg(1);
    if flags & LDF_WRITE != 0 {
        if !a.fs.dos.writer { return Err(Refusal(format!("UnLockDosList({flags:#x}): the list is not write-locked"))); }
        a.fs.dos.writer = false;
    } else {
        if a.fs.dos.readers == 0 { return Err(Refusal(format!("UnLockDosList({flags:#x}): the list is not locked"))); }
        a.fs.dos.readers -= 1;
    }
    Ok(format!("[dos] UnLockDosList {flags:#x}"))
}

fn type_wanted(ty: u32, flags: u32) -> bool {
    match ty {
        DLT_DEVICE => flags & LDF_DEVICES != 0,
        DLT_VOLUME => flags & LDF_VOLUMES != 0,
        DLT_DIRECTORY | DLT_LATE | DLT_NONBINDING => flags & LDF_ASSIGNS != 0,
        _ => false,
    }
}

/// The next entry after the node at `at` (the head or an entry) whose type `flags` ask for, or 0.
fn next_matching<U: GuestUi>(a: &Amiga<U>, mut at: u32, flags: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("DosList");
    for _ in 0..MAX_WALK {
        let n = a.mem.read_u32(at).map_err(&m)?;
        if n == 0 { return Ok(0); }
        let e = n << 2;
        if type_wanted(a.mem.read_u32(e + DL_TYPE).map_err(&m)?, flags) { return Ok(e); }
        at = e;
    }
    Err(Refusal("DOS list is cyclic or longer than any real list".into()))
}

/// NextDosEntry(d1 = dlist, d2 = flags) -> the next matching entry or NULL (ERROR_NO_MORE_ENTRIES).
fn next_dos_entry<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (at, flags) = (cpu.dreg(1), cpu.dreg(2));
    let e = next_matching(a, at, flags)?;
    if e == 0 { return fail(a, cpu, ERROR_NO_MORE_ENTRIES, 0, "[dos] NextDosEntry -> end of list".into()); }
    ok(a, cpu, e, format!("[dos] NextDosEntry {at:#x} -> {e:#x} \"{}\"", entry_name(a, e)))
}

/// The dol_Name of the entry at `e`.
pub fn entry_name<U: GuestUi>(a: &Amiga<U>, e: u32) -> String {
    let b = a.mem.read_u32(e + DL_NAME).unwrap_or(0) << 2;
    let n = a.mem.read_u8(b).unwrap_or(0) as u32;
    (0..n).map(|i| a.mem.read_u8(b + 1 + i).unwrap_or(b'?') as char).collect()
}

/// FindDosEntry(d1 = dlist, d2 = name, d3 = flags) -> the first matching entry after `dlist` whose name is `name`.
fn find_dos_entry<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (mut at, name, flags) = (cpu.dreg(1), str_arg(a, cpu.dreg(2), "FindDosEntry.name")?, cpu.dreg(3));
    let what = format!("[dos] FindDosEntry \"{name}\" {flags:#x}");
    loop {
        let e = next_matching(a, at, flags)?;
        if e == 0 { return fail(a, cpu, ERROR_OBJECT_NOT_FOUND, 0, what); }
        if entry_name(a, e).eq_ignore_ascii_case(&name) { return ok(a, cpu, e, format!("{what} -> {e:#x}")); }
        at = e;
    }
}

/// The DLT type and node of entry `name` of the kinds `flags` ask for (host-side lookup for GetDeviceProc & co).
pub fn find<'a, U: GuestUi>(a: &'a Amiga<U>, name: &str, flags: u32) -> Option<&'a Entry> {
    a.fs.dos.entries.iter().find(|e| type_wanted(e.ty, flags) && e.name.eq_ignore_ascii_case(name))
}

impl Entry {
    /// The BPTRs of the FileLocks the entry holds, in directory order (an assign's dol_Lock first, then its AssignList).
    pub fn lock_bptrs(&self) -> Vec<u32> { self.locks.iter().map(|l| l >> 2).collect() }
}
