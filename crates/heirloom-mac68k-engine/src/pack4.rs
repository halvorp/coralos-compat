//! `_Pack4` (`$A9EB`): Apple SANE `FP68K` floating-point arithmetic package
//! (*Apple Numerics Manual, 2nd Ed.*, Chapter 10 & Appendix C).
//!
//! Calling convention: caller pushes operand pointer(s) (`src`, then `dst` for binary ops;
//! `dst` alone for unary ops), then a 16-bit opcode word `op`, then invokes `_Pack4` (`$A9EB`).
//! All arguments and the opcode word are popped before returning.
//!
//! Bits 11..13 of `op` select the non-extended operand format:
//! - `0` (`FFEXT`): 80-bit SANE extended (10 bytes big-endian: 2-byte sign+exp, 8-byte explicit-bit mantissa)
//! - `1` (`FFDBL`): 64-bit IEEE double
//! - `2` (`FFSGL`): 32-bit IEEE single
//! - `4` (`FFINT`): 16-bit signed integer
//! - `5` (`FFLNG`): 32-bit signed integer
//! - `6` (`FFCOMP`): 64-bit signed integer (`Comp`, `-2^63` = NaN)
use crate::cpu::Cpu;
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA9EB, pack4::<U>)
}

const FFEXT: u16 = 0;
const FFDBL: u16 = 1;
const FFSGL: u16 = 2;
const FFINT: u16 = 4;
const FFLNG: u16 = 5;
const FFCOMP: u16 = 6;

const FOADD: u16 = 0x00;
const FOSUB: u16 = 0x02;
const FOMUL: u16 = 0x04;
const FODIV: u16 = 0x06;
const FOCMP: u16 = 0x08;
const FOCPX: u16 = 0x0A;
const FOREM: u16 = 0x0C;
const FOZ2X: u16 = 0x0E;
const FOX2Z: u16 = 0x10;
const FOSQRT: u16 = 0x12;
const FORTI: u16 = 0x14;
const FOTTI: u16 = 0x16;
const FOSCALB: u16 = 0x18;
const FOLOGB: u16 = 0x1A;
const FOCLASS: u16 = 0x1C;
const FOSETENV: u16 = 0x01;
const FOGETENV: u16 = 0x03;
const FOSETHV: u16 = 0x05;
const FOGETHV: u16 = 0x07;
const FONEG: u16 = 0x0D;
const FOABS: u16 = 0x0F;
const FOCPYSGN: u16 = 0x11;
const FOPROCENTRY: u16 = 0x17;
const FOPROCEXIT: u16 = 0x19;
const FOTESTXCP: u16 = 0x1B;
const FOSETXCP: u16 = 0x1D;

/// Convert a 10-byte big-endian SANE 80-bit extended value to IEEE `f64`.
pub fn ext80_to_f64(bytes: [u8; 10]) -> f64 {
    let sign = (bytes[0] & 0x80) != 0;
    let exp = u16::from_be_bytes([bytes[0] & 0x7F, bytes[1]]);
    let mant = u64::from_be_bytes([
        bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9],
    ]);
    let sbit = if sign { 1u64 << 63 } else { 0 };
    if exp == 0 && mant == 0 {
        return f64::from_bits(sbit);
    }
    if exp == 0x7FFF {
        if (mant & 0x7FFF_FFFF_FFFF_FFFF) == 0 {
            return f64::from_bits(sbit | 0x7FF0_0000_0000_0000);
        }
        return f64::NAN;
    }
    // Unbiased exponent for explicit integer bit at bit 63.
    let mut e = i32::from(exp) - 16383;
    let mut m = mant;
    if exp == 0 {
        // Subnormal 80-bit extended: effective exponent is -16382.
        e = -16382;
    }
    if m == 0 {
        return f64::from_bits(sbit);
    }
    let lz = m.leading_zeros() as i32;
    m <<= lz;
    e -= lz;
    let d_exp = e + 1023;
    if d_exp >= 0x7FF {
        return f64::from_bits(sbit | 0x7FF0_0000_0000_0000);
    }
    if d_exp <= 0 {
        let shift = 11 + (1 - d_exp);
        if shift >= 64 {
            return f64::from_bits(sbit);
        }
        return f64::from_bits(sbit | (m >> shift));
    }
    // Round-to-nearest-even from 64-bit mantissa (bit 63 explicit) to 52-bit fraction.
    let frac = (m >> 11) & 0x000F_FFFF_FFFF_FFFF;
    let rem = m & 0x7FF;
    let rounded = if rem > 0x400 || (rem == 0x400 && (frac & 1) != 0) {
        frac + 1
    } else {
        frac
    };
    let (final_exp, final_frac) = if rounded >= (1u64 << 52) {
        ((d_exp + 1) as u64, 0)
    } else {
        (d_exp as u64, rounded)
    };
    if final_exp >= 0x7FF {
        return f64::from_bits(sbit | 0x7FF0_0000_0000_0000);
    }
    f64::from_bits(sbit | (final_exp << 52) | final_frac)
}

/// Convert an IEEE `f64` to a 10-byte big-endian SANE 80-bit extended value.
pub fn f64_to_ext80(v: f64) -> [u8; 10] {
    let bits = v.to_bits();
    let sign = (bits >> 63) != 0;
    let d_exp = ((bits >> 52) & 0x7FF) as i32;
    let d_frac = bits & 0x000F_FFFF_FFFF_FFFF;
    let s16: u16 = if sign { 0x8000 } else { 0 };

    let (e16, mant): (u16, u64) = if d_exp == 0 && d_frac == 0 {
        (0, 0)
    } else if d_exp == 0x7FF {
        if d_frac == 0 {
            (0x7FFF, 0x8000_0000_0000_0000)
        } else {
            (0x7FFF, 0xC000_0000_0000_0000 | (d_frac << 11))
        }
    } else if d_exp == 0 {
        // Subnormal f64 -> normal ext80
        let lz = d_frac.leading_zeros() as i32 - 11; // bit 52..0
        let shifted = d_frac << (lz + 11);
        let e = -1022 - lz + 16383;
        (e as u16, shifted)
    } else {
        let e = (d_exp - 1023 + 16383) as u16;
        (e, 0x8000_0000_0000_0000 | (d_frac << 11))
    };

    let mut out = [0u8; 10];
    let hi = (s16 | (e16 & 0x7FFF)).to_be_bytes();
    out[0] = hi[0];
    out[1] = hi[1];
    out[2..10].copy_from_slice(&mant.to_be_bytes());
    out
}

fn no_std_trunc(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
    if exp < 0 { return f64::from_bits(bits & (1u64 << 63)); }
    if exp >= 52 { return x; }
    let mask = !((1u64 << (52 - exp)) - 1);
    f64::from_bits(bits & mask)
}

fn no_std_round_even(x: f64) -> f64 {
    if !x.is_finite() { return x; }
    let t = no_std_trunc(x);
    let diff = (x - t).abs();
    if diff > 0.5 {
        if x >= 0.0 { t + 1.0 } else { t - 1.0 }
    } else if diff < 0.5 {
        t
    } else {
        // Tie to even
        let half = t * 0.5;
        if no_std_trunc(half) == half { t } else if x >= 0.0 { t + 1.0 } else { t - 1.0 }
    }
}

fn no_std_sqrt(x: f64) -> f64 {
    if x < 0.0 || x.is_nan() { return f64::NAN; }
    if x == 0.0 || x.is_infinite() { return x; }
    // Initial guess from halving the exponent, then 6 Newton-Raphson steps (exact for f64).
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
    let half_exp = ((exp >> 1) + 1023) as u64;
    let mut g = f64::from_bits((half_exp << 52) | ((bits & 0x000F_FFFF_FFFF_FFFF) >> 1));
    for _ in 0..8 {
        g = 0.5 * (g + x / g);
    }
    g
}

fn read_operand<U: GuestUi>(m: &Mac<U>, addr: u32, fmt: u16) -> Result<f64, Refusal> {
    let e = Refusal::mem("FP68K operand");
    match fmt {
        FFEXT => {
            let mut b = [0u8; 10];
            for (i, slot) in b.iter_mut().enumerate() {
                *slot = m.mem.read_u8(addr.wrapping_add(i as u32)).map_err(&e)?;
            }
            Ok(ext80_to_f64(b))
        }
        FFDBL => {
            let hi = m.mem.read_u32(addr).map_err(&e)? as u64;
            let lo = m.mem.read_u32(addr.wrapping_add(4)).map_err(&e)? as u64;
            Ok(f64::from_bits((hi << 32) | lo))
        }
        FFSGL => Ok(f64::from(f32::from_bits(m.mem.read_u32(addr).map_err(&e)?))),
        FFINT => Ok(f64::from(m.mem.read_i16(addr).map_err(&e)?)),
        FFLNG => Ok(f64::from(m.mem.read_u32(addr).map_err(&e)? as i32)),
        FFCOMP => {
            let hi = m.mem.read_u32(addr).map_err(&e)? as u64;
            let lo = m.mem.read_u32(addr.wrapping_add(4)).map_err(&e)? as u64;
            let v = ((hi << 32) | lo) as i64;
            if v == i64::MIN { Ok(f64::NAN) } else { Ok(v as f64) }
        }
        _ => Err(Refusal(format!("FP68K: unsupported operand format {fmt}"))),
    }
}

fn write_operand<U: GuestUi>(m: &mut Mac<U>, addr: u32, fmt: u16, v: f64) -> Result<(), Refusal> {
    let e = Refusal::mem("FP68K result");
    match fmt {
        FFEXT => {
            let b = f64_to_ext80(v);
            for (i, byte) in b.into_iter().enumerate() {
                m.mem.write_u8(addr.wrapping_add(i as u32), byte).map_err(&e)?;
            }
            Ok(())
        }
        FFDBL => {
            let bits = v.to_bits();
            m.mem.write_u32(addr, (bits >> 32) as u32).map_err(&e)?;
            m.mem.write_u32(addr.wrapping_add(4), bits as u32).map_err(&e)
        }
        FFSGL => m.mem.write_u32(addr, (v as f32).to_bits()).map_err(&e),
        FFINT => {
            let r = no_std_round_even(v);
            let i = if r.is_nan() || r <= f64::from(i16::MIN) { i16::MIN } else if r >= f64::from(i16::MAX) { i16::MAX } else { r as i16 };
            m.mem.write_u16(addr, i as u16).map_err(&e)
        }
        FFLNG => {
            let r = no_std_round_even(v);
            let i = if r.is_nan() || r <= f64::from(i32::MIN) { i32::MIN } else if r >= f64::from(i32::MAX) { i32::MAX } else { r as i32 };
            m.mem.write_u32(addr, i as u32).map_err(&e)
        }
        FFCOMP => {
            let i: i64 = if v.is_nan() { i64::MIN } else { no_std_round_even(v) as i64 };
            m.mem.write_u32(addr, (i as u64 >> 32) as u32).map_err(&e)?;
            m.mem.write_u32(addr.wrapping_add(4), i as u32).map_err(&e)
        }
        _ => Err(Refusal(format!("FP68K: unsupported destination format {fmt}"))),
    }
}

fn is_unary(code: u16) -> bool {
    matches!(
        code,
        FOSQRT | FORTI | FOTTI | FOLOGB | FONEG | FOABS
            | FOSETENV | FOGETENV | FOSETHV | FOGETHV
            | FOPROCENTRY | FOPROCEXIT | FOTESTXCP | FOSETXCP
    )
}

fn pack4<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let sp = cpu.areg(7);
    let e = Refusal::mem("FP68K stack");
    let op = m.mem.read_u16(sp).map_err(&e)?;
    let code = op & 0x001F;
    let fmt = (op >> 11) & 0x7;

    if is_unary(code) {
        let dst = m.mem.read_u32(sp.wrapping_add(2)).map_err(&e)?;
        cpu.set_areg(7, sp.wrapping_add(6));
        match code {
            FOSQRT => {
                let d = read_operand(m, dst, FFEXT)?;
                write_operand(m, dst, FFEXT, no_std_sqrt(d))?;
            }
            FORTI => {
                let d = read_operand(m, dst, FFEXT)?;
                write_operand(m, dst, FFEXT, no_std_round_even(d))?;
            }
            FOTTI => {
                let d = read_operand(m, dst, FFEXT)?;
                write_operand(m, dst, FFEXT, no_std_trunc(d))?;
            }
            FONEG => {
                let d = read_operand(m, dst, FFEXT)?;
                write_operand(m, dst, FFEXT, -d)?;
            }
            FOABS => {
                let d = read_operand(m, dst, FFEXT)?;
                write_operand(m, dst, FFEXT, d.abs())?;
            }
            FOLOGB => {
                let d = read_operand(m, dst, FFEXT)?;
                let res = if d == 0.0 {
                    f64::NEG_INFINITY
                } else if d.is_infinite() {
                    f64::INFINITY
                } else if d.is_nan() {
                    f64::NAN
                } else {
                    let bits = d.abs().to_bits();
                    let exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
                    f64::from(exp)
                };
                write_operand(m, dst, FFEXT, res)?;
            }
            FOGETENV | FOPROCENTRY => {
                m.mem.write_u16(dst, 0).map_err(&e)?;
            }
            FOGETHV => {
                m.mem.write_u32(dst, 0).map_err(&e)?;
            }
            FOTESTXCP => {
                m.mem.write_u8(dst, 0).map_err(&e)?;
            }
            FOSETENV | FOSETHV | FOPROCEXIT | FOSETXCP => {}
            _ => unreachable!(),
        }
        return Ok(format!("[pack4] FP68K unary op={op:#06x} dst={dst:#x}"));
    }

    // Binary operation: sp+0 = op, sp+2 = dst_ptr, sp+6 = src_ptr
    let dst = m.mem.read_u32(sp.wrapping_add(2)).map_err(&e)?;
    let src = m.mem.read_u32(sp.wrapping_add(6)).map_err(&e)?;
    cpu.set_areg(7, sp.wrapping_add(10));

    match code {
        FOADD => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            write_operand(m, dst, FFEXT, d + s)?;
        }
        FOSUB => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            write_operand(m, dst, FFEXT, d - s)?;
        }
        FOMUL => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            write_operand(m, dst, FFEXT, d * s)?;
        }
        FODIV => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            write_operand(m, dst, FFEXT, d / s)?;
        }
        FOCMP | FOCPX => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            let ccr = if d.is_nan() || s.is_nan() {
                0x02 // V (unordered)
            } else if d < s {
                0x19 // X, N, C
            } else if d > s {
                0x00
            } else {
                0x04 // Z
            };
            cpu.set_sr((cpu.sr() & !0x1F) | ccr);
        }
        FOREM => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            let q = no_std_round_even(d / s);
            let rem = d - q * s;
            write_operand(m, dst, FFEXT, rem)?;
            let qi = (q as i64) & 0x7F;
            let qword = if q < 0.0 { (-qi) as i16 as u16 } else { qi as u16 };
            r.d0 = (r.d0 & 0xFFFF_0000) | u32::from(qword);
        }
        FOZ2X => {
            let s = read_operand(m, src, fmt)?;
            write_operand(m, dst, FFEXT, s)?;
        }
        FOX2Z => {
            // Note SANE convention: FOX2Z converts extended at `src` into format `fmt` at `dst`!
            let s = read_operand(m, src, FFEXT)?;
            write_operand(m, dst, fmt, s)?;
        }
        FOSCALB => {
            let exp = m.mem.read_i16(src).map_err(&e)?;
            let d = read_operand(m, dst, FFEXT)?;
            let mut factor = 1.0f64;
            let n = i32::from(exp);
            if n >= 0 {
                for _ in 0..n.min(1024) { factor *= 2.0; }
            } else {
                for _ in 0..(-n).min(1024) { factor *= 0.5; }
            }
            write_operand(m, dst, FFEXT, d * factor)?;
        }
        FOCLASS => {
            let s = read_operand(m, src, fmt)?;
            // SANE class codes (Apple Numerics Manual): 1=SNaN, 2=QNaN, 3=Inf, 4=Zero, 5=Normal, 6=Subnormal; negative if sign bit set.
            let mag: i16 = if s.is_nan() { 2 } else if s.is_infinite() { 3 } else if s == 0.0 { 4 } else { 5 };
            let cls = if s.is_sign_negative() { -mag } else { mag };
            m.mem.write_u16(dst, cls as u16).map_err(&e)?;
        }
        FOCPYSGN => {
            let s = read_operand(m, src, fmt)?;
            let d = read_operand(m, dst, FFEXT)?;
            let res = if d.is_sign_negative() { -s.abs() } else { s.abs() };
            write_operand(m, dst, FFEXT, res)?;
        }
        _ => {
            return Err(Refusal(format!(
                "FP68K (_Pack4): unimplemented opcode {op:#06x} (code={code:#x}, fmt={fmt})"
            )));
        }
    }
    Ok(format!("[pack4] FP68K op={op:#06x} fmt={fmt} dst={dst:#x} src={src:#x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext80_f64_roundtrip_specials_and_normals() {
        for &v in &[0.0, -0.0, 1.0, -1.0, 3.141592653589793, 1e-100, 1e100, -42.5, f64::INFINITY, f64::NEG_INFINITY] {
            let e = f64_to_ext80(v);
            let back = ext80_to_f64(e);
            assert_eq!(back.to_bits(), v.to_bits(), "roundtrip mismatch for {v}");
        }
        assert!(ext80_to_f64(f64_to_ext80(f64::NAN)).is_nan());
        // Check exact Apple SANE 80-bit encoding of 1.0: exp=0x3FFF, mant=0x8000_0000_0000_0000
        assert_eq!(f64_to_ext80(1.0), [0x3F, 0xFF, 0x80, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(f64_to_ext80(-2.0), [0xC0, 0x00, 0x80, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn no_std_math_helpers_are_exact() {
        assert_eq!(no_std_trunc(3.9), 3.0);
        assert_eq!(no_std_trunc(-3.9), -3.0);
        assert_eq!(no_std_round_even(2.5), 2.0);
        assert_eq!(no_std_round_even(3.5), 4.0);
        assert!((no_std_sqrt(9.0) - 3.0).abs() < 1e-15);
        assert!((no_std_sqrt(2.0) - core::f64::consts::SQRT_2).abs() < 1e-15);
        assert!(no_std_sqrt(-1.0).is_nan());
    }
}
