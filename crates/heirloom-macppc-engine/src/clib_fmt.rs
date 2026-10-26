//! `printf`-family formatting over the PowerPC argument registers (ISO C 7.19.6.1; Mac OS Runtime Architectures calling convention).
//! Integer and pointer arguments are consumed from the word slots r3..r10 then the parameter area; a `double` comes from the next FPR (f1..f13)
//! AND occupies two word slots (the ABI reserves GPR space for it, so later integers are found past it). `%n` is refused by name.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_ppc::Cpu;

#[derive(Default, Clone, Copy)]
struct Spec { minus: bool, plus: bool, space: bool, alt: bool, zero: bool, width: usize, prec: Option<usize> }

fn pad(s: Vec<u8>, sp: &Spec, numeric: bool) -> Vec<u8> {
    if s.len() >= sp.width { return s; }
    let fill = sp.width - s.len();
    if sp.minus { let mut s = s; s.extend(core::iter::repeat(b' ').take(fill)); s }
    else if sp.zero && numeric {
        // zeros go after the sign / 0x prefix
        let at = if s.first().is_some_and(|c| matches!(c, b'-' | b'+' | b' ')) { 1 } else { 0 };
        let at = if s[at..].starts_with(b"0x") || s[at..].starts_with(b"0X") { at + 2 } else { at };
        let mut out = s[..at].to_vec(); out.extend(core::iter::repeat(b'0').take(fill)); out.extend_from_slice(&s[at..]); out
    } else { let mut out: Vec<u8> = core::iter::repeat(b' ').take(fill).collect(); out.extend(s); out }
}

fn int_text(mag: u64, neg: bool, conv: u8, sp: &Spec) -> Vec<u8> {
    let mut digits = match conv { b'o' => format!("{mag:o}"), b'x' => format!("{mag:x}"), b'X' => format!("{mag:X}"), _ => format!("{mag}") };
    if let Some(p) = sp.prec { if p == 0 && mag == 0 { digits.clear(); } while digits.len() < p { digits.insert(0, '0'); } }
    let mut out = String::new();
    if matches!(conv, b'd' | b'i') { if neg { out.push('-'); } else if sp.plus { out.push('+'); } else if sp.space { out.push(' '); } }
    if sp.alt && mag != 0 { match conv { b'x' => out.push_str("0x"), b'X' => out.push_str("0X"), b'o' if !digits.starts_with('0') => out.push('0'), _ => {} } }
    out.push_str(&digits);
    out.into_bytes()
}

/// `%e` text with C's exponent (at least two digits, explicit sign).
fn exp_text(v: f64, prec: usize, upper: bool, alt: bool) -> String {
    let s = format!("{v:.prec$e}"); // "1.500000e2" / "1e-5"
    let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = e.parse().unwrap_or(0);
    let mut m = m.to_string();
    if alt && prec == 0 { m.push('.'); }
    let out = format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    if upper { out.to_uppercase() } else { out }
}

fn float_text(v: f64, conv: u8, sp: &Spec) -> Vec<u8> {
    let upper = conv.is_ascii_uppercase();
    if !v.is_finite() {
        let t = if v.is_nan() { "nan" } else { "inf" };
        let mut s = String::new();
        if v.is_sign_negative() && !v.is_nan() { s.push('-'); } else if sp.plus { s.push('+'); } else if sp.space { s.push(' '); }
        s.push_str(t);
        return if upper { s.to_uppercase().into_bytes() } else { s.into_bytes() };
    }
    let prec = sp.prec.unwrap_or(6);
    let neg = v.is_sign_negative();
    let a = v.abs();
    let body = match conv.to_ascii_lowercase() {
        b'f' => { let mut s = format!("{a:.prec$}"); if sp.alt && prec == 0 { s.push('.'); } s }
        b'e' => exp_text(a, prec, upper, sp.alt),
        _ => { // %g: precision P (0 -> 1); exponent X of the %e form; fixed when P > X >= -4
            let p = if prec == 0 { 1 } else { prec };
            let es = exp_text(a, p - 1, false, false);
            let x: i32 = es.split_once('e').and_then(|(_, e)| e.parse().ok()).unwrap_or(0);
            let strip = |s: String| if sp.alt || !s.contains('.') { s } else { s.trim_end_matches('0').trim_end_matches('.').to_string() };
            let s = if (x as i64) < p as i64 && x >= -4 { strip(format!("{a:.*}", (p as i64 - 1 - x as i64).max(0) as usize)) }
                else { let (m, e) = es.split_once('e').unwrap_or((&es, "+00")); format!("{}e{e}", strip(m.to_string())) };
            if upper { s.to_uppercase() } else { s }
        }
    };
    let mut s = String::new();
    if neg { s.push('-'); } else if sp.plus { s.push('+'); } else if sp.space { s.push(' '); }
    s.push_str(&body);
    s.into_bytes()
}

impl<U: GuestUi> MacPpc<U> {
    /// Format the C string at `fmt` (argument index `fi` of the call); the variadic arguments follow it.
    pub(crate) fn format_c(&mut self, cpu: &Cpu, fi: usize, fmt: u32) -> Result<Vec<u8>, String> {
        let f = self.cstr(fmt, "printf format")?;
        let (mut k, mut fpr) = (fi + 1, 1usize);
        let mut out = Vec::new();
        let mut i = 0;
        macro_rules! word { () => {{ let v = arg(self, cpu, k)?; k += 1; v }} }
        while i < f.len() {
            if f[i] != b'%' { out.push(f[i]); i += 1; continue; }
            i += 1;
            let mut sp = Spec::default();
            while let Some(&c) = f.get(i) {
                match c { b'-' => sp.minus = true, b'+' => sp.plus = true, b' ' => sp.space = true, b'#' => sp.alt = true, b'0' => sp.zero = true, _ => break }
                i += 1;
            }
            if f.get(i) == Some(&b'*') { let w = word!() as i32; if w < 0 { sp.minus = true; } sp.width = w.unsigned_abs() as usize; i += 1; }
            else { while let Some(d) = f.get(i).filter(|c| c.is_ascii_digit()) { sp.width = sp.width * 10 + usize::from(d - b'0'); i += 1; } }
            if f.get(i) == Some(&b'.') {
                i += 1;
                if f.get(i) == Some(&b'*') { let p = word!() as i32; sp.prec = (p >= 0).then_some(p as usize); i += 1; }
                else { let mut p = 0; while let Some(d) = f.get(i).filter(|c| c.is_ascii_digit()) { p = p * 10 + usize::from(d - b'0'); i += 1; } sp.prec = Some(p); }
            }
            let mut long = 0u8; // 1 = l / z / t (a word), 2 = ll / j (two words), 0xFF = h, 0xFE = hh
            loop {
                match f.get(i) { Some(b'l') => { long = if long == 1 { 2 } else { 1 }; } Some(b'h') => long = if long == 0xFF { 0xFE } else { 0xFF }, Some(b'L') | Some(b'q') | Some(b'j') => long = 2, Some(b'z') | Some(b't') => long = 1, _ => break }
                i += 1;
            }
            let Some(&conv) = f.get(i) else { return Err("printf: the format ends inside a conversion".into()) };
            i += 1;
            let piece: Vec<u8> = match conv {
                b'%' => { out.push(b'%'); continue; }
                b'd' | b'i' => {
                    let v: i64 = if long == 2 { let hi = word!() as u64; let lo = word!() as u64; ((hi << 32) | lo) as i64 } else if long == 0xFF { word!() as i16 as i64 } else if long == 0xFE { word!() as i8 as i64 } else { word!() as i32 as i64 };
                    pad(int_text(v.unsigned_abs(), v < 0, conv, &sp), &sp, sp.prec.is_none())
                }
                b'u' | b'o' | b'x' | b'X' => {
                    let v: u64 = if long == 2 { let hi = word!() as u64; let lo = word!() as u64; (hi << 32) | lo } else if long == 0xFF { u64::from(word!() as u16) } else if long == 0xFE { u64::from(word!() as u8) } else { u64::from(word!()) };
                    pad(int_text(v, false, conv, &sp), &sp, sp.prec.is_none())
                }
                b'p' => { let v = word!(); let s = Spec { alt: true, ..sp }; pad(int_text(u64::from(v), false, b'x', &s), &sp, false) }
                b'c' => pad(alloc::vec![word!() as u8], &sp, false),
                b's' => {
                    let p = word!();
                    let mut s = if p == 0 { b"(null)".to_vec() } else { self.cstr(p, "printf %s")? };
                    if let Some(pr) = sp.prec { s.truncate(pr); }
                    pad(s, &sp, false)
                }
                b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => {
                    let v = if fpr <= 13 { f64::from_bits(cpu.fpr[fpr]) } else {
                        let at = cpu.gpr[1].wrapping_add(24 + 4 * k as u32);
                        let hi = u64::from(self.mac.mem.read_u32(at).map_err(|e| format!("printf: double argument at {at:#x}: {e}"))?);
                        let lo = u64::from(self.mac.mem.read_u32(at + 4).map_err(|e| format!("printf: double argument at {at:#x}: {e}"))?);
                        f64::from_bits(hi << 32 | lo)
                    };
                    fpr += 1; k += 2;
                    pad(float_text(v, conv, &sp), &sp, v.is_finite())
                }
                // CoreFoundation's %@ (used by CFStringCreateWithFormat): the text of a CFString argument; any other object is refused by name
                b'@' => {
                    let p = word!();
                    if p == 0 { pad(b"(null)".to_vec(), &sp, false) } else {
                        let t = self.cf_bytes(p).ok_or_else(|| format!("CFStringCreateWithFormat: %@ argument {p:#x} is not a CFString this engine made (other objects' descriptions are not modelled)"))?.to_vec();
                        pad(t, &sp, false)
                    }
                }
                b'n' => return Err("printf: %n is refused (it writes through a pointer the format chooses)".into()),
                other => return Err(format!("printf: unsupported conversion '%{}'", char::from(other))),
            };
            out.extend(piece);
        }
        Ok(out)
    }
}
