//! MathLib: the C math functions (`<math.h>`), natively over the host's `libm` (a pure-Rust port of musl's libm, so results do not depend on the
//! host CPU). Arguments arrive in f1, f2 and the result goes back in f1 (Mac OS Runtime Architectures); `ldexp`/`frexp`/`modf` take an int or pointer as parameter word 2 (r5): the leading double also occupies words 0-1, r3/r4.
//! Results are IEEE-754 doubles as the C standard defines them; MathLib's classic `-0` / NaN payload conventions are not reproduced.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFn { Acos, Asin, Atan, Atan2, Ceil, Cos, Cosh, Exp, Fabs, Floor, Fmin, Fmax, Fmod, Frexp, Hypot, Ldexp, Log, Log2, Log10, Modf, Pow, Rint, Round, Sin, Sinh, Sqrt, Tan, Tanh, Dec2num, Num2dec, Nan, X80tod, Dtox80, Fix2X, X2Fix, Frac2X, X2Frac }

impl MathFn {
    pub fn by_name(name: &str) -> Option<MathFn> {
        use MathFn::*;
        Some(match name {
            "acos" => Acos, "asin" => Asin, "atan" => Atan, "atan2" => Atan2, "ceil" => Ceil, "cos" => Cos, "cosh" => Cosh, "exp" => Exp, "fabs" => Fabs,
            "floor" => Floor, "fmod" => Fmod, "frexp" => Frexp, "hypot" => Hypot, "ldexp" => Ldexp, "log" => Log, "log2" => Log2, "log10" => Log10, "fmin" => Fmin, "fmax" => Fmax, "modf" => Modf, "pow" => Pow,
            "rint" => Rint, "round" => Round, "sin" => Sin, "sinh" => Sinh, "sqrt" => Sqrt, "tan" => Tan, "tanh" => Tanh, "dec2num" => Dec2num, "num2dec" => Num2dec, "nan" => Nan,
            "x80tod" => X80tod, "dtox80" => Dtox80,
            "Fix2X" => Fix2X, "X2Fix" => X2Fix, "Frac2X" => Frac2X, "X2Frac" => X2Frac,
            _ => return None,
        })
    }
}

/// Convert Apple SANE 80-bit extended (10 bytes big-endian) to IEEE-754 f64 bits.
pub(crate) fn x80_to_f64_bits(b: &[u8]) -> u64 {
    let exp_word = u16::from_be_bytes([b[0], b[1]]);
    let sig = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    let sign = u64::from(exp_word >> 15) << 63;
    let exp15 = (exp_word & 0x7FFF) as i32;
    if exp15 == 0x7FFF {
        return if (sig & 0x7FFF_FFFF_FFFF_FFFF) != 0 { sign | 0x7FF8_0000_0000_0000 } else { sign | 0x7FF0_0000_0000_0000 };
    }
    if exp15 == 0 && sig == 0 {
        return sign;
    }
    let mut e = exp15 - 16383 + 1023;
    let mut m = sig;
    while m != 0 && (m >> 63) == 0 {
        m <<= 1;
        e -= 1;
    }
    if m == 0 {
        return sign;
    }
    if e >= 2047 {
        return sign | 0x7FF0_0000_0000_0000;
    }
    if e <= 0 {
        let shift = (12 - e) as u32;
        return if shift >= 64 { sign } else { sign | (m >> shift) };
    }
    sign | ((e as u64) << 52) | ((m >> 11) & 0x000F_FFFF_FFFF_FFFF)
}

/// Convert IEEE-754 f64 bits to Apple SANE 80-bit extended (10 bytes big-endian).
pub(crate) fn f64_bits_to_x80(bits: u64) -> [u8; 10] {
    let sign = ((bits >> 63) as u16) << 15;
    let exp11 = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & 0x000F_FFFF_FFFF_FFFF;
    let (exp15, sig) = if exp11 == 0x7FF {
        (0x7FFF_u16, if frac != 0 { 0xC000_0000_0000_0000 | (frac << 11) } else { 0x8000_0000_0000_0000 })
    } else if exp11 == 0 {
        if frac == 0 {
            (0u16, 0u64)
        } else {
            let mut m = frac << 12;
            let mut e = 16383 - 1022;
            while (m >> 63) == 0 { m <<= 1; e -= 1; }
            (e as u16, m)
        }
    } else {
        ((exp11 - 1023 + 16383) as u16, 0x8000_0000_0000_0000 | (frac << 11))
    };
    let mut out = [0u8; 10];
    out[0..2].copy_from_slice(&(sign | exp15).to_be_bytes());
    out[2..10].copy_from_slice(&sig.to_be_bytes());
    out
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn math_call(&mut self, f: MathFn, cpu: &mut Cpu) -> Result<(), String> {
        use MathFn::*;
        let (x, y) = (f64::from_bits(cpu.fpr[1]), f64::from_bits(cpu.fpr[2]));
        let store = |m: &mut Self, at: u32, v: u32, who: &str| m.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: result pointer {at:#x} leaves guest RAM"));
        // the decimal conversions (crate::decimal): pointers in r3 (and r6 for num2dec's `decimal *`: the double in f1 reserves r4 and r5)
        match f {
            Dec2num => {
                let rec = self.get_bytes(cpu.gpr[3], crate::decimal::DECIMAL_SIZE as u32, "dec2num")?;
                cpu.fpr[1] = crate::decimal::dec2num_bits(&rec);
                return Ok(());
            }
            Nan => { let t = self.cstr(cpu.gpr[3], "nan")?; cpu.fpr[1] = crate::decimal::nan_bits(&t); return Ok(()); }
            Num2dec => {
                let form = self.get_bytes(cpu.gpr[3], 4, "num2dec")?;
                let rec = crate::decimal::num2dec(form[0], i16::from_be_bytes([form[2], form[3]]), x)?;
                self.put_bytes(cpu.gpr[6], &rec, "num2dec")?;
                return Ok(());
            }
            X80tod => {
                let b = self.get_bytes(cpu.gpr[3], 10, "x80tod")?;
                cpu.fpr[1] = x80_to_f64_bits(&b);
                return Ok(());
            }
            Dtox80 => {
                let b = self.get_bytes(cpu.gpr[3], 8, "dtox80")?;
                let bits = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                let x80 = f64_bits_to_x80(bits);
                self.put_bytes(cpu.gpr[4], &x80, "dtox80")?;
                return Ok(());
            }
            _ => {}
        }
        let r = match f {
            Acos => libm::acos(x), Asin => libm::asin(x), Atan => libm::atan(x), Atan2 => libm::atan2(x, y), Ceil => libm::ceil(x), Cos => libm::cos(x),
            Cosh => libm::cosh(x), Exp => libm::exp(x), Fabs => libm::fabs(x), Floor => libm::floor(x), Fmod => libm::fmod(x, y), Hypot => libm::hypot(x, y),
            Log => libm::log(x), Log2 => libm::log2(x), Log10 => libm::log10(x), Fmin => libm::fmin(x, y), Fmax => libm::fmax(x, y), Pow => libm::pow(x, y), Rint => libm::rint(x), Round => libm::round(x), Sin => libm::sin(x),
            Sinh => libm::sinh(x), Sqrt => libm::sqrt(x), Tan => libm::tan(x), Tanh => libm::tanh(x),
            Dec2num | Num2dec | Nan | X80tod | Dtox80 => unreachable!("handled above"),
            // FixMath.h (InterfaceLib on PowerPC): double_t is IEEE-754 double in f1.
            Fix2X => f64::from(cpu.gpr[3] as i32) / 65536.0,
            Frac2X => f64::from(cpu.gpr[3] as i32) / 1_073_741_824.0,
            X2Fix => {
                let v = libm::round(x * 65536.0).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
                cpu.gpr[3] = v as u32;
                return Ok(());
            }
            X2Frac => {
                let v = libm::round(x * 1_073_741_824.0).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
                cpu.gpr[3] = v as u32;
                return Ok(());
            }
            Ldexp => libm::ldexp(x, arg(self, cpu, 2)? as i32),
            Frexp => { let (m, e) = libm::frexp(x); let p = arg(self, cpu, 2)?; store(self, p, e as u32, "frexp")?; m }
            Modf => {
                let (frac, int) = libm::modf(x);
                let p = arg(self, cpu, 2)?;
                self.mac.mem.write_u32(p, (int.to_bits() >> 32) as u32).and_then(|_| self.mac.mem.write_u32(p + 4, int.to_bits() as u32)).map_err(|_| format!("modf: result pointer {p:#x} leaves guest RAM"))?;
                frac
            }
        };
        cpu.fpr[1] = r.to_bits();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x80_and_f64_round_trip_normals_zero_inf_and_nan() {
        for v in [0.0f64, -0.0, 1.0, -42.5, 3.141592653589793, 1e100, -1e-100, f64::INFINITY, f64::NEG_INFINITY] {
            let x80 = f64_bits_to_x80(v.to_bits());
            let back = f64::from_bits(x80_to_f64_bits(&x80));
            assert_eq!(back.to_bits(), v.to_bits(), "round-trip failed for {v}");
        }
        // NEGATIVE CONTROL: NaN stays NaN (never collapses to 0 or finite).
        let nan_x80 = f64_bits_to_x80(f64::NAN.to_bits());
        assert!(f64::from_bits(x80_to_f64_bits(&nan_x80)).is_nan());
    }

    #[test]
    fn fix_and_frac_conversions_round_trip_in_f1_and_r3() {
        let mut m = MacPpc::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
        let mut cpu = Cpu::new(0x50000, 0x70000);
        cpu.gpr[3] = 0x0001_8000; // Fixed 1.5
        m.math_call(MathFn::Fix2X, &mut cpu).unwrap();
        assert_eq!(f64::from_bits(cpu.fpr[1]), 1.5);
        m.math_call(MathFn::X2Fix, &mut cpu).unwrap();
        assert_eq!(cpu.gpr[3], 0x0001_8000);
        cpu.gpr[3] = 0x2000_0000; // Fract 0.5
        m.math_call(MathFn::Frac2X, &mut cpu).unwrap();
        assert_eq!(f64::from_bits(cpu.fpr[1]), 0.5);
        m.math_call(MathFn::X2Frac, &mut cpu).unwrap();
        assert_eq!(cpu.gpr[3], 0x2000_0000);
        // NEGATIVE CONTROL: overflow saturates to i32::MAX instead of wrapping
        cpu.fpr[1] = 1e12f64.to_bits();
        m.math_call(MathFn::X2Fix, &mut cpu).unwrap();
        assert_eq!(cpu.gpr[3] as i32, i32::MAX);
    }
}
