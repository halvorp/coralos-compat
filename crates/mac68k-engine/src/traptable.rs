//! `_GetTrapAddress` / `_NGetTrapAddress` (`_GetOSTrapAddress` $A346, `_GetToolTrapAddress` $A746): "where is trap N?".
//! Programs ask to learn whether a trap exists (they compare the answer with the address of `_Unimplemented`, $A89F): the
//! Retro68 runtime does it in its startup and Toolbox-era code does it before using any System 7 call, so 9 of the 18 corpus
//! applications stopped here (docs/CORPUS.md).
//!
//! This engine has no ROM routines to point at: it bridges every trap through the A-line hook. The answer is therefore the
//! address of a 4-byte STUB, `<the trap word>; RTS`, which runs the very same bridged handler when the program calls it
//! (`JSR` to the address). A trap nobody has registered answers with the `_Unimplemented` stub, exactly what a real Mac does
//! for a trap its ROM lacks, so a program that probes sees "absent" and takes its fallback instead of crashing into a refusal.
//!
//! Inputs (Inside Macintosh VI): D0.W = the trap number; bit 10 of the CALLING trap word says Toolbox ($A746, number in 10
//! bits) or OS ($A346, 8 bits); a whole trap word is accepted and masked, as the ROM does. The old `_GetTrapAddress` ($A146)
//! has no type bit: bit 11 of the word or a number of $100 or more is a Toolbox trap. Output: A0 = the address. The stubs live in a reserved page range of low memory, written when first asked for.
use alloc::format;
use alloc::string::String;
use crate::machine::{Mac, OsHandler};
use crate::osconv::Regs;
use crate::traps::trap_key;
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// Toolbox traps' stubs: TOOL_STUBS + 4 * number (10-bit); OS traps': OS_STUBS + 4 * number (8-bit). Free low memory.
pub const TOOL_STUBS: u32 = 0x4000;
pub const OS_STUBS: u32 = 0x5000;
const UNIMPLEMENTED_TRAP: u16 = 0xA89F;
const RTS: u16 = 0x4E75;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    let h: OsHandler<U> = get_trap_address::<U>;
    m.register_os(0xA146, h) // the key drops the flag bits, so $A346 and $A746 reach the same handler
}

fn get_trap_address<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    // Programs pass either the bare number or the whole trap word (`GetOSTrapAddress(_Gestalt)` is $A1AD): the ROM masks the
    // number to 8 bits (OS) or 10 bits (Toolbox), so this does too.
    let n = (r.d0 & 0xFFFF) as u16;
    let tool = if r.opcode & 0x0400 != 0 { true } else if r.opcode & 0x0200 != 0 { false } else { n & 0x0800 != 0 || n & 0x03FF >= 0x100 };
    let word = if tool { 0xA800 | (n & 0x03FF) } else { 0xA000 | (n & 0x00FF) };
    let implemented = m.table.contains_key(&trap_key(word));
    let served = if implemented { word } else { UNIMPLEMENTED_TRAP };
    let at = if served & 0x0800 != 0 { TOOL_STUBS + 4 * (served & 0x03FF) as u32 } else { OS_STUBS + 4 * (served & 0x00FF) as u32 };
    m.mem.write_u16(at, served).map_err(Refusal::mem("trap stub"))?;
    m.mem.write_u16(at + 2, RTS).map_err(Refusal::mem("trap stub"))?;
    r.a0 = at;
    r.d0 = 0;
    Ok(format!("[toolbox] GetTrapAddress {} {n:#x} -> {at:#x} ({})", if tool { "Toolbox" } else { "OS" }, if implemented { "the trap's stub" } else { "absent: the _Unimplemented stub" }))
}
