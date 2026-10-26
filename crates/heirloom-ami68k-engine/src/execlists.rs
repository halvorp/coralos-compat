//! ExecBase's system lists (LibList, DeviceList, ResourceList, PortList, IntrList, MemList) as real empty
//! `struct List`s, and AddLibrary / RemLibrary over LibList, so a program that walks them (FindName on
//! `&SysBase->LibList`, as dopus5.library does for cybergraphics.library) finds a consistent list, not zeros.
//!
//! Offsets in `struct ExecBase` (exec/execbase.h, the NDK 3.x layout the fake ExecBase follows): MemList 322,
//! ResourceList 336, DeviceList 350, IntrList 364, LibList 378, PortList 392. `struct List` is 14 bytes: lh_Head 0,
//! lh_Tail 4, lh_TailPred 8, lh_Type 12.
use alloc::string::String;
use alloc::format;
use crate::machine::{Amiga, EXEC_BASE};
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_68k::Cpu;

pub const LVO_ADD_LIBRARY: i32 = -396;
pub const LVO_REM_LIBRARY: i32 = -402;
pub const LIB_LIST: u32 = EXEC_BASE + 378;
const LISTS: [(u32, u8); 6] = [(322, 10), (336, 8), (350, 3), (364, 2), (378, 9), (392, 4)];

/// Make the empty list at `list` (type `ty` = NT_*).
pub fn new_list(mem: &mut GuestMem, list: u32, ty: u8) -> Result<(), Refusal> {
    let m = Refusal::mem("List");
    mem.write_u32(list, list + 4).map_err(&m)?;
    mem.write_u32(list + 4, 0).map_err(&m)?;
    mem.write_u32(list + 8, list).map_err(&m)?;
    mem.write_u8(list + 12, ty).map_err(&m)
}

pub fn init(mem: &mut GuestMem) -> Result<(), Refusal> {
    for (off, ty) in LISTS { new_list(mem, EXEC_BASE + off, ty)?; }
    Ok(())
}

/// AddTail(list, node).
pub fn add_tail(mem: &mut GuestMem, list: u32, node: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("AddTail");
    let pred = mem.read_u32(list + 8).map_err(&m)?;
    mem.write_u32(node, list + 4).map_err(&m)?;
    mem.write_u32(node + 4, pred).map_err(&m)?;
    mem.write_u32(pred, node).map_err(&m)?;
    mem.write_u32(list + 8, node).map_err(&m)
}

/// Remove(node).
pub fn remove(mem: &mut GuestMem, node: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("Remove");
    let (succ, pred) = (mem.read_u32(node).map_err(&m)?, mem.read_u32(node + 4).map_err(&m)?);
    mem.write_u32(pred, succ).map_err(&m)?;
    mem.write_u32(succ + 4, pred).map_err(&m)
}

/// AddLibrary(a1 = library): link it into LibList.
pub fn add_library<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let lib = cpu.areg(1);
    add_tail(&mut a.mem, LIB_LIST, lib)?;
    Ok(format!("[exec] AddLibrary {lib:#x}"))
}

/// RemLibrary(a1 = library) -> 0 (the segment list is only returned by the library's own Expunge): unlink it.
pub fn rem_library<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let lib = cpu.areg(1);
    remove(&mut a.mem, lib)?;
    cpu.set_d0(0);
    Ok(format!("[exec] RemLibrary {lib:#x}"))
}

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.func(LVO_ADD_LIBRARY, "AddLibrary", add_library::<U>).func(LVO_REM_LIBRARY, "RemLibrary", rem_library::<U>)
}
