//! MathLib: the C math functions (`<math.h>`), natively over the host's `libm` (a pure-Rust port of musl's libm, so results do not depend on the
//! host CPU). Arguments arrive in f1, f2 and the result goes back in f1 (Mac OS Runtime Architectures); `ldexp`/`frexp`/`modf` also use r3.
//! Results are IEEE-754 doubles as the C standard defines them; MathLib's classic `-0` / NaN payload conventions are not reproduced.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFn { Acos, Asin, Atan, Atan2, Ceil, Cos, Cosh, Exp, Fabs, Floor, Fmod, Frexp, Hypot, Ldexp, Log, Log10, Modf, Pow, Rint, Round, Sin, Sinh, Sqrt, Tan, Tanh }

impl MathFn {
    pub fn by_name(name: &str) -> Option<MathFn> {
        use MathFn::*;
        Some(match name {
            "acos" => Acos, "asin" => Asin, "atan" => Atan, "atan2" => Atan2, "ceil" => Ceil, "cos" => Cos, "cosh" => Cosh, "exp" => Exp, "fabs" => Fabs,
            "floor" => Floor, "fmod" => Fmod, "frexp" => Frexp, "hypot" => Hypot, "ldexp" => Ldexp, "log" => Log, "log10" => Log10, "modf" => Modf, "pow" => Pow,
            "rint" => Rint, "round" => Round, "sin" => Sin, "sinh" => Sinh, "sqrt" => Sqrt, "tan" => Tan, "tanh" => Tanh,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn math_call(&mut self, f: MathFn, cpu: &mut Cpu) -> Result<(), String> {
        use MathFn::*;
        let (x, y) = (f64::from_bits(cpu.fpr[1]), f64::from_bits(cpu.fpr[2]));
        let store = |m: &mut Self, at: u32, v: u32, who: &str| m.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: result pointer {at:#x} leaves guest RAM"));
        let r = match f {
            Acos => libm::acos(x), Asin => libm::asin(x), Atan => libm::atan(x), Atan2 => libm::atan2(x, y), Ceil => libm::ceil(x), Cos => libm::cos(x),
            Cosh => libm::cosh(x), Exp => libm::exp(x), Fabs => libm::fabs(x), Floor => libm::floor(x), Fmod => libm::fmod(x, y), Hypot => libm::hypot(x, y),
            Log => libm::log(x), Log10 => libm::log10(x), Pow => libm::pow(x, y), Rint => libm::rint(x), Round => libm::round(x), Sin => libm::sin(x),
            Sinh => libm::sinh(x), Sqrt => libm::sqrt(x), Tan => libm::tan(x), Tanh => libm::tanh(x),
            Ldexp => libm::ldexp(x, arg(self, cpu, 0)? as i32),
            Frexp => { let (m, e) = libm::frexp(x); let p = arg(self, cpu, 0)?; store(self, p, e as u32, "frexp")?; m }
            Modf => {
                let (frac, int) = libm::modf(x);
                let p = arg(self, cpu, 0)?;
                self.mac.mem.write_u32(p, (int.to_bits() >> 32) as u32).and_then(|_| self.mac.mem.write_u32(p + 4, int.to_bits() as u32)).map_err(|_| format!("modf: result pointer {p:#x} leaves guest RAM"))?;
                frac
            }
        };
        cpu.fpr[1] = r.to_bits();
        Ok(())
    }
}
