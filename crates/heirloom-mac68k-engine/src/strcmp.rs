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
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA03C, cmp_string::<U>)?;
    m.register_os(0xA050, rel_string::<U>)?;
    m.register_os(0xA054, upr_string::<U>)
}

/// MacRoman: (lower-case accented letter, its capital with the same mark). The other direction of the system script's case pairs.
const ACCENTED_CAPITAL: &[(u8, u8)] = &[
    (0x87, 0xE7), (0x88, 0xCB), (0x89, 0xE5), (0x8A, 0x80), (0x8B, 0xCC), (0x8C, 0x81), (0x8D, 0x82), (0x8E, 0x83), (0x8F, 0xE9), (0x90, 0xE6),
    (0x91, 0xE8), (0x92, 0xEA), (0x93, 0xED), (0x94, 0xEB), (0x95, 0xEC), (0x96, 0x84), (0x97, 0xEE), (0x98, 0xF1), (0x99, 0xEF), (0x9A, 0x85),
    (0x9B, 0xCD), (0x9C, 0xF2), (0x9D, 0xF4), (0x9E, 0xF3), (0x9F, 0x86), (0xBE, 0xAE), (0xBF, 0xAF), (0xCF, 0xCE), (0xD8, 0xD9),
];
/// MacRoman: (accented capital, the plain capital it is built on): what `UprString` without `,MARKS` leaves of it.
const CAPITAL_BASE: &[(u8, u8)] = &[
    (0x80, b'A'), (0x81, b'A'), (0x82, b'C'), (0x83, b'E'), (0x84, b'N'), (0x85, b'O'), (0x86, b'U'), (0xCB, b'A'), (0xCC, b'A'), (0xCD, b'O'),
    (0xD9, b'Y'), (0xE5, b'A'), (0xE6, b'E'), (0xE7, b'A'), (0xE8, b'E'), (0xE9, b'E'), (0xEA, b'I'), (0xEB, b'I'), (0xEC, b'I'), (0xED, b'I'),
    (0xEE, b'O'), (0xEF, b'O'), (0xF1, b'O'), (0xF2, b'U'), (0xF3, b'U'), (0xF4, b'U'),
];

/// `UprString`'s rule on MacRoman text (spec macos9-specs/Misc/UpperString.md): a-z become A-Z, an accented lower-case letter becomes its
/// accented capital; with `keep_marks` FALSE every accented capital then loses its mark (e-acute -> 'E'). Other bytes are unchanged.
pub fn upper_string(text: &mut [u8], keep_marks: bool) {
    for c in text.iter_mut() {
        let mut x = c.to_ascii_uppercase();
        if let Some(&(_, cap)) = ACCENTED_CAPITAL.iter().find(|(l, _)| *l == x) { x = cap; }
        if !keep_marks { if let Some(&(_, base)) = CAPITAL_BASE.iter().find(|(a, _)| *a == x) { x = base; } }
        *c = x;
    }
}

/// `_UprString` ($A054; bit 9 = `,MARKS` keeps the marks): A0 = the first character, D0.W = the length; uppercased in place, D0 = 0.
fn upr_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let n = r.d0 & 0xFFFF;
    let sp = m.mem.span(r.a0, n).map_err(Refusal::mem("UprString text"))?;
    let mut text = m.mem.bytes(sp).map(<[u8]>::to_vec).map_err(Refusal::mem("UprString text"))?;
    upper_string(&mut text, r.opcode & 0x0200 != 0);
    m.mem.write_bytes(r.a0, &text).map_err(Refusal::mem("UprString text"))?;
    r.d0 = 0;
    Ok(format!("[util] UprString {n} bytes (marks {})", if r.opcode & 0x0200 != 0 { "kept" } else { "stripped" }))
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
