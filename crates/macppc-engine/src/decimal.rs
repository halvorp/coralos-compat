//! SANE/fp.h decimal conversion for MathLib: `dec2num`, `num2dec` and `nan`, as pure functions over the 42-byte `decimal` record
//! (sgn @0, exp @2 (i16), sig.length @4, sig.text @5.., up to 36 digits) and the `decform` (style @0: FLOATDECIMAL 0 / FIXEDDECIMAL 1, digits @2).
//! Specs (clean room): macos9-specs/MathLib/{dec2num,num2dec,nan}.
//! Named limits: the finite dec2num path rounds to nearest-even (the CURRENT FPSCR rounding mode is not consulted) and raises no floating-point status flags; num2dec's digit
//! generation uses Rust's exact decimal expansion (correctly rounded, nearest-even) and does not set the inexact flag; FIXEDDECIMAL with a NEGATIVE digit count (rounding to
//! tens, hundreds...) is refused by name; an NaN code is written as up to 8 hexadecimal digits of the 30-bit code, left-aligned with trailing zeros dropped, the inverse of dec2num.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const DECIMAL_SIZE: usize = 42;

/// `dec2num`: the double (as bits) for the decimal record.
pub fn dec2num_bits(rec: &[u8]) -> u64 {
    let sgn = rec[0] != 0;
    let (exp, len) = (i16::from_be_bytes([rec[2], rec[3]]), usize::from(rec[4]).min(36));
    let text = &rec[5..5 + len];
    let sign = if sgn { 1u64 << 63 } else { 0 };
    match text.first() {
        None | Some(b'0') => sign,
        Some(b'I') => sign | 0x7FF0_0000_0000_0000,
        Some(b'N') => {
            // up to 8 characters after the N, each a hexadecimal nibble ('A'..'F' = value - '0' + 9 masked to 4 bits), left-aligned; missing ones are 0
            let mut v = 0u32;
            for i in 0..8 {
                let nib = text.get(1 + i).map_or(0, |&c| { let d = c.wrapping_sub(b'0'); u32::from(if d > 9 { d.wrapping_add(9) } else { d } & 0xF) });
                v = v << 4 | nib;
            }
            let mut code = v & 0x3FFF_FFFF;
            if code == 0 { code = 0x15_0000; }
            let hi = ((code | 0x4000_0000) >> 11) | 0x7FF0_0000;
            let lo = code << 23;
            sign | (u64::from(hi) << 32) | u64::from(lo)
        }
        Some(_) => {
            let digits: String = text.iter().map(|&c| char::from(c)).collect();
            let s = format!("{digits}e{exp}");
            let v: f64 = s.parse().unwrap_or(0.0);
            sign | v.abs().to_bits()
        }
    }
}

/// `nan("tag")`: the quiet NaN with the library's code: the tag is scanned as a decimal number and the integer (saturated to i16; INF/NAN spellings are -32768) adds k * 0x20 to the high word, clamped at 255.
pub fn nan_bits(tag: &[u8]) -> u64 {
    let s = String::from_utf8_lossy(tag);
    let t = s.trim_start_matches([' ', '\t']);
    let up = t.trim_start_matches(['+', '-']).to_ascii_uppercase();
    let k: i32 = if up.starts_with("INF") || up.starts_with("NAN") { -32768 } else {
        // sign, digits, optional fraction and exponent: the longest numeric prefix
        let b = t.as_bytes();
        let mut i = usize::from(!b.is_empty() && (b[0] == b'+' || b[0] == b'-'));
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; }
        if i < b.len() && b[i] == b'.' { i += 1; while i < b.len() && b[i].is_ascii_digit() { i += 1; } }
        if i == start { 0 } else {
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') { j += 1; }
                let d = j;
                while j < b.len() && b[j].is_ascii_digit() { j += 1; }
                if j > d { i = j; }
            }
            let v: f64 = t[..i].parse().unwrap_or(0.0);
            let r = libm::rint(v);
            r.clamp(-32768.0, 32767.0) as i32
        }
    };
    let add = if k >= 0 { (k.min(255) as u32) * 0x20 } else { 0 };
    u64::from(0x7FF8_0000u32 + add) << 32
}

/// `num2dec`: the decimal record for `x` under the decform (`style`, `digits`).
pub fn num2dec(style: u8, digits: i16, x: f64) -> Result<[u8; DECIMAL_SIZE], String> {
    let mut d = [0u8; DECIMAL_SIZE];
    let bits = x.to_bits();
    d[0] = u8::from(bits >> 63 != 0);
    let set = |d: &mut [u8; DECIMAL_SIZE], text: &[u8], exp: i16| { d[2..4].copy_from_slice(&exp.to_be_bytes()); d[4] = text.len() as u8; d[5..5 + text.len()].copy_from_slice(text); };
    if x.is_nan() {
        let (hi, lo) = ((bits >> 32) as u32, bits as u32);
        let code = (((hi & 0xF_FFFF) << 11) | (lo >> 23)) & 0x3FFF_FFFF;
        let mut hex = format!("{code:08X}");
        while hex.ends_with('0') { hex.pop(); }
        let mut t = alloc::vec![b'N']; t.extend_from_slice(hex.as_bytes());
        set(&mut d, &t, 0);
    } else if x.is_infinite() { set(&mut d, b"I", 0); }
    else if x == 0.0 { set(&mut d, b"0", 0); }
    else {
        let a = x.abs();
        match style {
            0 => {
                let n = if digits <= 0 { 1 } else { digits.min(36) } as usize;
                let s = format!("{:.*e}", n - 1, a); // d.ddddde[-]N
                let (mant, e) = s.split_once('e').ok_or("num2dec: float formatting")?;
                let e: i32 = e.parse().map_err(|_| "num2dec: exponent")?;
                let text: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).collect();
                set(&mut d, &text, (e - (n as i32 - 1)) as i16);
            }
            1 => {
                if digits < 0 { return Err(format!("num2dec: FIXEDDECIMAL with {digits} digits (rounding to tens/hundreds) is not implemented")); }
                let s = format!("{:.*}", digits as usize, a);
                let text: Vec<u8> = s.bytes().filter(u8::is_ascii_digit).collect();
                let first = text.iter().position(|&c| c != b'0');
                match first {
                    None => set(&mut d, b"0", -digits),
                    Some(f) => {
                        let sig = &text[f..];
                        // more than 36 digits: cut to 36 and raise the exponent
                        let cut = sig.len().saturating_sub(36);
                        set(&mut d, &sig[..sig.len() - cut], (-(digits as i32) + cut as i32) as i16);
                    }
                }
            }
            s => return Err(format!("num2dec: decform style {s} is neither FLOATDECIMAL nor FIXEDDECIMAL")),
        }
    }
    Ok(d)
}
