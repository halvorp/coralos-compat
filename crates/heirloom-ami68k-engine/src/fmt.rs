//! The one printf-style formatter of the engine: exec's RawDoFmt, locale.library's FormatString (and dos's VFPrintf /
//! VPrintf, which are RawDoFmt into a file) all run [`format`], and the callers only decide where the bytes go.
//!
//! Template: `%[n$][-][0][width][.limit][l|ll|L|i]type`. Data are 16-bit unless `l`; strings (`s`, `b`), pointers (`p`, `P`)
//! and `l` arguments are 32-bit, packed back to back in the data stream. Types: `b` BSTR, `d` signed, `u` unsigned, `x` upper
//! case hex (RawDoFmt's `%lx` is UPPER case), `X` lower case hex, `s` C string (NULL prints as empty), `c` character; with
//! `ext` (FormatString, AROS formatstring.c) also `D`/`U` (grouped by the default locale: groups of 3, ","), `p`/`P`,
//! positional `n$` arguments and `i`; without it those are the faulty-`%` case, printed as the character itself.
//! 64-bit arguments are a named refusal.
use alloc::format;
use alloc::vec::Vec;
use crate::Refusal;
use heirloom_mem_flat32::GuestMem;

pub struct Formatted {
    /// The text, WITHOUT the terminating NUL.
    pub bytes: Vec<u8>,
    /// How many bytes of the data stream the template used (RawDoFmt returns data + used).
    pub used: u32,
}

struct Arg { pos: usize, size: u32 }

/// Decimal digits of `n` grouped by 3 with the default locale's separator (loc_Grouping "\x03\x00", loc_GroupSeparator ",").
fn group(n: u32) -> Vec<u8> {
    let s = format!("{n}");
    let mut out = Vec::new();
    for (i, c) in s.bytes().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(b','); } out.push(c); }
    out
}

pub fn format(mem: &GuestMem, t: &[u8], data: u32, ext: bool) -> Result<Formatted, Refusal> {
    // Pass 1: the directives, to lay out the argument stream by position.
    struct Dir { arg: usize, left: bool, zero: bool, width: usize, limit: usize, size: u32, ty: u8 }
    enum Item { Lit(u8), Dir(Dir) }
    let mut items = Vec::new();
    let (mut i, mut counter) = (0, 0usize);
    let num = |i: &mut usize| { let mut n = 0usize; while *i < t.len() && t[*i].is_ascii_digit() { n = n * 10 + (t[*i] - b'0') as usize; *i += 1; } n };
    while i < t.len() {
        if t[i] != b'%' { items.push(Item::Lit(t[i])); i += 1; continue; }
        i += 1;
        if t.get(i) == Some(&b'%') { items.push(Item::Lit(b'%')); i += 1; continue; }
        counter += 1;
        let save = i;
        let mut arg = counter;
        if ext && t.get(i).map_or(false, u8::is_ascii_digit) {
            let n = num(&mut i);
            if t.get(i) == Some(&b'$') { i += 1; arg = n; } else { i = save; }
        }
        let left = t.get(i) == Some(&b'-'); if left { i += 1; }
        let zero = t.get(i) == Some(&b'0'); if zero { i += 1; }
        let width = num(&mut i);
        let mut limit = usize::MAX;
        if t.get(i) == Some(&b'.') { i += 1; if t.get(i).map_or(false, u8::is_ascii_digit) { limit = num(&mut i); } }
        let size = match t.get(i) {
            Some(b'i') if ext => { i += 1; 4 }
            Some(b'L') => return Err(Refusal("format: 64-bit arguments (%L, %ll) are not modelled".into())),
            Some(b'l') => { i += 1; if t.get(i) == Some(&b'l') { return Err(Refusal("format: 64-bit arguments (%L, %ll) are not modelled".into())); } 4 }
            _ => 2,
        };
        let Some(&ty) = t.get(i) else { break };
        i += 1;
        let size = if matches!(ty, b'b' | b's') || (ext && matches!(ty, b'p' | b'P')) { 4 } else { size };
        items.push(Item::Dir(Dir { arg, left, zero, width, limit, size, ty }));
    }
    let mut args: Vec<Arg> = items.iter().filter_map(|it| if let Item::Dir(d) = it { Some(Arg { pos: d.arg, size: d.size }) } else { None }).collect();
    args.sort_by_key(|x| x.pos);
    let off_of = |pos: usize| -> u32 { args.iter().filter(|x| x.pos < pos).map(|x| x.size).sum() };
    let total: u32 = args.iter().map(|x| x.size).sum();
    // Pass 2: format.
    let rd = |o: u32, size: u32| -> Result<u32, Refusal> {
        if size == 4 { mem.read_u32(data + o) } else { mem.read_u16(data + o).map(u32::from) }.map_err(Refusal::mem("data stream"))
    };
    let mut out: Vec<u8> = Vec::new();
    for it in &items {
        let d = match it { Item::Lit(c) => { out.push(*c); continue } Item::Dir(d) => d };
        let o = off_of(d.arg);
        let v = rd(o, d.size)?;
        let mut body: Vec<u8> = match d.ty {
            b'd' | b'D' if d.ty == b'd' || ext => {
                let neg = if d.size == 4 { (v as i32) < 0 } else { (v as u16 as i16) < 0 };
                let mag = if d.size == 4 { (v as i32).unsigned_abs() } else { (v as u16 as i16).unsigned_abs() as u32 };
                let mut s = if d.ty == b'D' { group(mag) } else { format!("{mag}").into_bytes() };
                if neg { s.insert(0, b'-'); }
                s
            }
            b'u' | b'U' if d.ty == b'u' || ext => { let mag = if d.size == 4 { v } else { v & 0xFFFF }; if d.ty == b'U' { group(mag) } else { format!("{mag}").into_bytes() } }
            b'x' => format!("{:X}", if d.size == 4 { v } else { v & 0xFFFF }).into_bytes(),   // RawDoFmt's %lx is UPPER case
            b'X' => format!("{:x}", if d.size == 4 { v } else { v & 0xFFFF }).into_bytes(),
            b'p' | b'P' if ext => { let s = format!("{v:08x}"); if d.ty == b'p' { s.to_uppercase() } else { s }.into_bytes() }
            b'c' => alloc::vec![v as u8],
            b's' | b'b' => {
                let mut p = v;
                let mut s = Vec::new();
                if d.ty == b'b' { p <<= 2; let n = if p == 0 { 0 } else { mem.read_u8(p).map_err(Refusal::mem("format %b"))? as u32 }; if p != 0 { p += 1; } for k in 0..n.min(d.limit as u32) { s.push(mem.read_u8(p + k).map_err(Refusal::mem("format %b"))?); } }
                else if p != 0 { while s.len() < d.limit { let c = mem.read_u8(p + s.len() as u32).map_err(Refusal::mem("format %s"))?; if c == 0 { break; } s.push(c); } }
                s
            }
            other => alloc::vec![other],   // an unknown type: the faulty '%' is ignored, the character printed
        };
        if ext && matches!(d.ty, b'p' | b'P') && body.len() < 8 { body.splice(0..0, core::iter::repeat(b'0').take(8 - body.len())); }
        let pad = d.width.saturating_sub(body.len());
        if d.left { out.extend(body); out.extend(core::iter::repeat(b' ').take(pad)); }
        else { out.extend(core::iter::repeat(if d.zero { b'0' } else { b' ' }).take(pad)); out.extend(body); }
    }
    Ok(Formatted { bytes: out, used: total })
}
