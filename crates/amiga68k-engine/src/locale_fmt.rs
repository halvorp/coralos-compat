//! locale.library's formatting calls, all through guest hooks: FormatDate and FormatString send the formatted bytes (and a
//! final NUL) to the caller's putChar hook one call each, via guest_call.rs (A0 = hook, A1 = the character, A2 = locale,
//! entry = `h_Entry` at hook+8); ParseDate reads its input from the caller's getChar hook (D0 = the character, 0 at the
//! end). The text is produced host-side first, so a hook that misbehaves cannot change what was formatted.
//!
//! Behaviour from AROS workbench/libs/locale (formatdate.c, formatstring.c, parsedate.c) with these decisions:
//! - FormatDate: %I and %Q show 12 for hour 0 and 12 (AROS prints 0 for midnight; AmigaOS shows 12); %j is the real day
//!   of the year; %Z prints nothing (unimplemented in 3.1); a lone trailing '%' prints nothing.
//! - FormatString: `%[n$][-][0][width][.limit][l|ll|L|i]type` with type b d D u U x X p P s c; data are 16-bit unless `l`,
//!   pointers and strings 32-bit; the result is the address after the arguments the template used. %D/%U group by the
//!   default locale's loc_Grouping/loc_GroupSeparator.
//! - ParseDate reads the WHOLE input first (up to 1024 characters, to the first NUL) and parses that, instead of pulling
//!   characters on demand: a getChar hook with side effects sees more calls than on AROS. A month missing from the
//!   template counts as January (AROS indexes before its table).
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::dos_date::{civil, days_of};
use crate::guest_call::{self, Next};
use crate::libtable::LibraryBuilder;
use crate::locale::{locale_arg, ABDAY_1, ABMON_1, AM_STR, DAY_1, MON_1, PM_STR, STRINGS};
use crate::locale_tables::{CTYPE, TO_UPPER};
use crate::machine::Amiga;
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const H_ENTRY: u32 = 8;
const MAX_INPUT: usize = 1024;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("FormatDate", format_date::<U>).named("FormatString", format_string::<U>).named("ParseDate", parse_date::<U>)
}

/// Call the hook once per byte, in order, then answer `ret`.
fn emit<U: GuestUi>(bytes: Vec<u8>, i: usize, hook: u32, entry: u32, locale: u32, ret: u32) -> Next<U> {
    let Some(&ch) = bytes.get(i) else { return Next::Return(ret) };
    Next::Call {
        target: entry, d0: 0, aregs: alloc::vec![(0, hook), (1, ch as u32), (2, locale)],
        then: Box::new(move |_, _, _| Ok(emit(bytes, i + 1, hook, entry, locale, ret))),
    }
}

fn hook_entry<U: GuestUi>(a: &Amiga<U>, hook: u32, call: &str) -> Result<u32, Refusal> {
    if hook == 0 { return Err(Refusal(format!("{call}: no hook"))); }
    let e = a.mem.read_u32(hook + H_ENTRY).map_err(Refusal::mem("h_Entry"))?;
    if e == 0 { return Err(Refusal(format!("{call}: the hook {hook:#x} has no h_Entry"))); }
    Ok(e)
}

fn digits(n: u32, fill: Option<u8>, len: usize, out: &mut Vec<u8>) {
    let s = format!("{n}");
    let s = &s.as_bytes()[s.len().saturating_sub(len)..]; // PrintDigits keeps the low `len` digits
    if let Some(f) = fill { for _ in s.len()..len { out.push(f); } }
    out.extend_from_slice(s);
}

fn text(i: usize, out: &mut Vec<u8>) { out.extend(STRINGS[i].chars().map(|c| c as u32 as u8)); }

/// The expansion of a FormatDate template for the day count / minute / tick of a DateStamp.
pub fn format_text(fmt: &[u8], days: i32, minute: u32, tick: u32) -> Vec<u8> {
    let (y, m, d) = civil(days);
    let wday = days.rem_euclid(7) as usize;
    let (hour, min, sec) = (minute / 60, minute % 60, tick / 50);
    let yday = days_of(y, m, d) - days_of(y, 0, 1) + 1;
    let h12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' { out.push(c); continue; }
        let Some(&k) = fmt.get(i) else { break };
        i += 1;
        match k {
            b'a' => text(ABDAY_1 + wday, &mut out),
            b'A' => text(DAY_1 + wday, &mut out),
            b'b' | b'h' => text(ABMON_1 + m, &mut out),
            b'B' => text(MON_1 + m, &mut out),
            b'c' => out.extend(format_text(b"%a %b %d %H:%M:%S %Y", days, minute, tick)),
            b'C' => out.extend(format_text(b"%a %b %e %T %Z %Y", days, minute, tick)),
            b'x' | b'D' => out.extend(format_text(b"%m/%d/%y", days, minute, tick)),
            b'X' | b'T' => out.extend(format_text(b"%H:%M:%S", days, minute, tick)),
            b'r' => out.extend(format_text(b"%I:%M:%S %p", days, minute, tick)),
            b'R' => out.extend(format_text(b"%H:%M", days, minute, tick)),
            b'd' => digits(d as u32, Some(b'0'), 2, &mut out),
            b'e' => digits(d as u32, Some(b' '), 2, &mut out),
            b'H' => digits(hour, Some(b'0'), 2, &mut out),
            b'I' => digits(h12, Some(b'0'), 2, &mut out),
            b'j' => digits(yday as u32, Some(b'0'), 3, &mut out),
            b'm' => digits(m as u32 + 1, Some(b'0'), 2, &mut out),
            b'M' => digits(min, Some(b'0'), 2, &mut out),
            b'n' => out.push(b'\n'),
            b'p' => text(if hour < 12 { AM_STR } else { PM_STR }, &mut out),
            b'q' => digits(hour, None, 2, &mut out),
            b'Q' => digits(h12, None, 2, &mut out),
            b'S' => digits(sec, Some(b'0'), 2, &mut out),
            b't' => out.push(b'\t'),
            b'U' | b'W' => {
                let tmp = if k == b'U' { yday + (6 - wday as i32) } else if wday != 0 { yday + (7 - wday as i32) } else { yday };
                digits(if tmp < 7 { 0 } else { ((tmp - tmp % 7) / 7) as u32 }, Some(b'0'), 2, &mut out);
            }
            b'w' => digits(wday as u32, None, 1, &mut out),
            b'y' => digits(y.rem_euclid(100) as u32, Some(b'0'), 2, &mut out),
            b'Y' => digits(y as u32, Some(b'0'), 4, &mut out),
            b'Z' => {}
            other => out.push(other),
        }
    }
    out
}

/// FormatDate(a0 = locale, a1 = template, a2 = DateStamp, a3 = putChar hook) -> TRUE. A NULL template sends one NUL.
fn format_date<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let loc = locale_arg(a, cpu.areg(0), "FormatDate")?;
    let (fmt, ds, hook) = (cpu.areg(1), cpu.areg(2), cpu.areg(3));
    let entry = hook_entry(a, hook, "FormatDate")?;
    let mut bytes = if fmt == 0 { Vec::new() } else {
        if ds == 0 { return Err(Refusal("FormatDate: no DateStamp".into())); }
        let f = crate::guest_str::read_bytes_z(&a.mem, fmt, 4096).map_err(Refusal::mem("FormatDate.template"))?;
        let r = |o| a.mem.read_u32(ds + o).map_err(Refusal::mem("DateStamp"));
        format_text(&f, r(0)? as i32, r(4)?, r(8)?)
    };
    bytes.push(0);
    let line = format!("[locale] FormatDate -> {} characters to hook {hook:#x}", bytes.len() - 1);
    guest_call::start(a, cpu, emit(bytes, 0, hook, entry, loc, 1))?;
    Ok(line)
}

struct Arg { pos: usize, size: u32 }

/// FormatString(a0 = locale, a1 = template, a2 = dataStream, a3 = putChar hook) -> the address after the used arguments.
fn format_string<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let loc = locale_arg(a, cpu.areg(0), "FormatString")?;
    let (tpl, data, hook) = (cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if tpl == 0 { cpu.set_d0(data); return Ok("[locale] FormatString NULL template".into()); }
    let entry = hook_entry(a, hook, "FormatString")?;
    let t = crate::guest_str::read_bytes_z(&a.mem, tpl, 4096).map_err(Refusal::mem("FormatString.template"))?;
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
        if t.get(i).map_or(false, u8::is_ascii_digit) {
            let n = num(&mut i);
            if t.get(i) == Some(&b'$') { i += 1; arg = n; } else { i = save; }
        }
        let left = t.get(i) == Some(&b'-'); if left { i += 1; }
        let zero = t.get(i) == Some(&b'0'); if zero { i += 1; }
        let width = num(&mut i);
        let mut limit = usize::MAX;
        if t.get(i) == Some(&b'.') { i += 1; if t.get(i).map_or(false, u8::is_ascii_digit) { limit = num(&mut i); } }
        let size = match t.get(i) {
            Some(b'i') => { i += 1; 4 }
            Some(b'L') => return Err(Refusal("FormatString: 64-bit arguments (%L, %ll) are not modelled".into())),
            Some(b'l') => { i += 1; if t.get(i) == Some(&b'l') { return Err(Refusal("FormatString: 64-bit arguments (%L, %ll) are not modelled".into())); } 4 }
            _ => 2,
        };
        let Some(&ty) = t.get(i) else { break };
        i += 1;
        let size = if matches!(ty, b'b' | b's' | b'p' | b'P') { 4 } else { size };
        items.push(Item::Dir(Dir { arg, left, zero, width, limit, size, ty }));
    }
    let mut args: Vec<Arg> = items.iter().filter_map(|it| if let Item::Dir(d) = it { Some(Arg { pos: d.arg, size: d.size }) } else { None }).collect();
    args.sort_by_key(|x| x.pos);
    let off_of = |pos: usize| -> u32 { args.iter().filter(|x| x.pos < pos).map(|x| x.size).sum() };
    let total: u32 = args.iter().map(|x| x.size).sum();
    // Pass 2: format.
    let rd = |a: &Amiga<U>, o: u32, size: u32| -> Result<u32, Refusal> {
        if size == 4 { a.mem.read_u32(data + o) } else { a.mem.read_u16(data + o).map(u32::from) }.map_err(Refusal::mem("FormatString.dataStream"))
    };
    let mut out: Vec<u8> = Vec::new();
    for it in &items {
        let d = match it { Item::Lit(c) => { out.push(*c); continue } Item::Dir(d) => d };
        let o = off_of(d.arg);
        let v = rd(a, o, d.size)?;
        let mut body: Vec<u8> = match d.ty {
            b'd' | b'D' => {
                let neg = if d.size == 4 { (v as i32) < 0 } else { (v as u16 as i16) < 0 };
                let mag = if d.size == 4 { (v as i32).unsigned_abs() } else { (v as u16 as i16).unsigned_abs() as u32 };
                let mut s = if d.ty == b'D' { group(mag) } else { format!("{mag}").into_bytes() };
                if neg { s.insert(0, b'-'); }
                s
            }
            b'u' | b'U' => { let mag = if d.size == 4 { v } else { v & 0xFFFF }; if d.ty == b'U' { group(mag) } else { format!("{mag}").into_bytes() } }
            b'x' => format!("{:X}", if d.size == 4 { v } else { v & 0xFFFF }).into_bytes(),   // RawDoFmt's %lx is UPPER case
            b'X' => format!("{:x}", if d.size == 4 { v } else { v & 0xFFFF }).into_bytes(),
            b'p' | b'P' => { let s = format!("{v:08x}"); if d.ty == b'p' { s.to_uppercase() } else { s }.into_bytes() }
            b'c' => alloc::vec![v as u8],
            b's' | b'b' => {
                let mut p = v;
                let mut s = Vec::new();
                if d.ty == b'b' { p <<= 2; let n = if p == 0 { 0 } else { a.mem.read_u8(p).map_err(Refusal::mem("FormatString %b"))? as u32 }; if p != 0 { p += 1; } for k in 0..n.min(d.limit as u32) { s.push(a.mem.read_u8(p + k).map_err(Refusal::mem("FormatString %b"))?); } }
                else if p != 0 { while s.len() < d.limit { let c = a.mem.read_u8(p + s.len() as u32).map_err(Refusal::mem("FormatString %s"))?; if c == 0 { break; } s.push(c); } }
                s
            }
            other => alloc::vec![other],   // an unknown type: the faulty '%' is ignored, the character printed
        };
        if matches!(d.ty, b'p' | b'P') && body.len() < 8 { body.splice(0..0, core::iter::repeat(b'0').take(8 - body.len())); }
        let pad = d.width.saturating_sub(body.len());
        if d.left { out.extend(body); out.extend(core::iter::repeat(b' ').take(pad)); }
        else { out.extend(core::iter::repeat(if d.zero { b'0' } else { b' ' }).take(pad)); out.extend(body); }
    }
    out.push(0);
    let line = format!("[locale] FormatString \"{}\" -> {} characters", String::from_utf8_lossy(&t), out.len() - 1);
    guest_call::start(a, cpu, emit(out, 0, hook, entry, loc, data + total))?;
    Ok(line)
}

/// Decimal digits of `n` grouped by 3 with the default locale's separator (loc_Grouping "\x03\x00", loc_GroupSeparator ",").
fn group(n: u32) -> Vec<u8> {
    let s = format!("{n}");
    let mut out = Vec::new();
    for (i, c) in s.bytes().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(b','); } out.push(c); }
    out
}

/// ParseDate(a0 = locale, a1 = DateStamp or NULL, a2 = template, a3 = getChar hook) -> TRUE if the input matched.
fn parse_date<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let loc = locale_arg(a, cpu.areg(0), "ParseDate")?;
    let (ds, tpl, hook) = (cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if tpl == 0 || hook == 0 { cpu.set_d0(0); return Ok("[locale] ParseDate: no template or hook -> FALSE".into()); }
    let entry = hook_entry(a, hook, "ParseDate")?;
    let t = crate::guest_str::read_bytes_z(&a.mem, tpl, 4096).map_err(Refusal::mem("ParseDate.template"))?;
    if t.is_empty() { cpu.set_d0(0); return Ok("[locale] ParseDate: empty template -> FALSE".into()); }
    // Read the whole input through the hook, then parse it.
    fn pull<U: GuestUi>(got: Vec<u8>, hook: u32, entry: u32, loc: u32, ds: u32, t: Vec<u8>) -> Next<U> {
        Next::Call {
            target: entry, d0: 0, aregs: alloc::vec![(0, hook), (1, 0), (2, loc)],
            then: Box::new(move |a: &mut Amiga<U>, _, c| {
                let mut got = got;
                if c & 0xFF != 0 && got.len() < MAX_INPUT { got.push(c as u8); return Ok(pull(got, hook, entry, loc, ds, t)); }
                let r = parse(&t, &got);
                if let (Some((days, minute, tick)), true) = (r, ds != 0) {
                    let m = Refusal::mem("DateStamp");
                    a.mem.write_u32(ds, days as u32).map_err(m)?;
                    a.mem.write_u32(ds + 4, minute).map_err(Refusal::mem("DateStamp"))?;
                    a.mem.write_u32(ds + 8, tick).map_err(Refusal::mem("DateStamp"))?;
                }
                Ok(Next::Return(r.is_some() as u32))
            }),
        }
    }
    guest_call::start(a, cpu, pull(Vec::new(), hook, entry, loc, ds, t))?;
    Ok("[locale] ParseDate: reading the input".into())
}

fn upper(c: u8) -> u8 { TO_UPPER[c as usize] }
fn is_digit(c: u8) -> bool { CTYPE[c as usize] & 4 != 0 }
fn is_space(c: u8) -> bool { CTYPE[c as usize] & 0x80 != 0 }

/// The parser of AROS parsedate.c over a prefetched input: Some((days, minute, tick)) when `input` matches `t`.
pub fn parse(t: &[u8], input: &[u8]) -> Option<(i32, u32, u32)> {
    let mut pos = 0usize;
    let mut next = || { let c = input.get(pos).copied().unwrap_or(0); pos += 1; c };
    let (mut day, mut month, mut hour, mut min, mut sec, mut year) = (0i32, 0i32, 0i32, 0i32, 0i32, 1978i32);
    let (mut am, mut pm) = (false, false);
    let mut ti = 0usize;
    let tc = |ti: usize| t.get(ti).copied().unwrap_or(0);
    let mut check_eof;
    let mut c: u8;
    // _getnum: `c` already holds the first character.
    macro_rules! getnum { ($n:expr, $out:expr) => {{
        if !is_digit(c) { return None; }
        let mut val = (c - b'0') as i32;
        let mut left = $n;
        loop {
            left -= 1;
            if left < 1 { break; }
            c = next();
            if is_digit(c) { val = val * 10 + (c - b'0') as i32; }
            else {
                if c != tc(ti) { return None; }
                if c == 0 { check_eof = false; } else { ti += 1; }
                break;
            }
        }
        $out = val;
    }}; }
    while ti < t.len() {
        check_eof = true;
        let k = t[ti];
        ti += 1;
        if k != b'%' { c = next(); if c != k { return None; } continue; }
        let f = tc(ti);
        ti += 1;
        match f {
            b'a' | b'A' | b'b' | b'h' | b'B' => {
                let (base, count, names): (usize, usize, usize) = match f { b'a' => (ABDAY_1, 7, 7), b'A' => (DAY_1, 7, 7), b'B' => (MON_1, 12, 12), _ => (ABMON_1, 12, 12) };
                let _ = names;
                // Accept the characters up to the template's next character, then match them against the names.
                let mut got = Vec::new();
                c = next();
                while c != 0 && c != tc(ti) { got.push(upper(c)); c = next(); }
                if (c == 0 && tc(ti) != 0) || c != tc(ti) { return None; }
                let hit = (0..count).find(|&i| STRINGS[base + i].bytes().map(upper).eq(got.iter().copied()));
                let hit = hit?;
                if f != b'a' && f != b'A' { month = hit as i32 + 1; }
                if tc(ti) != 0 { ti += 1; }
                check_eof = false;
            }
            b'd' => { c = next(); getnum!(2, day); day -= 1; if day < 0 { return None; } }
            b'e' => { c = next(); while is_space(c) { c = next(); } getnum!(2, day); day -= 1; if day < 0 { return None; } }
            b'H' => { am = false; pm = false; c = next(); getnum!(2, hour); if hour > 23 { return None; } }
            b'I' => { c = next(); getnum!(2, hour); if hour > 12 || hour == 0 { return None; } }
            b'm' => { c = next(); getnum!(2, month); if month > 12 || month == 0 { return None; } }
            b'M' => { c = next(); getnum!(2, min); if min > 59 { return None; } }
            b'S' => { c = next(); getnum!(2, sec); if sec > 59 { return None; } }
            b'p' => {
                let (am_s, pm_s) = (STRINGS[AM_STR].as_bytes(), STRINGS[PM_STR].as_bytes());
                let (mut am_ok, mut pm_ok, mut i) = (true, true, 0usize);
                c = next();
                while c != 0 && c != tc(ti) {
                    let (x, y, cc) = (am_s.get(i).copied().map_or(0, upper), pm_s.get(i).copied().map_or(0, upper), upper(c));
                    if am_ok && x != 0 && x != cc { am_ok = false; }
                    if pm_ok && y != 0 && y != cc { pm_ok = false; }
                    i += 1;
                    c = next();
                }
                if c != tc(ti) { return None; }
                am = am_ok; pm = pm_ok;
                if tc(ti) != 0 { ti += 1; }
                check_eof = false;
            }
            b'y' => {
                c = next();
                getnum!(4, year);
                if (100..1978).contains(&year) { return None; }
                if year < 78 { year += 100; }
                if year < 1900 { year += 1900; }
            }
            b'Y' => {
                year = 0;
                for p in [1000, 100, 10, 1] { c = next(); if !is_digit(c) { return None; } year += (c - b'0') as i32 * p; }
                if year < 1978 { return None; }
            }
            _ => return None,
        }
        let _ = check_eof;
        if ti >= t.len() && check_eof { if next() != 0 { return None; } }
    }
    let month = if month == 0 { 1 } else { month };
    let leap = year % 400 == 0 || (year % 4 == 0 && year % 100 != 0);
    let len = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(month - 1) as usize];
    if day >= len { return None; }
    let mut minute = hour * 60 + min;
    if hour == 12 && am { minute -= 720; }
    if hour < 12 && pm { minute += 720; }
    Some((days_of(year, (month - 1) as usize, day + 1), minute as u32, (sec * 50) as u32))
}
