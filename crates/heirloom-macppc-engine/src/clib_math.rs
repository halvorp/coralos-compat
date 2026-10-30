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
pub enum MathFn { Acos, Asin, Atan, Atan2, Ceil, Cos, Cosh, Exp, Fabs, Floor, Fmod, Frexp, Hypot, Ldexp, Log, Log10, Modf, Pow, Rint, Round, Sin, Sinh, Sqrt, Tan, Tanh, Dec2num, Num2dec, Nan }

impl MathFn {
    pub fn by_name(name: &str) -> Option<MathFn> {
        use MathFn::*;
        Some(match name {
            "acos" => Acos, "asin" => Asin, "atan" => Atan, "atan2" => Atan2, "ceil" => Ceil, "cos" => Cos, "cosh" => Cosh, "exp" => Exp, "fabs" => Fabs,
            "floor" => Floor, "fmod" => Fmod, "frexp" => Frexp, "hypot" => Hypot, "ldexp" => Ldexp, "log" => Log, "log10" => Log10, "modf" => Modf, "pow" => Pow,
            "rint" => Rint, "round" => Round, "sin" => Sin, "sinh" => Sinh, "sqrt" => Sqrt, "tan" => Tan, "tanh" => Tanh, "dec2num" => Dec2num, "num2dec" => Num2dec, "nan" => Nan,
            _ => return None,
        })
    }
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
            _ => {}
        }
        let r = match f {
            Acos => libm::acos(x), Asin => libm::asin(x), Atan => libm::atan(x), Atan2 => libm::atan2(x, y), Ceil => libm::ceil(x), Cos => libm::cos(x),
            Cosh => libm::cosh(x), Exp => libm::exp(x), Fabs => libm::fabs(x), Floor => libm::floor(x), Fmod => libm::fmod(x, y), Hypot => libm::hypot(x, y),
            Log => libm::log(x), Log10 => libm::log10(x), Pow => libm::pow(x, y), Rint => libm::rint(x), Round => libm::round(x), Sin => libm::sin(x),
            Sinh => libm::sinh(x), Sqrt => libm::sqrt(x), Tan => libm::tan(x), Tanh => libm::tanh(x),
            Dec2num | Num2dec | Nan => unreachable!("handled above"),
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
