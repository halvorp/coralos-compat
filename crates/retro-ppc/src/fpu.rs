//! Floating-point arithmetic, conversion, compare and FPSCR instructions: opcodes 59 (single) and 63 (double)
//! (PEM 4.2.? "Floating-Point Processor Instructions"). See the lib.rs notes for the deliberate deviations.
use crate::cpu::Cpu;
use crate::exec::R;
use crate::fields::W;
use crate::fpscr::{self as fs, classify, copy_to_cr1, raise, set_fprf};

/// The QNaN the hardware produces for an invalid operation (PEM 3.3.6.? "default NaN"): sign 0, quiet bit set.
pub const DEFAULT_NAN: u64 = 0x7FF8_0000_0000_0000;

#[derive(Clone, Copy)]
enum Op { Add, Sub, Mul, Div, Fma { neg: bool, sub: bool }, Sqrt, Res, Rsqrte }

fn f(cpu: &Cpu, n: usize) -> f64 { f64::from_bits(cpu.fpr[n]) }
fn is_snan(x: f64) -> bool { x.is_nan() && x.to_bits() & (1 << 51) == 0 }
fn quiet(x: f64) -> f64 { f64::from_bits(x.to_bits() | 1 << 51) }

/// Write an FP result: FPRF from its class, then CR1 if Rc.
fn put(cpu: &mut Cpu, w: W, d: usize, bits: u64, single: bool) {
    cpu.fpr[d] = bits;
    set_fprf(cpu, classify(f64::from_bits(bits), single));
    if w.rc() { copy_to_cr1(cpu); }
}

pub(crate) fn exec_59(cpu: &mut Cpu, w: W) -> R {
    let (op, single) = match w.xo5() {
        18 => (Op::Div, true), 20 => (Op::Sub, true), 21 => (Op::Add, true), 22 => (Op::Sqrt, true),
        24 => (Op::Res, true), 25 => (Op::Mul, true),
        28 => (Op::Fma { neg: false, sub: true }, true), 29 => (Op::Fma { neg: false, sub: false }, true),
        30 => (Op::Fma { neg: true, sub: true }, true), 31 => (Op::Fma { neg: true, sub: false }, true),
        _ => return Err(cpu.unimpl(w, "opcode 59 extended (reserved)")),
    };
    arith(cpu, w, op, single)
}

pub(crate) fn exec_63(cpu: &mut Cpu, w: W) -> R {
    match w.xo() {
        0 | 32 => return fcmp(cpu, w, w.xo() == 32),
        12 => return frsp(cpu, w),
        14 | 15 => return fctiw(cpu, w, w.xo() == 15),
        40 | 72 | 136 | 264 => { // fneg, fmr, fnabs, fabs: sign/bit moves, no FPSCR effect besides CR1
            let b = cpu.fpr[w.rb()];
            cpu.fpr[w.rt()] = match w.xo() { 40 => b ^ 1 << 63, 72 => b, 136 => b | 1 << 63, _ => b & !(1 << 63) };
            if w.rc() { copy_to_cr1(cpu); }
            return Ok(());
        }
        583 => { // mffs: FPSCR in the low word; the high word is undefined (we set the pattern 0xFFF80000)
            cpu.fpr[w.rt()] = 0xFFF8_0000_0000_0000 | cpu.fpscr as u64;
            if w.rc() { copy_to_cr1(cpu); }
            return Ok(());
        }
        711 => return mtfsf(cpu, w),
        134 => return mtfsfi(cpu, w),
        38 | 70 => { // mtfsb1 / mtfsb0
            let bit = 31 - w.rt() as u32;
            if w.xo() == 38 { raise_bit(cpu, 1 << bit) } else { cpu.fpscr &= !(1 << bit); fs::fix(cpu) }
            if w.rc() { copy_to_cr1(cpu); }
            return Ok(());
        }
        64 => { // mcrfs: copy FPSCR field BFA to CR field BF, then clear the sticky bits that were in it
            let sh = 28 - 4 * w.bfa();
            let v = cpu.fpscr >> sh & 0xF;
            cpu.set_cr_field(w.bf(), v);
            cpu.fpscr &= !(fs::STICKY & 0xF << sh);
            fs::fix(cpu);
            return Ok(());
        }
        _ => {}
    }
    let (op, single) = match w.xo5() {
        18 => (Op::Div, false), 20 => (Op::Sub, false), 21 => (Op::Add, false), 22 => (Op::Sqrt, false),
        23 => return fsel(cpu, w),
        25 => (Op::Mul, false), 26 => (Op::Rsqrte, false),
        28 => (Op::Fma { neg: false, sub: true }, false), 29 => (Op::Fma { neg: false, sub: false }, false),
        30 => (Op::Fma { neg: true, sub: true }, false), 31 => (Op::Fma { neg: true, sub: false }, false),
        _ => return Err(cpu.unimpl(w, "opcode 63 extended (reserved)")),
    };
    arith(cpu, w, op, single)
}

/// Setting a sticky flag by hand (mtfsb1): FX goes up if the flag was down.
fn raise_bit(cpu: &mut Cpu, m: u32) {
    if m & fs::STICKY != 0 && cpu.fpscr & m == 0 { cpu.fpscr |= fs::FX; }
    cpu.fpscr |= m;
    // FEX and VX are derived, never written directly.
    fs::fix(cpu);
}

fn need_rne(cpu: &Cpu, w: W) -> R {
    if cpu.fpscr & fs::RN != 0 {
        Err(cpu.unimpl(w, "FP arithmetic under a directed rounding mode (FPSCR[RN] != 0)"))
    } else { Ok(()) }
}

/// Rc=1 updates CR1 from FPSCR whether or not the result was written (a suppressed result under an enabled
/// exception still records the exception summary: PEM, "Recording" of FP instructions).
fn arith(cpu: &mut Cpu, w: W, op: Op, single: bool) -> R {
    let r = arith_inner(cpu, w, op, single);
    if r.is_ok() && w.rc() { copy_to_cr1(cpu); }
    r
}

fn arith_inner(cpu: &mut Cpu, w: W, op: Op, single: bool) -> R {
    need_rne(cpu, w)?;
    let (a, b, c) = (f(cpu, w.ra()), f(cpu, w.rb()), f(cpu, w.frc()));
    let used: &[f64] = match op {
        Op::Add | Op::Sub | Op::Div => &[a, b],
        Op::Mul => &[a, c],
        Op::Fma { .. } => &[a, b, c],
        Op::Sqrt | Op::Res | Op::Rsqrte => &[b],
    };
    let d = w.rt();
    let mut flags = 0u32;
    if used.iter().any(|x| is_snan(*x)) { flags |= fs::VXSNAN; }
    // PEM fmul/fmadd: infinity x zero is VXIMZ whatever the addend is, so it is raised even when frB is a NaN and
    // the NaN (not the default QNaN) is what lands in frD.
    let inf0 = |x: f64, y: f64| (x.is_infinite() && y == 0.0) || (y.is_infinite() && x == 0.0);
    if matches!(op, Op::Mul | Op::Fma { .. }) && inf0(a, c) { flags |= fs::VXIMZ; }
    // NaN operands propagate, first in the order frA, frB, frC (PEM 3.3.6.? "NaN results").
    if let Some(n) = used.iter().find(|x| x.is_nan()) {
        if flags != 0 { raise(cpu, flags); if cpu.fpscr & fs::VE != 0 { return Ok(()); } }
        let q = quiet(*n);
        let q = if single { (q as f32) as f64 } else { q };
        put(cpu, w, d, q.to_bits(), single);
        return Ok(());
    }
    match op {
        Op::Add if a.is_infinite() && b.is_infinite() && a.is_sign_negative() != b.is_sign_negative() => flags |= fs::VXISI,
        Op::Sub if a.is_infinite() && b.is_infinite() && a.is_sign_negative() == b.is_sign_negative() => flags |= fs::VXISI,
        Op::Div if a.is_infinite() && b.is_infinite() => flags |= fs::VXIDI,
        Op::Div if a == 0.0 && b == 0.0 => flags |= fs::VXZDZ,
        Op::Fma { sub, .. } if !inf0(a, c) && (a.is_infinite() || c.is_infinite()) && b.is_infinite() => {
            let prod_neg = a.is_sign_negative() != c.is_sign_negative();
            let add_neg = b.is_sign_negative() != sub;
            if prod_neg != add_neg { flags |= fs::VXISI; }
        }
        Op::Sqrt | Op::Rsqrte if b < 0.0 => flags |= fs::VXSQRT, // also -inf; -0 is fine
        _ => {}
    }
    if flags != 0 {
        raise(cpu, flags);
        if cpu.fpscr & fs::VE != 0 { return Ok(()); }
        put(cpu, w, d, DEFAULT_NAN, single);
        return Ok(());
    }
    let mut zx = false;
    let mut r = match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * c,
        Op::Div => { zx = b == 0.0 && a != 0.0 && a.is_finite(); a / b }
        Op::Fma { neg, sub } => {
            let b = if sub { -b } else { b };
            let m = if single { fma_single(a, c, b) } else { libm::fma(a, c, b) };
            if neg { -m } else { m }
        }
        Op::Sqrt => libm::sqrt(b),
        Op::Res => { zx = b == 0.0; 1.0 / b }
        Op::Rsqrte => { zx = b == 0.0; 1.0 / libm::sqrt(b) }
    };
    if zx {
        raise(cpu, fs::ZX);
        // PEM 3.3.6.? zero-divide exception, enabled (ZE=1): the target FPR is left unchanged.
        if cpu.fpscr & fs::ZE != 0 { return Ok(()); }
    }
    if single || matches!(op, Op::Res) { r = (r as f32) as f64; }
    if r.is_infinite() && !zx && used.iter().all(|x| x.is_finite()) { raise(cpu, fs::OX); }
    put(cpu, w, d, r.to_bits(), single || matches!(op, Op::Res));
    Ok(())
}

/// a*c+b rounded ONCE to single precision, returned widened to f64. Rounding the f64 result to f32 would round twice
/// (and can land one ulp off at a tie). When the operands are single-representable the product is exact in f64, so
/// TwoSum gives the exact sum as s+t; round that to ODD in f64 (truncate toward zero, force the lsb when inexact),
/// then the f64->f32 nearest-even rounding is correct because f64 has >= 2 more bits than f32 (PEM fmadds).
/// Operands that are not single-representable are undefined on hardware; they take the plain fma.
fn fma_single(a: f64, c: f64, b: f64) -> f64 {
    let exact_single = |x: f64| !x.is_finite() || (x as f32) as f64 == x;
    if !(exact_single(a) && exact_single(b) && exact_single(c)) { return libm::fma(a, c, b); }
    let p = a * c; // exact: 24 x 24 bits
    let s = p + b;
    if !s.is_finite() { return s; }
    let bb = s - p;
    let t = (p - (s - bb)) + (b - bb); // Knuth TwoSum error term: p + b == s + t exactly
    if t == 0.0 { return s; }
    let toward_zero = (s > 0.0) != (t > 0.0);
    let mut bits = s.to_bits();
    if toward_zero { bits -= 1; } // |s| shrinks by one ulp (s != 0 here: s == 0 would mean t == 0)
    f64::from_bits(bits | 1)
}

fn fsel(cpu: &mut Cpu, w: W) -> R {
    let a = f(cpu, w.ra());
    cpu.fpr[w.rt()] = if a >= 0.0 { cpu.fpr[w.frc()] } else { cpu.fpr[w.rb()] }; // NaN compares false -> frB
    if w.rc() { copy_to_cr1(cpu); }
    Ok(())
}

fn frsp(cpu: &mut Cpu, w: W) -> R {
    let r = frsp_inner(cpu, w);
    if r.is_ok() && w.rc() { copy_to_cr1(cpu); }
    r
}

fn frsp_inner(cpu: &mut Cpu, w: W) -> R {
    need_rne(cpu, w)?;
    let b = f(cpu, w.rb());
    if is_snan(b) {
        raise(cpu, fs::VXSNAN);
        if cpu.fpscr & fs::VE != 0 { return Ok(()); }
    }
    let r = if b.is_nan() { (quiet(b) as f32) as f64 } else { b as f32 as f64 };
    if r.is_infinite() && b.is_finite() { raise(cpu, fs::OX); }
    put(cpu, w, w.rt(), r.to_bits(), true);
    Ok(())
}

/// fcmpu / fcmpo: CR field BF and FPSCR[FPCC] get FL FG FE FU (PEM fcmpu). Unordered sets VXSNAN for a signalling
/// NaN; fcmpo also VXVC for any NaN. FPRF[C] is left alone.
fn fcmp(cpu: &mut Cpu, w: W, ordered: bool) -> R {
    let (a, b) = (f(cpu, w.ra()), f(cpu, w.rb()));
    let nib = if a.is_nan() || b.is_nan() { 1 } else if a < b { 8 } else if a > b { 4 } else { 2 };
    if nib == 1 {
        let snan = is_snan(a) || is_snan(b);
        let mut fl = 0;
        if snan { fl |= fs::VXSNAN; }
        if ordered && (!snan || cpu.fpscr & fs::VE == 0) { fl |= fs::VXVC; }
        if fl != 0 { raise(cpu, fl); }
    }
    cpu.fpscr = (cpu.fpscr & !(0xF << fs::FPRF_SHIFT)) | nib << fs::FPRF_SHIFT;
    cpu.set_cr_field(w.bf(), nib);
    Ok(())
}

/// Round to integral per `mode` (0 nearest-even, 1 toward zero, 2 toward +inf, 3 toward -inf); |x| < 2^52.
fn round_int(x: f64, mode: u32) -> f64 {
    let t = (x as i64) as f64; // truncation
    let frac = x - t;
    let away = if x < 0.0 { -1.0 } else { 1.0 };
    match mode {
        1 => t,
        2 => if frac > 0.0 { t + 1.0 } else { t },
        3 => if frac < 0.0 { t - 1.0 } else { t },
        _ => {
            let a = if frac < 0.0 { -frac } else { frac };
            if a > 0.5 { t + away } else if a == 0.5 { if (t as i64) & 1 == 0 { t } else { t + away } } else { t }
        }
    }
}

/// fctiw / fctiwz: convert to a 32-bit integer in the low word of frD (the high word is undefined; we set
/// 0xFFF80000). Out-of-range and NaN inputs saturate and raise VXCVI (NaN -> 0x80000000).
fn fctiw(cpu: &mut Cpu, w: W, z: bool) -> R {
    let r = fctiw_inner(cpu, w, z);
    if r.is_ok() && w.rc() { copy_to_cr1(cpu); }
    r
}

fn fctiw_inner(cpu: &mut Cpu, w: W, z: bool) -> R {
    let b = f(cpu, w.rb());
    let mode = if z { 1 } else { cpu.fpscr & fs::RN };
    let mut flags = 0;
    let v: u32 = if b.is_nan() {
        flags |= fs::VXCVI;
        if is_snan(b) { flags |= fs::VXSNAN; }
        0x8000_0000
    } else if b.abs() >= 4503599627370496.0 {
        flags |= fs::VXCVI;
        if b < 0.0 { 0x8000_0000 } else { 0x7FFF_FFFF }
    } else {
        let r = round_int(b, mode);
        if r > 2147483647.0 { flags |= fs::VXCVI; 0x7FFF_FFFF }
        else if r < -2147483648.0 { flags |= fs::VXCVI; 0x8000_0000 }
        else {
            if r != b { raise(cpu, fs::XX); cpu.fpscr |= fs::FI; }
            r as i32 as u32
        }
    };
    if flags != 0 {
        raise(cpu, flags);
        if cpu.fpscr & fs::VE != 0 { return Ok(()); }
    }
    cpu.fpr[w.rt()] = 0xFFF8_0000_0000_0000 | v as u64;
    if w.rc() { copy_to_cr1(cpu); }
    Ok(())
}

fn field_mask(fm: u32) -> u32 {
    (0..8).filter(|i| fm & (0x80 >> i) != 0).fold(0, |m, i| m | 0xF << (28 - 4 * i))
}

/// Write the FPSCR bits selected by `m` from `v`. FEX and VX are derived, so they are never taken from `v`; every
/// other bit, FX and the sticky flags included, is copied exactly as given (PEM mtfsf/mtfsfi: no FX-rising rule,
/// that rule belongs to the hardware-raised flags and mtfsb1).
fn write_fpscr(cpu: &mut Cpu, m: u32, v: u32) {
    let m = m & !(fs::FEX | fs::VX);
    cpu.fpscr = (cpu.fpscr & !m) | (v & m);
    fs::fix(cpu);
}

fn mtfsf(cpu: &mut Cpu, w: W) -> R {
    write_fpscr(cpu, field_mask(w.0 >> 17 & 0xFF), cpu.fpr[w.rb()] as u32);
    if w.rc() { copy_to_cr1(cpu); }
    Ok(())
}
fn mtfsfi(cpu: &mut Cpu, w: W) -> R {
    let imm = w.0 >> 12 & 0xF;
    let sh = 28 - 4 * w.bf();
    write_fpscr(cpu, 0xF << sh, imm << sh);
    if w.rc() { copy_to_cr1(cpu); }
    Ok(())
}
