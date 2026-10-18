//! Filesystem state of one Amiga machine and the helpers every dos handler shares: the [`Volume`], the host-side
//! tables behind guest `FileLock`/`FileHandle` structs, and the IoErr plumbing.
//!
//! Locks and file handles are REAL structs in guest RAM (so a program that inspects them sees plausible fields),
//! but everything that matters (which node, the position, the access mode) is host-side bookkeeping keyed by the
//! struct's address: a guest cannot corrupt it, and a BPTR the engine never issued is a clean ERROR_INVALID_LOCK.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::dos_err::{set_ioerr, DosErr, ERROR_INVALID_LOCK};
use crate::volume::{NodeId, Volume};
use crate::{machine::Amiga, process, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// dos/dos.h DOSTRUE.
pub const DOSTRUE: u32 = 0xFFFF_FFFF;
/// `struct FileLock` is 20 bytes (fl_Link, fl_Key, fl_Access, fl_Task, fl_Volume), computed with m68k-linux-gnu-gcc
/// -mno-align-int over the AROS dos/dosextens.h field list; `struct FileHandle` is 44 (dos.rs).
pub const LOCK_SIZE: u32 = 20;
pub const FH_SIZE: u32 = 44;
pub const FIB_SIZE: u32 = 260;
const FL_KEY: u32 = 4;
const FL_ACCESS: u32 = 8;
/// `Process.pr_CurrentDir`.
const PR_CURRENT_DIR: u32 = 152;

pub struct LockRec { pub node: NodeId, pub exclusive: bool }
pub struct OpenFile { pub node: NodeId, pub pos: u32, pub read: bool, pub write: bool, pub exclusive: bool }

#[derive(Default)]
pub struct FsState {
    pub vol: Volume,
    /// Live guest locks: struct address -> what they lock.
    pub locks: BTreeMap<u32, LockRec>,
    /// Live guest file handles (not the console's): struct address -> state.
    pub files: BTreeMap<u32, OpenFile>,
    /// Directory scans begun by Examine: FileInfoBlock address -> (directory, next child index).
    pub scans: BTreeMap<u32, (NodeId, usize)>,
    /// Segment lists LoadSeg made: first segment's BPTR -> (heap region, region size) (loadseg.rs).
    pub segs: BTreeMap<u32, (u32, u32)>,
    /// Libraries loaded from disk (disklib.rs).
    pub disk_libs: Vec<crate::disklib::DiskLib>,
}

impl FsState {
    pub fn node_in_use(&self, n: NodeId) -> bool { self.locks.values().any(|l| l.node == n) || self.files.values().any(|f| f.node == n) }
}

/// Set the secondary error and return `ret` in D0 with a log line: the shape of every failing dos call.
pub fn fail<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, e: DosErr, ret: u32, what: String) -> Result<String, Refusal> {
    set_ioerr(&mut a.mem, e);
    cpu.set_d0(ret);
    Ok(format!("{what} -> failed, IoErr {}", e.0))
}

/// A successful call clears the secondary error as AmigaDOS routines that return it do.
pub fn ok<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, ret: u32, what: String) -> Result<String, Refusal> {
    set_ioerr(&mut a.mem, DosErr(0));
    cpu.set_d0(ret);
    Ok(what)
}

pub fn str_arg<U: GuestUi>(a: &Amiga<U>, addr: u32, what: &str) -> Result<String, Refusal> { a.mem.read_cstr(addr).map_err(Refusal::mem(what)) }

/// The node a lock BPTR names. 0 is the boot volume's root (AmigaDOS: "the root of the boot device").
pub fn lock_node<U: GuestUi>(a: &Amiga<U>, bptr: u32) -> Result<NodeId, DosErr> {
    if bptr == 0 { return Ok(a.fs.vol.boot_device()); }
    a.fs.locks.get(&(bptr << 2)).map(|l| l.node).ok_or(ERROR_INVALID_LOCK)
}

/// The directory relative paths resolve against: the Process's current directory.
pub fn cur_dir<U: GuestUi>(a: &Amiga<U>) -> NodeId {
    let b = a.mem.read_u32(process::PROCESS + PR_CURRENT_DIR).unwrap_or(0);
    lock_node(a, b).unwrap_or_else(|_| a.fs.vol.boot_device())
}

pub fn set_cur_dir<U: GuestUi>(a: &mut Amiga<U>, bptr: u32) -> Result<u32, Refusal> {
    let old = a.mem.read_u32(process::PROCESS + PR_CURRENT_DIR).map_err(Refusal::mem("pr_CurrentDir"))?;
    a.mem.write_u32(process::PROCESS + PR_CURRENT_DIR, bptr).map_err(Refusal::mem("pr_CurrentDir"))?;
    Ok(old)
}

/// Allocate a zeroed struct from the guest heap (NOMEM is a legit ERROR_NO_FREE_STORE for the caller).
pub fn alloc_zeroed<U: GuestUi>(a: &mut Amiga<U>, size: u32) -> Result<Option<u32>, Refusal> {
    let Some(p) = a.heap.alloc(size) else { return Ok(None) };
    a.mem.zero(p, size).map_err(Refusal::mem("alloc_zeroed"))?;
    Ok(Some(p))
}

/// Make a guest FileLock on `node`; None = out of guest memory.
pub fn new_lock<U: GuestUi>(a: &mut Amiga<U>, node: NodeId, exclusive: bool) -> Result<Option<u32>, Refusal> {
    let Some(p) = alloc_zeroed(a, LOCK_SIZE)? else { return Ok(None) };
    a.mem.write_u32(p + FL_KEY, node as u32).map_err(Refusal::mem("fl_Key"))?;
    a.mem.write_u32(p + FL_ACCESS, if exclusive { -1i32 as u32 } else { -2i32 as u32 }).map_err(Refusal::mem("fl_Access"))?;
    a.fs.locks.insert(p, LockRec { node, exclusive });
    Ok(Some(p))
}
