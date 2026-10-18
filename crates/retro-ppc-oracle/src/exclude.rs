//! The NAMED exclusions: places where the architecture leaves a result undefined, retro-ppc deliberately does not
//! model a bit (documented in retro-ppc/src/lib.rs), or ppc-rs is wrong. Every exclusion is listed in README.md
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

/// op 59/63 instructions that compute (and so may raise the flags retro-ppc does not track).
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

/// EA of an stfs-family store (stfs, stfsu, stfsx, stfsux), if `w` is one.
fn stfs_ea(w: u32, pre: &Snap) -> Option<u32> {
    let base = |upd: bool| if ra(w) == 0 && !upd { 0 } else { pre.gpr[ra(w)] };
    match (op(w), xo(w)) {
        (52, _) => Some(base(false).wrapping_add(w as u16 as i16 as i32 as u32)),
        (53, _) => Some(base(true).wrapping_add(w as u16 as i16 as i32 as u32)),
        (31, 663) => Some(base(false).wrapping_add(pre.gpr[rb(w)])),
        (31, 695) => Some(base(true).wrapping_add(pre.gpr[rb(w)])),
        _ => None,
    }
}

/// The relaxation for `word` executed from `pre`. `post_oracle` is consulted only for what the oracle newly set.
pub fn relax(w: u32, pre: &Snap, post_retro: &Snap, post_oracle: &Snap) -> Relax {
    let mut r = Relax::full();
    // XER[3:24] are reserved: mfspr XER reads them as zero on retro-ppc; ppc-rs keeps what mtspr wrote.
    if op(w) == 31 && xo(w) == 339 && (w >> 16 & 31) | (w >> 11 & 31) << 5 == 1 {
        r.gpr[rt(w)] = retro_ppc::cpu::XER_MASK;
        r.note("xer-reserved-bits-read");
    }
    // FPRF[C] for a single-precision result that is denormal IN SINGLE but a normal double in the register: retro
    // classifies it as a denormalized number (PEM 2.1.4 Table "Floating-point result flags"; the instruction is
    // single-precision so the result class is judged in single, as on the 750); ppc-rs judges the register as a double.
    if (op(w) == 59 || (op(w) == 63 && xo(w) == 12)) && !post_retro.trap_is_some() {
        let x = f64::from_bits(post_retro.fpr[rt(w)]);
        if x != 0.0 && x.is_finite() && x.abs() < f32::MIN_POSITIVE as f64 {
            r.fpscr &= !(1 << 16); // FPRF[C] = manual bit 15
            r.note("ppcrs-fprf-single-denormal");
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
        if post_retro.fpr[rt(w)] == m && post_retro.trap == Tr::None {
            r.fpr[rt(w)] = 0;
            r.note("ppcrs-nan-selection");
        }
    }
    // frsp of a value far below the single denormal range: ppc-rs returns a garbage value (it stores a double whose
    // exponent was not rebiased); PEM rounds to the nearest single (zero or the smallest denormal).
    if op(w) == 63 && xo(w) == 12 && post_retro.trap == Tr::None {
        let b = pre.fpr[rb(w)];
        let f = f64::from_bits(b);
        if f.is_finite() && f != 0.0 && (b >> 52 & 0x7FF) < 874 && post_retro.fpr[rt(w)] == crate::state::widen(f as f32) {
            r.fpr[rt(w)] = 0;
            r.fpscr &= !crate::exclude::FPRF;
            r.note("ppcrs-frsp-deep-underflow");
        }
    }
    // fctiw/fctiwz of a NaN: PEM gives 0x80000000 (VXCVI); ppc-rs gives 0.
    if op(w) == 63 && matches!(xo(w), 14 | 15) {
        let b = f64::from_bits(pre.fpr[rb(w)]);
        if b.is_nan() && post_retro.fpr[rt(w)] as u32 == crate::models::FCTIW_NAN && post_oracle.fpr[rt(w)] as u32 != crate::models::FCTIW_NAN {
            r.fpr[rt(w)] = 0;
            r.note("ppcrs-fctiw-nan");
        }
    }
    // stfs: PEM truncates (no rounding); ppc-rs rounds per FPSCR[RN]. The retro word is checked against the model.
    if let Some(ea) = stfs_ea(w, pre) {
        if post_retro.trap == Tr::None && post_oracle.trap == Tr::None {
            let e = ea as usize;
            let got = u32::from_be_bytes(post_retro.mem[e..e + 4].try_into().unwrap());
            match crate::models::stfs_word(pre.fpr[rt(w)]) {
                Some(m) if m != got => r.model_fail.push(format!("stfs stored {got:#010x}, PEM store conversion gives {m:#010x}")),
                Some(_) => r.note("ppcrs-stfs-rounds"),
                None => r.note("stfs-below-single-denormal-undefined"),
            }
            r.mem_skip.push((e, e + 4));
        }
    }
    // retro-ppc does not track XX, FI, FR, UX (lib.rs "Deliberate deviations"); FX/FEX derive from them.
    if fp_arith(w) {
        r.fpscr &= !(XX | UX | FI | FR);
        let new_untracked = post_oracle.fpscr & (XX | UX) & !pre.fpscr != 0;
        let enabled_untracked = (post_oracle.fpscr & XX != 0 && post_oracle.fpscr & XE != 0) || (post_oracle.fpscr & UX != 0 && post_oracle.fpscr & UE != 0);
        if new_untracked || enabled_untracked {
            r.fpscr &= !(FX | FEX);
            if rc(w) { r.cr &= !0x0C00_0000; }
        }
        r.note("retro-untracked-fpscr-xx-ux-fi-fr");
    }
    // fctiw/fctiwz raise XX/FI in retro too, but FR is not tracked.
    // ppc-rs quiets a signalling NaN on lfs; PEM lfs copies the fraction unchanged (no FPSCR, no quieting).
    let lfs = matches!(op(w), 48 | 49) || (op(w) == 31 && matches!(xo(w), 535 | 567));
    if lfs {
        r.fpr[rt(w)] &= !(1 << 51);
        r.note("ppcrs-lfs-quiets-snan");
    }
    r
}

/// A trap-class mismatch that is not a defect of retro-ppc. `Some(name)` excludes the case.
pub fn trap_excuse(w: u32, retro: &Snap, oracle: &Snap) -> Option<&'static str> {
    // retro rejects an encoding the architecture marks invalid: results are boundedly undefined, anything goes.
    if let Some(why) = retro.invalid_why {
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
    if retro.trap == Tr::Align && oracle.trap != Tr::Align && (matches!(op(w), 46 | 47) || (op(w) == 31 && matches!(xo(w), 20 | 150))) {
        return Some("misaligned-multiple-or-reserved");
    }
    // retro refuses arithmetic FP under a directed rounding mode by name (lib.rs).
    if retro.trap == Tr::Illegal && retro.note.contains("directed rounding") {
        return Some("retro-rn-directed-rounding");
    }
    // ppc-rs refuses an encoding with a reserved bit set (IllegalInstruction/InvalidForm); retro-ppc ignores reserved
    // bits. PEM: a reserved bit that is not zero makes the instruction form invalid and the result boundedly
    // undefined, so executing it is allowed. (Only reachable when a store patches code: the generator keeps reserved
    // bits zero.)
    if retro.trap == Tr::None && oracle.trap == Tr::Illegal && oracle.note.contains("InvalidForm") {
        return Some("reserved-bits-set-ignored-by-retro");
    }
    // mfspr of the time base: user code uses mftb; an mfspr of TBL/TBU is outside the architected user SPR set.
    if op(w) == 31 && xo(w) == 339 && oracle.trap == Tr::Illegal && retro.trap == Tr::None {
        return Some("mfspr-time-base");
    }
    None
}

/// FP modes retro-ppc does not model (retro-ppc/src/lib.rs "Deliberate deviations"): an enabled overflow / underflow
/// exception (PEM writes the exponent-wrapped result; retro writes the unwrapped one) and FPSCR[NI].
pub fn fp_mode_excuse(w: u32, pre: &Snap, retro: &Snap) -> Option<&'static str> {
    if !fp_arith(w) { return None; }
    const NI: u32 = 4;
    const OE: u32 = 1 << 6;
    const UE_: u32 = 1 << 5;
    if pre.fpscr & NI != 0 { return Some("fpscr-ni-non-ieee-mode"); }
    // Overflow with OE set: retro's result is an infinity (or max finite) made from finite operands.
    let used: &[u32] = if xo(w) == 12 && op(w) == 63 { &[11] } else { crate::models::operands(w) };
    let operands_finite = used.iter().all(|s| f64::from_bits(pre.fpr[(w >> s & 31) as usize]).is_finite());
    let res = f64::from_bits(retro.fpr[rt(w)]);
    if pre.fpscr & OE != 0 && operands_finite && res.is_infinite() { return Some("retro-unwrapped-result-ox-ux-enabled"); }
    // Underflow with UE set: the result is tiny (or zero from a non-zero exact result).
    let tiny = if op(w) == 59 || xo(w) == 12 { f32::MIN_POSITIVE as f64 } else { f64::MIN_POSITIVE };
    if pre.fpscr & UE_ != 0 && res.is_finite() && res.abs() <= tiny { return Some("retro-unwrapped-result-ox-ux-enabled"); }
    None
}
