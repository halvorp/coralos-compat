//! exec.library SetFunction / SumLibrary: patching a library's vector table.
//!
//! SetFunction(a1 = library, a0 = funcOffset (negative, a multiple of 6), d0 = newFunction) writes a `JMP newFunction`
//! (6 bytes, $4EF9 + address) into the vector, sets LIBF_CHANGED, re-sums the library, and returns the OLD function's
//! address (AROS rom/exec/setfunction.c). The engine's own vectors are `ILLEGAL; RTS` slots that the host traps by address,
//! so a patch that chains to "the old function" must reach the host handler at a NEW address: the first time a trapped
//! slot is patched its 6 bytes are copied to a fresh trampoline slot in the engine's arena, registered with the same handler
//! and library/function name, and the trampoline's address is the old function. Calling it, or patching it back with
//! SetFunction(library, offset, old), behaves as the original vector. A vector that already holds a JMP (a guest library, or
//! one patched before) returns the JMP's target. Library bases and offsets that do not describe a vector are named refusals.
//!
//! DOpus 5 (Library/wb.c:2420-2720) patches ~30 functions of exec, dos, intuition, icon and workbench with SetFunction in
//! L_WB_Install_Patch, keeps each old pointer, restores them with SetFunction at removal and expects the value returned
//! then to be its own patch; both hold here.
use alloc::format;
use alloc::string::String;
use crate::libtable::{write_slot, ILLEGAL};
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const LVO_SET_FUNCTION: i32 = -420;
pub const LVO_SUM_LIBRARY: i32 = -426;
const JMP_ABS_L: u16 = 0x4EF9;
const VECTOR: u32 = 6;
const LIB_FLAGS: u32 = 14;
const LIB_NEG_SIZE: u32 = 16;
const LIB_SUM: u32 = 28;
const LIBF_SUMMING: u8 = 1 << 0;
const LIBF_CHANGED: u8 = 1 << 1;
const LIBF_SUMUSED: u8 = 1 << 2;

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.func(LVO_SET_FUNCTION, "SetFunction", set_function::<U>).func(LVO_SUM_LIBRARY, "SumLibrary", sum_library::<U>)
}

fn m(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem(what) }

fn set_function<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (lib, off, new) = (cpu.areg(1), cpu.areg(0) as i32, cpu.dreg(0));
    let neg = a.mem.read_u16(lib + LIB_NEG_SIZE).map_err(m("lib_NegSize"))? as i32;
    if off >= 0 || off % VECTOR as i32 != 0 || -off > neg {
        return Err(Refusal(format!("SetFunction: offset {off} is not a vector of library {lib:#x} (its table is {neg} bytes of 6-byte vectors)")));
    }
    let vec = lib.wrapping_add(off as u32);
    let w0 = a.mem.read_u16(vec).map_err(m("library vector"))?;
    let old = if w0 == JMP_ABS_L {
        a.mem.read_u32(vec + 2).map_err(m("library vector"))?
    } else if w0 == ILLEGAL {
        let Some((l, f, h)) = a.dispatch_entry(vec) else { return Err(Refusal(format!("SetFunction: the vector at {vec:#x} is an ILLEGAL no function owns"))) };
        let tramp = crate::fs::alloc_zeroed_sys(a, VECTOR)?.ok_or_else(|| Refusal("SetFunction: no room for the old-function trampoline in the engine arena (NOMEM)".into()))?;
        write_slot(&mut a.mem, tramp).map_err(m("trampoline"))?;
        a.install_slot(tramp, l, f, h)?;
        tramp
    } else {
        return Err(Refusal(format!("SetFunction: the vector at {vec:#x} holds {w0:#06x}, neither a JMP nor an engine slot")));
    };
    a.mem.write_u16(vec, JMP_ABS_L).map_err(m("library vector"))?;
    a.mem.write_u32(vec + 2, new).map_err(m("library vector"))?;
    let flags = a.mem.read_u8(lib + LIB_FLAGS).map_err(m("lib_Flags"))?;
    a.mem.write_u8(lib + LIB_FLAGS, flags | LIBF_CHANGED).map_err(m("lib_Flags"))?;
    sum(a, lib)?;
    cpu.set_d0(old);
    Ok(format!("[exec] SetFunction lib={lib:#x} offset {off} -> {new:#x}, old function {old:#x}"))
}

/// SumLibrary's body (exec: only when the library asks for it with LIBF_SUMUSED and is not being summed).
fn sum<U: GuestUi>(a: &mut Amiga<U>, lib: u32) -> Result<(), Refusal> {
    let flags = a.mem.read_u8(lib + LIB_FLAGS).map_err(m("lib_Flags"))?;
    if flags & LIBF_SUMUSED == 0 || flags & LIBF_SUMMING != 0 { return Ok(()); }
    let neg = a.mem.read_u16(lib + LIB_NEG_SIZE).map_err(m("lib_NegSize"))? as u32;
    let mut sum = 0u32;
    let mut p = lib - neg;
    while p < lib { sum = sum.wrapping_add(a.mem.read_u32(p).map_err(m("library vector"))?); p += 4; }
    let stored = a.mem.read_u32(lib + LIB_SUM).map_err(m("lib_Sum"))?;
    a.mem.write_u8(lib + LIB_FLAGS, flags & !LIBF_CHANGED).map_err(m("lib_Flags"))?;
    if flags & LIBF_CHANGED == 0 && stored != sum {
        return Err(Refusal(format!("SumLibrary {lib:#x}: checksum {stored:#x} != {sum:#x} and LIBF_CHANGED clear (exec would Alert AN_LibChkSum)")));
    }
    a.mem.write_u32(lib + LIB_SUM, sum).map_err(m("lib_Sum"))
}

/// SumLibrary(a1 = library).
fn sum_library<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let lib = cpu.areg(1);
    sum(a, lib)?;
    Ok(format!("[exec] SumLibrary {lib:#x}"))
}
