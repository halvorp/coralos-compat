//! `_Pack7`, the Binary-Decimal Conversion Package (Inside Macintosh I, "Binary-Decimal Conversion Package"):
//! `NumToString` (selector 0) and `StringToNum` (selector 1). The floating-point decimal routines (`Str2Dec` ...) belong to
//! the Floating-Point/SANE packages and are a named refusal here.
//!
//! `NumToString` writes the decimal text of a LONGINT (a leading `-` for a negative, no padding). `StringToNum` reads an
//! optional sign and decimal digits; it stops at the first non-digit and does not validate (the book: "the result is
//! undefined for a string that is not a number"); an empty string gives 0.
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::{String, ToString};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA9EE, pack7::<U>) }

/// `_Pack7` is a REGISTER package, unlike the Pascal-stack packages ([`crate::package`]): the stack holds only the selector
/// word (the package pops it and leaves the caller's return address on top), `NumToString` takes the number in D0 and the
/// Str255 in A0, `StringToNum` takes the Str255 in A0 and answers the number in D0. Facts: the Advanced Mac Substitute Pack7
/// (`ams-pack/BinaryDecimal.cc`: `NumToString_call(long x : __D0, StringPtr p : __A0)`, `StringToNum_call(... : __A0)`, the
/// selector popped by `Pack7_patch`) and the glue of the corpus applications themselves (`move.w #sel,-(sp); _Pack7;
/// movea.l (sp)+,a0; addq #8,sp; jmp (a0)` with D0 / A0 loaded before the trap). A selector this package lacks halts the
/// guest by name (the frame cannot be told, as in [`crate::package::call`]).
fn pack7<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let sp = cpu.areg(7);
    let sel = m.mem.read_u16(sp).map_err(Refusal::mem("package selector"))?;
    let log = match sel {
        0 => {
            let (n, at) = (r.d0 as i32, r.a0);
            let text = num_text(n);
            crate::pascal::write_pstring(&mut m.mem, at, text.as_bytes())?;
            format!("[pack7] NumToString {n} -> \"{text}\"")
        }
        1 => {
            let s = crate::pascal::read_pstring_bytes(&m.mem, r.a0)?;
            let n = parse_num(&s);
            r.d0 = n as u32;
            format!("[pack7] StringToNum \"{}\" -> {n}", s.iter().map(|&b| char::from(b)).collect::<String>())
        }
        _ => {
            cpu.halt();
            return Err(Refusal(format!("Pack7 selector {sel:#x} is not a routine this engine implements (the guest is stopped: its argument size is unknown)")));
        }
    };
    cpu.set_areg(7, sp.wrapping_add(2));
    Ok(log)
}

/// Text of a LONGINT as `NumToString` writes it.
pub fn num_text(n: i32) -> String { n.to_string() }

/// The number `StringToNum` reads from `s`: sign, then digits up to the first non-digit (wrapping at 32 bits, as the 68k loop does).
pub fn parse_num(s: &[u8]) -> i32 {
    let (neg, digits) = match s.first() { Some(b'-') => (true, &s[1..]), Some(b'+') => (false, &s[1..]), _ => (false, s) };
    let n = digits.iter().take_while(|b| b.is_ascii_digit()).fold(0i32, |a, &b| a.wrapping_mul(10).wrapping_add(i32::from(b - b'0')));
    if neg { n.wrapping_neg() } else { n }
}
