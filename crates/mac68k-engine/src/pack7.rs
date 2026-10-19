//! `_Pack7`, the Binary-Decimal Conversion Package (Inside Macintosh I, "Binary-Decimal Conversion Package"):
//! `NumToString` (selector 0) and `StringToNum` (selector 1). The floating-point decimal routines (`Str2Dec` ...) belong to
//! the Floating-Point/SANE packages and are a named refusal here.
//!
//! `NumToString` writes the decimal text of a LONGINT (a leading `-` for a negative, no padding). `StringToNum` reads an
//! optional sign and decimal digits; it stops at the first non-digit and does not validate (the book: "the result is
//! undefined for a string that is not a number"); an empty string gives 0.
use crate::machine::{Done, Mac};
use crate::osconv::Regs;
use crate::package::{call, Routine};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::string::{String, ToString};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA9EE, pack7::<U>) }

fn pack7<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> { call(m, c, r, "Pack7", lookup::<U>) }

fn lookup<U: GuestUi>(sel: u16) -> Option<Routine<U>> {
    match sel {
        0 => Some(Routine { name: "NumToString", args: &[Arg::L, Arg::P], result: None, run: num_to_string::<U> }),
        1 => Some(Routine { name: "StringToNum", args: &[Arg::P, Arg::P], result: None, run: string_to_num::<U> }),
        _ => None,
    }
}

/// Text of a LONGINT as `NumToString` writes it.
pub fn num_text(n: i32) -> String { n.to_string() }

/// The number `StringToNum` reads from `s`: sign, then digits up to the first non-digit (wrapping at 32 bits, as the 68k loop does).
pub fn parse_num(s: &[u8]) -> i32 {
    let (neg, digits) = match s.first() { Some(b'-') => (true, &s[1..]), Some(b'+') => (false, &s[1..]), _ => (false, s) };
    let n = digits.iter().take_while(|b| b.is_ascii_digit()).fold(0i32, |a, &b| a.wrapping_mul(10).wrapping_add(i32::from(b - b'0')));
    if neg { n.wrapping_neg() } else { n }
}

fn num_to_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &crate::machine::Call) -> Result<Done, Refusal> {
    let (n, at) = (c.args.get(0) as i32, c.args.get(1));
    let text = num_text(n);
    crate::pascal::write_pstring(&mut m.mem, at, text.as_bytes())?;
    Ok(Done { result: 0, log: format!("[pack7] NumToString {n} -> \"{text}\"") })
}

fn string_to_num<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &crate::machine::Call) -> Result<Done, Refusal> {
    let (s, out) = (crate::pascal::read_pstring_bytes(&m.mem, c.args.get(0))?, c.args.get(1));
    let n = parse_num(&s);
    m.mem.write_u32(out, n as u32).map_err(Refusal::mem("StringToNum result"))?;
    Ok(Done { result: 0, log: format!("[pack7] StringToNum \"{}\" -> {n}", s.iter().map(|&b| char::from(b)).collect::<String>()) })
}
