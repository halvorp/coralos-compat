//! `_CmpString` ($A03C) and `_RelString` ($A050): the Operating System's string comparison (Inside Macintosh I, "Utilities").
//! Registers: A0 = the first string's characters, A1 = the second's, D0 high word = the first length, D0 low word = the second
//! length (these are NOT Pascal strings: the lengths are explicit). The trap word's flag bits choose the rule: bit 9
//! (`,MARKS`) makes diacritical marks significant, bit 10 (`,CASE`) makes case significant; with neither, "ae" equals "AE" and
//! "e" equals the accented "e". `CmpString` answers D0 = 0 for equal and 1 otherwise; `RelString` answers -1, 0 or 1 for
//! the first string preceding, equal to, or following the second. Both set the Z flag from D0 like every OS trap.
use crate::intl::compare_with;
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA03C, cmp_string::<U>)?;
    m.register_os(0xA050, rel_string::<U>)
}

fn order<U: GuestUi>(m: &Mac<U>, r: &Regs) -> Result<i32, Refusal> {
    let (la, lb) = (r.d0 >> 16, r.d0 & 0xFFFF);
    let read = |p: u32, n: u32| m.mem.span(p, n).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(Refusal::mem("string"));
    let (a, b) = (read(r.a0, la)?, read(r.a1, lb)?);
    Ok(compare_with(&a, &b, r.opcode & 0x0400 != 0, r.opcode & 0x0200 != 0))
}

fn cmp_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let o = order(m, r)?;
    r.d0 = u32::from(o != 0);
    Ok(format!("[util] CmpString (flags {:#05x}) -> {}", r.opcode & 0x0600, r.d0))
}

fn rel_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let o = order(m, r)?;
    r.d0 = o as u32;
    Ok(format!("[util] RelString (flags {:#05x}) -> {o}", r.opcode & 0x0600))
}
