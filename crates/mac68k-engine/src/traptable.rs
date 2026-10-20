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

/// Is `pc` inside one of the stubs [`get_trap_address`] hands out (so a trap word executing there is a CALL to the
/// original routine, with the caller's return address on the stack)?
pub fn in_stub(pc: u32) -> bool { (TOOL_STUBS..TOOL_STUBS + 0x1000).contains(&pc) || (OS_STUBS..OS_STUBS + 0x400).contains(&pc) }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    let h: OsHandler<U> = get_trap_address::<U>;
    m.register_os(0xA146, h)?; // the key drops the flag bits, so $A346 and $A746 reach the same handler
    m.register_os(0xA047, set_trap_address::<U>) // ... and $A247 / $A647 (SetOSTrapAddress / SetToolTrapAddress)
}

/// The trap word a trap NUMBER names, from the calling opcode's type bits and the number in D0 (see the module comment).
fn trap_word(opcode: u16, d0: u32) -> (u16, bool) {
    // Programs pass either the bare number or the whole trap word (`GetOSTrapAddress(_Gestalt)` is $A1AD): the ROM masks the
    // number to 8 bits (OS) or 10 bits (Toolbox), so this does too.
    let n = (d0 & 0xFFFF) as u16;
    let tool = if opcode & 0x0400 != 0 { true } else if opcode & 0x0200 != 0 { false } else { n & 0x0800 != 0 || n & 0x03FF >= 0x100 };
    (if tool { 0xA800 | (n & 0x03FF) } else { 0xA000 | (n & 0x00FF) }, tool)
}

/// The stub (`<trap word>; RTS`) that runs trap `word`'s bridged handler when called.
fn stub_address(word: u16) -> u32 { if word & 0x0800 != 0 { TOOL_STUBS + 4 * (word & 0x03FF) as u32 } else { OS_STUBS + 4 * (word & 0x00FF) as u32 } }

fn kind(tool: bool) -> &'static str { if tool { "Toolbox" } else { "OS" } }

fn get_trap_address<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let (at, line) = trap_address(m, r.opcode, r.d0)?;
    r.a0 = at;
    r.d0 = 0;
    Ok(line)
}

/// THE answer to "where is trap N?", for every caller: the 68k `_GetTrapAddress` family above and the PowerPC imports of
/// `NGetTrapAddress` / `GetOSTrapAddress` / `GetToolTrapAddress` / `GetTrapAddress` (macppc-engine natives). `opcode` is the
/// calling trap word whose type bits pick the table ($A146 old, $A346 OS, $A746 Toolbox) and `number` the trap number or whole
/// trap word. The address is the guest 68k stub (or the patch routine of a patched trap); a trap with no handler answers
/// with the `_Unimplemented` stub, so `TrapAvailable`-style comparisons are truthful. Returns the address and the log line.
pub fn trap_address<U: GuestUi>(m: &mut Mac<U>, opcode: u16, number: u32) -> Result<(u32, String), Refusal> {
    let (word, tool) = trap_word(opcode, number);
    let n = (number & 0xFFFF) as u16;
    let implemented = m.table.contains_key(&trap_key(word));
    let served = if implemented { word } else { UNIMPLEMENTED_TRAP };
    let at = stub_address(served);
    m.mem.write_u16(at, served).map_err(Refusal::mem("trap stub"))?;
    m.mem.write_u16(at + 2, RTS).map_err(Refusal::mem("trap stub"))?;
    if let Some(&patch) = m.patches.get(&trap_key(word)) { // a patched trap's address is the patch routine itself
        return Ok((patch, format!("[toolbox] GetTrapAddress {} {n:#x} -> {patch:#x} (patched)", kind(tool))));
    }
    Ok((at, format!("[toolbox] GetTrapAddress {} {n:#x} -> {at:#x} ({})", kind(tool), if implemented { "the trap's stub" } else { "absent: the _Unimplemented stub" })))
}

/// `_SetTrapAddress` / `_SetOSTrapAddress` / `_SetToolTrapAddress`: D0 = the trap number, A0 = the new routine. From now on
/// the trap runs THAT guest routine first (the bridge enters it as a `JSR` from the trapping instruction); the routine chains
/// to the original with a `JMP` to the address `GetTrapAddress` returned before the patch, and returns with `RTS`. Setting a
/// trap back to its own stub removes the patch (the `SetFunction` pattern of the Amiga engine, `exec_patch.rs`).
fn set_trap_address<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let (word, tool) = trap_word(r.opcode, r.d0);
    let (key, to) = (trap_key(word), r.a0);
    if to == stub_address(word) || (to == stub_address(UNIMPLEMENTED_TRAP) && !m.table.contains_key(&key)) {
        m.patches.remove(&key);
        return Ok(format!("[toolbox] SetTrapAddress {} {word:#06x}: back to the original", kind(tool)));
    }
    m.mem.read_u16(to).map_err(Refusal::mem("SetTrapAddress routine"))?; // the routine must be in guest RAM
    m.patches.insert(key, to);
    r.d0 = 0;
    Ok(format!("[toolbox] SetTrapAddress {} {word:#06x} -> {to:#x} (a guest routine; it chains to the original through the stub GetTrapAddress gave)", kind(tool)))
}
