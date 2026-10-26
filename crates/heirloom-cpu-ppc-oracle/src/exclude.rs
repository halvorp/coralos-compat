//! The NAMED exclusions: places where the architecture leaves a result undefined, heirloom-cpu-ppc deliberately does not
//! model a bit (documented in heirloom-cpu-ppc/src/lib.rs), or ppc-rs is wrong. Every exclusion is listed in README.md
//! with the PowerPC Programming Environments Manual (PEM, MPCFPE32B) rule behind it, and the run reports how many
//! comparisons each one relaxed. Nothing is excluded by "the two differ".
use crate::compare::Relax;
use crate::world::{Snap, Tr};

impl Snap {
    fn trap_is_some(&self) -> bool { self.trap != Tr::None }
}

pub const XX: u32 = 1 << 25;
pub const UX: u32 = 1 << 27;
pub const FI: u32 = 1 << 17;
pub const FR: u32 = 1 << 18;
pub const FEX: u32 = 1 << 30;
pub const FX: u32 = 1 << 31;
pub const FPRF: u32 = 0x1F << 12;
pub const XE: u32 = 1 << 3;
pub const UE: u32 = 1 << 5;

fn op(w: u32) -> u32 { w >> 26 }
fn xo(w: u32) -> u32 { w >> 1 & 0x3FF }
fn rt(w: u32) -> usize { (w >> 21 & 31) as usize }
fn ra(w: u32) -> usize { (w >> 16 & 31) as usize }
fn rb(w: u32) -> usize { (w >> 11 & 31) as usize }
fn rc(w: u32) -> bool { w & 1 != 0 }

/// op 59/63 instructions that compute (and so may raise the flags heirloom-cpu-ppc does not track).
fn fp_arith(w: u32) -> bool {
    match op(w) {
        59 => true,
        63 => matches!(xo(w) & 0x1F, 18 | 20 | 21 | 22 | 25 | 26 | 28..=31) || matches!(xo(w), 12 | 14 | 15),
        _ => false,
    }
}

/// PEM 4.2.? / 3.3.? : an op-59 (single-precision) instruction whose operand is not exactly representable in
/// single precision gives an undefined result. The operands an instruction reads: A,B (add/sub/div), A,C (mul),
/// A,B,C (fused), B (sqrt/res).
fn single_operands(w: u32) -> &'static [fn(u32) -> usize] {
    if op(w) != 59 { return &[]; }
    match w >> 1 & 0x1F {
        18 | 20 | 21 => &[ra, rb],
        25 => &[ra, frc],
        22 | 24 => &[rb],
        28..=31 => &[ra, rb, frc],
        _ => &[],
    }
}
fn frc(w: u32) -> usize { (w >> 6 & 31) as usize }
pub fn exact_single(bits: u64) -> bool {
    let f = f64::from_bits(bits);
    if f.is_nan() { bits & 0x1FFF_FFFF == 0 } else { (f as f32) as f64 == f && ((f as f32) as f64).to_bits() == bits }
}
pub fn nonsingle_operand(w: u32, pre: &Snap) -> bool {
    single_operands(w).iter().any(|r| !exact_single(pre.fpr[r(w)]))
}

/// EA and direction (`true` = store) of an lfs/stfs-family access (lfs, lfsu, lfsx, lfsux, stfs, stfsu, stfsx, stfsux).
fn fp_single_ea(w: u32, pre: &Snap) -> Option<(u32, bool)> {
    let base = |upd: bool| if ra(w) == 0 && !upd { 0 } else { pre.gpr[ra(w)] };
    let disp = w as u16 as i16 as i32 as u32;
    match (op(w), xo(w)) {
        (48 | 49 | 52 | 53, _) => Some((base(op(w) & 1 == 1).wrapping_add(disp), op(w) >= 52)),
        (31, 535 | 567 | 663 | 695) => Some((base(matches!(xo(w), 567 | 695)).wrapping_add(pre.gpr[rb(w)]), xo(w) >= 663)),
        _ => None,
    }
}
fn stfs_ea(w: u32, pre: &Snap) -> Option<u32> { fp_single_ea(w, pre).filter(|e| e.1).map(|e| e.0) }

/// The relaxation for `word` executed from `pre`. `post_oracle` is consulted only for what the oracle newly set.
pub fn relax(w: u32, pre: &Snap, post_heirloom: &Snap, post_oracle: &Snap) -> Relax {
    let mut r = Relax::full();
    // XER[3:24] are reserved: mfspr XER reads them as zero on heirloom-cpu-ppc; ppc-rs keeps what mtspr wrote.
    if op(w) == 31 && xo(w) == 339 && (w >> 16 & 31) | (w >> 11 & 31) << 5 == 1 {
        r.gpr[rt(w)] = heirloom_cpu_ppc::cpu::XER_MASK;
        r.note("xer-reserved-bits-read");
    }
    // FPRF[C] for a single-precision result that is denormal IN SINGLE but a normal double in the register: heirloom
    // classifies it as a denormalized number (PEM 2.1.4 Table "Floating-point result flags"; the instruction is
    // single-precision so the result class is judged in single, as on the 750); ppc-rs judges the register as a double.
    if (op(w) == 59 || (op(w) == 63 && xo(w) == 12)) && !post_heirloom.trap_is_some() {
        let x = f64::from_bits(post_heirloom.fpr[rt(w)]);
        if x != 0.0 && x.is_finite() && x.abs() < f32::MIN_POSITIVE as f64 {
            // The class is judged by the model (PEM Table "Floating-point result flags": denormalized = C plus the
            // sign's FG/FL), not assumed: heirloom must have written exactly that, ppc-rs differs only in C.
            let want = crate::models::fprf_denormal(post_heirloom.fpr[rt(w)]);
            let got = post_heirloom.fpscr & FPRF;
            // A write suppressed by an enabled exception leaves both frD and FPRF as they were: nothing to relax.
            let unwritten = post_heirloom.fpr[rt(w)] == pre.fpr[rt(w)] && got == pre.fpscr & FPRF;
            if got == want {
                r.fpscr &= !(1 << 16); // FPRF[C] = manual bit 15
                r.note("ppcrs-fprf-single-denormal");
            } else if !unwritten {
                r.model_fail.push(format!("FPRF of a single-denormal result is {got:#x}, PEM gives {want:#x}"));
            }
        }
    }
    // PEM divw/divwu: x/0 (and 0x80000000 / -1 for divw): rD and CR0[LT,GT,EQ] are undefined (OV is defined).
    if op(w) == 31 && matches!(xo(w) & 0x1FF, 491 | 459) {
        let (a, b) = (pre.gpr[ra(w)], pre.gpr[rb(w)]);
        if b == 0 || (xo(w) & 0x1FF == 491 && a == 0x8000_0000 && b == u32::MAX) {
            r.gpr[rt(w)] = 0;
            r.cr &= !0xE000_0000;
            r.note("div-undefined-result");
        }
    }
    // PEM fctiw/fctiwz/mffs: FRT[0:31] is undefined.
    if op(w) == 63 && matches!(xo(w), 14 | 15 | 583) {
        r.fpr[rt(w)] = 0xFFFF_FFFF;
        r.note("fp-int-result-high-word-undefined");
    }
    // ppc-rs takes the NaN result from rustc_apfloat (SNaN-first, sign flipped by fmsub/fnmsub's negation, payload
    // dropped for sqrt/rsqrte); PEM: first NaN operand in the order frA, frB, frC, quieted, sign/payload kept.
    if let Some(m) = crate::models::nan_result(w, pre) {
        if post_heirloom.fpr[rt(w)] == m && post_heirloom.trap == Tr::None {
            r.fpr[rt(w)] = 0;
            r.note("ppcrs-nan-selection");
        }
    }
    // frsp of a value far below the single denormal range: ppc-rs returns a garbage value (it stores a double whose
    // exponent was not rebiased); PEM rounds to the nearest single (zero or the smallest denormal).
    if op(w) == 63 && xo(w) == 12 && post_heirloom.trap == Tr::None {
        let b = pre.fpr[rb(w)];
        let f = f64::from_bits(b);
        if f.is_finite() && f != 0.0 && (b >> 52 & 0x7FF) < 874 && post_heirloom.fpr[rt(w)] == crate::state::widen(f as f32) {
            r.fpr[rt(w)] = 0;
            r.fpscr &= !crate::exclude::FPRF;
            r.note("ppcrs-frsp-deep-underflow");
        }
    }
    // fctiw/fctiwz of a NaN: PEM gives 0x80000000 (VXCVI); ppc-rs gives 0.
    if op(w) == 63 && matches!(xo(w), 14 | 15) {
        let b = f64::from_bits(pre.fpr[rb(w)]);
        if b.is_nan() && post_heirloom.fpr[rt(w)] as u32 == crate::models::FCTIW_NAN && post_oracle.fpr[rt(w)] as u32 != crate::models::FCTIW_NAN {
            r.fpr[rt(w)] = 0;
            r.note("ppcrs-fctiw-nan");
        }
    }
    // stfs: PEM truncates (no rounding); ppc-rs rounds per FPSCR[RN]. The heirloom word is checked against the model.
    if let Some(ea) = stfs_ea(w, pre) {
        if post_heirloom.trap == Tr::None && post_oracle.trap == Tr::None {
            let e = ea as usize;
            let got = u32::from_be_bytes(post_heirloom.mem[e..e + 4].try_into().unwrap());
            match crate::models::stfs_word(pre.fpr[rt(w)]) {
                Some(m) if m != got => r.model_fail.push(format!("stfs stored {got:#010x}, PEM store conversion gives {m:#010x}")),
                Some(_) => r.note("ppcrs-stfs-rounds"),
                None => r.note("stfs-below-single-denormal-undefined"),
            }
            r.mem_skip.push((e, e + 4));
        }
    }
    // heirloom-cpu-ppc does not track XX, FI, FR, UX (lib.rs "Deliberate deviations"); FX/FEX derive from them.
    if fp_arith(w) {
        r.fpscr &= !(XX | UX | FI | FR);
        let new_untracked = post_oracle.fpscr & (XX | UX) & !pre.fpscr != 0;
        let enabled_untracked = (post_oracle.fpscr & XX != 0 && post_oracle.fpscr & XE != 0) || (post_oracle.fpscr & UX != 0 && post_oracle.fpscr & UE != 0);
        if new_untracked || enabled_untracked {
            r.fpscr &= !(FX | FEX);
            if rc(w) { r.cr &= !0x0C00_0000; }
        }
        r.note("heirloom-untracked-fpscr-xx-ux-fi-fr");
    }
    // fctiw/fctiwz raise XX/FI in heirloom too, but FR is not tracked.
    // ppc-rs quiets a signalling NaN on lfs; PEM lfs copies the fraction unchanged (no FPSCR, no quieting). Heirloom's
    // loaded value is checked against the PEM load conversion for EVERY lfs-family access; only a loaded signalling
    // NaN that ppc-rs quieted (and heirloom left as the model says) is relaxed, and only in bit 51.
    if let Some((ea, false)) = fp_single_ea(w, pre) {
        let e = ea as usize;
        if post_heirloom.trap == Tr::None && e + 4 <= pre.mem.len() {
            let word = u32::from_be_bytes(pre.mem[e..e + 4].try_into().unwrap());
            let m = crate::models::lfs_double(word);
            if post_heirloom.fpr[rt(w)] != m {
                r.model_fail.push(format!("lfs loaded {:#018x}, PEM load conversion gives {m:#018x}", post_heirloom.fpr[rt(w)]));
            } else if crate::models::is_snan_single(word) && post_oracle.fpr[rt(w)] == m | 1 << 51 {
                r.fpr[rt(w)] &= !(1 << 51);
                r.note("ppcrs-lfs-quiets-snan");
            }
        }
    }
    r
}

/// A trap-class mismatch that is not a defect of heirloom-cpu-ppc. `Some(name)` excludes the case.
pub fn trap_excuse(w: u32, heirloom: &Snap, oracle: &Snap) -> Option<&'static str> {
    // heirloom rejects an encoding the architecture marks invalid: results are boundedly undefined, anything goes.
    if let Some(why) = heirloom.invalid_why {
        return Some(match why {
            "update-form load/store with RA=0 or RA=RT" => "invalid-form-update-ra",
            "lmw with RA in the loaded range" => "invalid-form-lmw-ra",
            "lswi/lswx with RA or RB inside the loaded register range" => "invalid-form-lsw-overlap",
            "update-form FP load/store with RA=0" => "invalid-form-update-ra",
            "bcctr with BO decrementing CTR" => "invalid-form-bcctr-bo",
            "stwcx. with Rc=0" => "invalid-form-stwcx-rc0",
            _ => "invalid-form-other",
        });
    }
    // lmw/stmw/lwarx/stwcx. need a word-aligned EA: alignment exception OR boundedly undefined (PEM 4.2.3).
    if heirloom.trap == Tr::Align && oracle.trap != Tr::Align && (matches!(op(w), 46 | 47) || (op(w) == 31 && matches!(xo(w), 20 | 150))) {
        return Some("misaligned-multiple-or-reserved");
    }
    // heirloom refuses arithmetic FP under a directed rounding mode by name (lib.rs).
    if heirloom.trap == Tr::Illegal && heirloom.note.contains("directed rounding") {
        return Some("heirloom-rn-directed-rounding");
    }
    // ppc-rs refuses an encoding with a reserved bit set (IllegalInstruction/InvalidForm); heirloom-cpu-ppc ignores reserved
    // bits. PEM: a reserved bit that is not zero makes the instruction form invalid and the result boundedly
    // undefined, so executing it is allowed. (Only reachable when a store patches code: the generator keeps reserved
    // bits zero.)
    if heirloom.trap == Tr::None && oracle.trap == Tr::Illegal && oracle.note.contains("InvalidForm") {
        return Some("reserved-bits-set-ignored-by-heirloom");
    }
    // mfspr of the time base: user code uses mftb; an mfspr of TBL/TBU is outside the architected user SPR set.
    if op(w) == 31 && xo(w) == 339 && oracle.trap == Tr::Illegal && heirloom.trap == Tr::None {
        return Some("mfspr-time-base");
    }
    None
}

/// FP modes heirloom-cpu-ppc does not model (heirloom-cpu-ppc/src/lib.rs "Deliberate deviations"): an enabled overflow / underflow
/// exception (PEM writes the exponent-wrapped result; heirloom writes the unwrapped one) and FPSCR[NI].
/// Narrow: the OX clause needs an infinity made from finite operands that is not a divide by zero (that is ZX), and
/// the UX clause needs a result that genuinely underflowed (a tiny non-zero value, or a zero whose exact result is
/// non-zero), never an exact zero such as 0 + -0 or a cancelling sum.
pub fn fp_mode_excuse(w: u32, pre: &Snap, heirloom: &Snap) -> Option<&'static str> {
    if !fp_arith(w) || heirloom.trap != Tr::None { return None; }
    const NI: u32 = 4;
    const OE: u32 = 1 << 6;
    const UE_: u32 = 1 << 5;
    if pre.fpscr & NI != 0 { return Some("fpscr-ni-non-ieee-mode"); }
    let used: &[u32] = if xo(w) == 12 && op(w) == 63 { &[11] } else { crate::models::operands(w) };
    let val = |s: u32| f64::from_bits(pre.fpr[(w >> s & 31) as usize]);
    let operands_finite = used.iter().all(|s| val(*s).is_finite());
    let res = f64::from_bits(heirloom.fpr[rt(w)]);
    // Division by zero (fdiv, fres, frsqrte of a zero divisor) gives an infinity through ZX, not OX.
    let div_by_zero = match w >> 1 & 0x1F { 18 => val(11) == 0.0 && val(16) != 0.0, 24 | 26 => val(11) == 0.0, _ => false };
    if pre.fpscr & OE != 0 && operands_finite && res.is_infinite() && !div_by_zero { return Some("heirloom-unwrapped-result-ox-ux-enabled"); }
    let tiny = if op(w) == 59 || xo(w) == 12 { f32::MIN_POSITIVE as f64 } else { f64::MIN_POSITIVE };
    if pre.fpscr & UE_ != 0 && operands_finite && res.is_finite() {
        let underflowed = if res != 0.0 { res.abs() <= tiny } else { exact_result_nonzero(w, &val) };
        if underflowed { return Some("heirloom-unwrapped-result-ox-ux-enabled"); }
    }
    None
}

/// For a zero result: was the exact (infinitely precise) result non-zero, i.e. did it flush to zero by underflow?
/// Add/subtract are exact in the denormal range, so a zero sum is a true zero; a product, quotient or conversion
/// underflows to zero only if no operand is zero; a fused op only if the product is non-zero and does not cancel b.
fn exact_result_nonzero(w: u32, val: &dyn Fn(u32) -> f64) -> bool {
    let (a, b, c) = (val(16), val(11), val(6));
    match (op(w), w >> 1 & 0x1F) {
        (59 | 63, 18) => a != 0.0 && b.is_finite() && b != 0.0,
        (59 | 63, 25) => a != 0.0 && c != 0.0,
        (63, 12) => b != 0.0,
        (59 | 63, 28..=31) => {
            let (b, p) = if matches!(w >> 1 & 0x1F, 28 | 30) { (-b, a * c) } else { (b, a * c) };
            a != 0.0 && c != 0.0 && !(p.is_finite() && p != 0.0 && p == -b)
        }
        (59 | 63, 24 | 26) => false, // estimates of a finite non-zero operand are not zero by underflow here
        _ => false,                   // add, sub, sqrt: exact zero
    }
}
