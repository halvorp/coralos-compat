//! IEEE 754 binary32/binary64 arithmetic with the ARM ARM's FPSCR semantics: all four rounding modes,
//! flush-to-zero (FZ, with IDC on flushed inputs), default NaN (DN), ARM NaN propagation (first signalling
//! NaN, then first quiet NaN) and tininess detected before rounding. Results are exact (no host floats):
//! significands are carried in `u128` with every discarded bit folded into a sticky bit.
//!
//! Written for this crate, following the FPRound / FPUnpack / FPProcessNaNs pseudocode. rax's VFP layer
//! delegates this arithmetic to its SMIR IR interpreter (src/smir), a whole subsystem this port does not
//! bring in, so this file is not a port.

/// Cumulative exception flags, in FPSCR's bit positions.
pub(crate) const IOC: u32 = 1;
pub(crate) const DZC: u32 = 1 << 1;
pub(crate) const OFC: u32 = 1 << 2;
pub(crate) const UFC: u32 = 1 << 3;
pub(crate) const IXC: u32 = 1 << 4;
pub(crate) const IDC: u32 = 1 << 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fmt {
    ebits: u32,
    fbits: u32,
}

pub(crate) const F32: Fmt = Fmt { ebits: 8, fbits: 23 };
pub(crate) const F64: Fmt = Fmt { ebits: 11, fbits: 52 };

impl Fmt {
    fn bias(self) -> i32 {
        (1 << (self.ebits - 1)) - 1
    }
    fn exp_max(self) -> u64 {
        (1 << self.ebits) - 1
    }
    fn sign_bit(self) -> u64 {
        1 << (self.ebits + self.fbits)
    }
    fn sign(self, x: u64) -> bool {
        x & self.sign_bit() != 0
    }
    fn exp(self, x: u64) -> u64 {
        (x >> self.fbits) & self.exp_max()
    }
    fn frac(self, x: u64) -> u64 {
        x & ((1 << self.fbits) - 1)
    }
    fn quiet_bit(self) -> u64 {
        1 << (self.fbits - 1)
    }
    pub(crate) fn is_nan(self, x: u64) -> bool {
        self.exp(x) == self.exp_max() && self.frac(x) != 0
    }
    fn is_snan(self, x: u64) -> bool {
        self.is_nan(x) && x & self.quiet_bit() == 0
    }
    pub(crate) fn default_nan(self) -> u64 {
        self.exp_max() << self.fbits | self.quiet_bit()
    }
    fn inf(self, sign: bool) -> u64 {
        self.zero(sign) | self.exp_max() << self.fbits
    }
    fn zero(self, sign: bool) -> u64 {
        if sign {
            self.sign_bit()
        } else {
            0
        }
    }
    fn max_finite(self, sign: bool) -> u64 {
        self.zero(sign) | (self.exp_max() - 1) << self.fbits | ((1 << self.fbits) - 1)
    }
}

/// A finite non-zero value: (-1)^sign * sig * 2^exp.
#[derive(Clone, Copy, Debug)]
struct Num {
    sign: bool,
    exp: i32,
    sig: u128,
}

#[derive(Clone, Copy, Debug)]
enum Val {
    Zero(bool),
    Inf(bool),
    Num(Num),
}

/// The FPSCR controls an operation reads, and the flags it raised.
pub(crate) struct Env {
    fpscr: u32,
    pub(crate) flags: u32,
}

/// Rounding modes in FPSCR.RMode order.
const RN: u32 = 0;
const RP: u32 = 1;
const RM: u32 = 2;
const RZ: u32 = 3;

impl Env {
    pub(crate) fn new(fpscr: u32) -> Self {
        Env { fpscr, flags: 0 }
    }

    fn rmode(&self) -> u32 {
        (self.fpscr >> 22) & 3
    }

    fn fz(&self) -> bool {
        self.fpscr & (1 << 24) != 0
    }

    fn dn(&self) -> bool {
        self.fpscr & (1 << 25) != 0
    }

    fn nan_result(&self, fmt: Fmt, x: u64) -> u64 {
        if self.dn() {
            fmt.default_nan()
        } else {
            x
        }
    }

    /// FPProcessNaN for one operand: `Some(result)` when it is a NaN.
    fn process_nan(&mut self, fmt: Fmt, x: u64) -> Option<u64> {
        if fmt.is_snan(x) {
            self.flags |= IOC;
            Some(self.nan_result(fmt, x | fmt.quiet_bit()))
        } else if fmt.is_nan(x) {
            Some(self.nan_result(fmt, x))
        } else {
            None
        }
    }

    /// FPProcessNaNs: signalling NaNs first (op1 before op2), then quiet NaNs.
    fn process_nans(&mut self, fmt: Fmt, a: u64, b: u64) -> Option<u64> {
        if fmt.is_snan(a) {
            self.process_nan(fmt, a)
        } else if fmt.is_snan(b) {
            self.process_nan(fmt, b)
        } else {
            self.process_nan(fmt, a).or_else(|| self.process_nan(fmt, b))
        }
    }

    fn invalid(&mut self, fmt: Fmt) -> u64 {
        self.flags |= IOC;
        fmt.default_nan()
    }

    /// FPUnpack for a non-NaN: a denormal input flushes to zero (with IDC) when FZ is set.
    fn unpack(&mut self, fmt: Fmt, x: u64) -> Val {
        let sign = fmt.sign(x);
        let (e, f) = (fmt.exp(x), fmt.frac(x));
        if e == fmt.exp_max() {
            return Val::Inf(sign);
        }
        if e == 0 {
            if f == 0 {
                return Val::Zero(sign);
            }
            if self.fz() {
                self.flags |= IDC;
                return Val::Zero(sign);
            }
            return Val::Num(Num { sign, exp: 1 - fmt.bias() - fmt.fbits as i32, sig: f as u128 });
        }
        Val::Num(Num { sign, exp: e as i32 - fmt.bias() - fmt.fbits as i32, sig: (f | 1 << fmt.fbits) as u128 })
    }

    /// FPRound: round `n` (exact, or with a sticky low bit far below the rounding point) to `fmt`.
    fn round(&mut self, fmt: Fmt, n: Num) -> u64 {
        let fb = fmt.fbits as i32;
        let emin = 1 - fmt.bias();
        let msb = 127 - n.sig.leading_zeros() as i32;
        let top = n.exp + msb; // exponent of the leading bit
        let tiny = top < emin;
        if tiny && self.fz() {
            self.flags |= UFC;
            return fmt.zero(n.sign);
        }
        let mut lsb_exp = (top - fb).max(emin - fb);
        let shift = lsb_exp - n.exp;
        let (mut kept, guard, sticky) = if shift <= 0 {
            (n.sig << -shift, false, false)
        } else {
            let kept = if shift >= 128 { 0 } else { n.sig >> shift };
            let below = shift - 1; // bits under the guard bit
            let guard = below < 128 && (n.sig >> below) & 1 != 0;
            let sticky = if below >= 128 { n.sig != 0 } else { n.sig & ((1u128 << below) - 1) != 0 };
            (kept, guard, sticky)
        };
        let inexact = guard || sticky;
        let up = match self.rmode() {
            RN => guard && (sticky || kept & 1 != 0),
            RP => !n.sign && inexact,
            RM => n.sign && inexact,
            _ => false,
        };
        kept += up as u128;
        if kept == 1u128 << (fb + 1) {
            kept >>= 1;
            lsb_exp += 1;
        }
        if tiny && inexact {
            self.flags |= UFC;
        }
        let biased = if kept >> fb != 0 { (lsb_exp + fb + fmt.bias()) as i64 } else { 0 };
        if biased >= fmt.exp_max() as i64 {
            self.flags |= OFC | IXC;
            return match (self.rmode(), n.sign) {
                (RN, s) => fmt.inf(s),
                (RP, false) | (RM, true) => fmt.inf(n.sign),
                (_, s) => fmt.max_finite(s),
            };
        }
        if inexact {
            self.flags |= IXC;
        }
        fmt.zero(n.sign) | (biased as u64) << fmt.fbits | (kept as u64 & ((1 << fmt.fbits) - 1))
    }

    pub(crate) fn add(&mut self, fmt: Fmt, a: u64, b: u64) -> u64 {
        // FPUnpack (which raises IDC for a flushed denormal) runs on both operands before NaN handling.
        let (x, y) = (self.unpack(fmt, a), self.unpack(fmt, b));
        if let Some(nan) = self.process_nans(fmt, a, b) {
            return nan;
        }
        match (x, y) {
            (Val::Inf(x), Val::Inf(y)) if x != y => self.invalid(fmt),
            (Val::Inf(s), _) | (_, Val::Inf(s)) => fmt.inf(s),
            (Val::Zero(x), Val::Zero(y)) => fmt.zero(if x == y { x } else { self.rmode() == RM }),
            (Val::Zero(_), Val::Num(n)) | (Val::Num(n), Val::Zero(_)) => self.round(fmt, n),
            (Val::Num(x), Val::Num(y)) => {
                let (big, small) = if x.exp >= y.exp { (x, y) } else { (y, x) };
                let diff = big.exp - small.exp;
                // Beyond 64 bits of separation the smaller operand lies wholly under the larger one's
                // rounding point; a single sticky bit stands in for it.
                let (bs, ss, exp) =
                    if diff <= 64 { (big.sig << diff, small.sig, small.exp) } else { (big.sig << 64, 1, big.exp - 64) };
                if big.sign == small.sign {
                    self.round(fmt, Num { sign: big.sign, exp, sig: bs + ss })
                } else if bs > ss {
                    self.round(fmt, Num { sign: big.sign, exp, sig: bs - ss })
                } else if ss > bs {
                    self.round(fmt, Num { sign: small.sign, exp, sig: ss - bs })
                } else {
                    fmt.zero(self.rmode() == RM)
                }
            }
        }
    }

    pub(crate) fn sub(&mut self, fmt: Fmt, a: u64, b: u64) -> u64 {
        if fmt.is_nan(a) || fmt.is_nan(b) {
            self.unpack(fmt, a);
            self.unpack(fmt, b);
            return self.process_nans(fmt, a, b).unwrap_or(0);
        }
        self.add(fmt, a, b ^ fmt.sign_bit())
    }

    pub(crate) fn mul(&mut self, fmt: Fmt, a: u64, b: u64) -> u64 {
        let (x, y) = (self.unpack(fmt, a), self.unpack(fmt, b));
        if let Some(nan) = self.process_nans(fmt, a, b) {
            return nan;
        }
        let sign = fmt.sign(a) != fmt.sign(b);
        match (x, y) {
            (Val::Inf(_), Val::Zero(_)) | (Val::Zero(_), Val::Inf(_)) => self.invalid(fmt),
            (Val::Inf(_), _) | (_, Val::Inf(_)) => fmt.inf(sign),
            (Val::Zero(_), _) | (_, Val::Zero(_)) => fmt.zero(sign),
            (Val::Num(x), Val::Num(y)) => self.round(fmt, Num { sign, exp: x.exp + y.exp, sig: x.sig * y.sig }),
        }
    }

    pub(crate) fn div(&mut self, fmt: Fmt, a: u64, b: u64) -> u64 {
        let (x, y) = (self.unpack(fmt, a), self.unpack(fmt, b));
        if let Some(nan) = self.process_nans(fmt, a, b) {
            return nan;
        }
        let sign = fmt.sign(a) != fmt.sign(b);
        match (x, y) {
            (Val::Inf(_), Val::Inf(_)) | (Val::Zero(_), Val::Zero(_)) => self.invalid(fmt),
            (Val::Inf(_), _) => fmt.inf(sign),
            (_, Val::Inf(_)) | (Val::Zero(_), _) => fmt.zero(sign),
            (Val::Num(_), Val::Zero(_)) => {
                self.flags |= DZC;
                fmt.inf(sign)
            }
            (Val::Num(x), Val::Num(y)) => {
                let up = 120 - (127 - x.sig.leading_zeros() as i32);
                let dividend = x.sig << up;
                let (q, r) = (dividend / y.sig, dividend % y.sig);
                self.round(fmt, Num { sign, exp: x.exp - up - y.exp - 1, sig: q << 1 | (r != 0) as u128 })
            }
        }
    }

    pub(crate) fn sqrt(&mut self, fmt: Fmt, a: u64) -> u64 {
        let x = self.unpack(fmt, a);
        if let Some(nan) = self.process_nan(fmt, a) {
            return nan;
        }
        match x {
            Val::Zero(s) => fmt.zero(s),
            Val::Inf(false) => fmt.inf(false),
            Val::Inf(true) | Val::Num(Num { sign: true, .. }) => self.invalid(fmt),
            Val::Num(Num { exp, sig, .. }) => {
                let (mut exp, mut sig) = (exp, sig);
                if exp & 1 != 0 {
                    sig <<= 1;
                    exp -= 1;
                }
                let up = (120 - (127 - sig.leading_zeros() as i32)) & !1;
                sig <<= up;
                exp -= up;
                let (root, rem) = isqrt(sig);
                self.round(fmt, Num { sign: false, exp: exp / 2 - 1, sig: root << 1 | (rem != 0) as u128 })
            }
        }
    }

    /// VCMP/VCMPE: the FPSCR NZCV nibble (in bits 31:28). `signal_quiet` is VCMPE.
    pub(crate) fn compare(&mut self, fmt: Fmt, a: u64, b: u64, signal_quiet: bool) -> u32 {
        if fmt.is_nan(a) || fmt.is_nan(b) {
            self.unpack(fmt, a);
            self.unpack(fmt, b);
            if signal_quiet || fmt.is_snan(a) || fmt.is_snan(b) {
                self.flags |= IOC;
            }
            return 0b0011 << 28;
        }
        let key = |env: &mut Self, x: u64| -> i128 {
            match env.unpack(fmt, x) {
                Val::Zero(_) => 0,
                Val::Inf(s) => {
                    if s {
                        i128::MIN
                    } else {
                        i128::MAX
                    }
                }
                Val::Num(_) => {
                    let mag = (x & !fmt.sign_bit()) as i128;
                    if fmt.sign(x) {
                        -mag
                    } else {
                        mag
                    }
                }
            }
        };
        let (ka, kb) = (key(self, a), key(self, b));
        match ka.cmp(&kb) {
            core::cmp::Ordering::Equal => 0b0110 << 28,
            core::cmp::Ordering::Less => 0b1000 << 28,
            core::cmp::Ordering::Greater => 0b0010 << 28,
        }
    }

    /// FPToFixed with fbits = 0: float to 32-bit integer, saturating with IOC. `round_zero` is VCVT's
    /// round-towards-zero; otherwise FPSCR.RMode applies (VCVTR).
    pub(crate) fn float_to_int(&mut self, fmt: Fmt, a: u64, signed: bool, round_zero: bool) -> u32 {
        if fmt.is_nan(a) {
            self.flags |= IOC;
            return 0;
        }
        let (neg, mag, inexact) = match self.unpack(fmt, a) {
            Val::Zero(_) => return 0,
            Val::Inf(s) => (s, u128::MAX, false),
            Val::Num(n) => {
                if n.exp >= 0 {
                    (n.sign, if n.exp > 64 { u128::MAX } else { n.sig << n.exp }, false)
                } else {
                    let shift = (-n.exp) as u32;
                    let kept = if shift >= 128 { 0 } else { n.sig >> shift };
                    let below = shift - 1;
                    let guard = below < 128 && (n.sig >> below) & 1 != 0;
                    let sticky = if below >= 128 { n.sig != 0 } else { n.sig & ((1u128 << below) - 1) != 0 };
                    let inexact = guard || sticky;
                    let up = match if round_zero { RZ } else { self.rmode() } {
                        RN => guard && (sticky || kept & 1 != 0),
                        RP => !n.sign && inexact,
                        RM => n.sign && inexact,
                        _ => false,
                    };
                    (n.sign, kept + up as u128, inexact)
                }
            }
        };
        let (min, max): (i128, i128) = if signed { (i32::MIN as i128, i32::MAX as i128) } else { (0, u32::MAX as i128) };
        let value = if mag > (1u128 << 64) { if neg { i128::MIN } else { i128::MAX } } else if neg { -(mag as i128) } else { mag as i128 };
        if value < min || value > max {
            self.flags |= IOC;
            return value.clamp(min, max) as u32;
        }
        if inexact {
            self.flags |= IXC;
        }
        value as u32
    }

    /// Integer to float, rounded per FPSCR.RMode.
    pub(crate) fn int_to_float(&mut self, fmt: Fmt, v: u32, signed: bool) -> u64 {
        let (sign, mag) = if signed && (v as i32) < 0 { (true, (v as i32).unsigned_abs()) } else { (false, v) };
        if mag == 0 {
            return 0;
        }
        self.round(fmt, Num { sign, exp: 0, sig: mag as u128 })
    }

    /// FPConvert between binary32 and binary64.
    pub(crate) fn convert(&mut self, from: Fmt, to: Fmt, a: u64) -> u64 {
        if from.is_nan(a) {
            if from.is_snan(a) {
                self.flags |= IOC;
            }
            if self.dn() {
                return to.default_nan();
            }
            let frac = from.frac(a) | from.quiet_bit();
            let frac = if to.fbits > from.fbits { frac << (to.fbits - from.fbits) } else { frac >> (from.fbits - to.fbits) };
            return to.zero(from.sign(a)) | to.exp_max() << to.fbits | frac;
        }
        match self.unpack(from, a) {
            Val::Zero(s) => to.zero(s),
            Val::Inf(s) => to.inf(s),
            Val::Num(n) => self.round(to, n),
        }
    }
}

/// Integer square root with remainder: (floor(sqrt(n)), n - root^2).
fn isqrt(mut n: u128) -> (u128, u128) {
    let mut root = 0u128;
    let mut bit = 1u128 << 126;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if n >= root + bit {
            n -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    (root, n)
}
