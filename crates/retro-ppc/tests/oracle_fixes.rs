//! Regressions found by the differential fuzz against ppc-rs (crates/retro-ppc-oracle). Each test names the PowerPC
//! Programming Environments Manual (PEM, MPCFPE32B) rule it enforces and has a control that must keep its behaviour.
mod common;
use common::*;
use retro_ppc::Cpu;

const VE: u32 = 1 << 7;
const ZE: u32 = 1 << 4;
const ZX: u32 = 1 << 26;
const VXSNAN: u32 = 1 << 24;
const VXIMZ: u32 = 1 << 20;
const FX: u32 = 1 << 31;
const VX: u32 = 1 << 29;
const FEX: u32 = 1 << 30;

fn a_form(op: u32, xo: u32, frt: u32, fra: u32, frb: u32, frc: u32, rc: u32) -> u32 {
    op << 26 | frt << 21 | fra << 16 | frb << 11 | frc << 6 | xo << 1 | rc
}
fn stfs(frs: u32, ra: u32, d: u32) -> u32 { 52 << 26 | frs << 21 | ra << 16 | d }
fn run(word: u32, setup: impl FnOnce(&mut Cpu)) -> Cpu {
    let (c, _, st) = run_words(&[word], |c, _| setup(c));
    assert!(st.halted && st.trap.is_none(), "{st:?}");
    c
}
fn store_single(d: f64, rn: u32) -> u32 {
    let (_, v, st) = run_words(&[stfs(1, 3, 0)], |c, _| { c.gpr[3] = 0x4000; c.fpr[1] = d.to_bits(); c.fpscr = rn; });
    assert!(st.trap.is_none());
    v.mem.read_u32(0x4000).unwrap()
}

/// PEM stfs pseudocode: FRS[1:11] > 896 gives WORD[2:31] <- FRS[5:34], i.e. the low 29 fraction bits are DROPPED, not
/// rounded, and FPSCR[RN] is not consulted. (A program wanting a rounded single does frsp first.)
#[test]
fn stfs_truncates_instead_of_rounding_and_ignores_rn() {
    let three_quarter_ulp = 1.0 + 0.75 * 2f64.powi(-23);
    for rn in 0..4 {
        assert_eq!(store_single(three_quarter_ulp, rn), 0x3F80_0000, "truncated, RN={rn}");
    }
    assert_eq!(store_single(-three_quarter_ulp, 0), 0xBF80_0000, "sign kept, magnitude truncated");
    // CONTROL: exactly representable values are stored exactly, and frsp (which rounds) then stfs gives the rounded word.
    assert_eq!(store_single(1.0 + 2f64.powi(-23), 0), 0x3F80_0001);
    let rounded = run(a_form(63, 12, 1, 0, 2, 0, 0) & !(31 << 16), |c| c.fpr[2] = three_quarter_ulp.to_bits());
    assert_eq!(rounded.fpr_f64(1), (1.0 + 2f64.powi(-23)) as f32 as f64, "frsp rounds to nearest");
}

/// PEM stfs: 874 <= FRS[1:11] <= 896 denormalises (the implicit 1 shifted right) and truncates the rest.
#[test]
fn stfs_denormalises_and_truncates_in_the_single_denormal_range() {
    // 512.75 * 2^-149: between the single denormals 512 and 513 units; truncation gives 512 = 0x200.
    assert_eq!(store_single(512.75 * 2f64.powi(-149), 0), 0x0000_0200);
    assert_eq!(store_single(-512.75 * 2f64.powi(-149), 0), 0x8000_0200);
    assert_eq!(store_single(2f64.powi(-149), 0), 1, "CONTROL: the smallest single denormal is exact");
    // Below the range is architecturally undefined; we store a signed zero (documented in fpmem.rs).
    assert_eq!(store_single(2f64.powi(-160), 0), 0);
    assert_eq!(store_single(-(2f64.powi(-160)), 0), 0x8000_0000);
    assert_eq!(store_single(0.0, 0), 0, "CONTROL: +0");
    assert_eq!(store_single(-0.0, 0), 0x8000_0000, "CONTROL: -0");
}

/// PEM lfs: for an infinity/NaN source the fraction is copied unchanged; a signalling NaN stays signalling and
/// FPSCR is untouched (ppc-rs quiets it: oracle exclusion ppcrs-lfs-quiets-snan).
#[test]
fn lfs_keeps_a_signalling_nan_and_leaves_fpscr_alone() {
    let (c, _, st) = run_words(&[0xc0230000], |c, v| { c.gpr[3] = 0x4000; c.fpscr = 0; v.mem.write_u32(0x4000, 0x7F80_0001).unwrap(); });
    assert!(st.trap.is_none());
    assert_eq!(c.fpr[1], 0x7FF0_0000_2000_0000);
    assert_eq!(c.fpscr, 0, "no VXSNAN: a load is not an arithmetic operation");
    let (c, _, _) = run_words(&[0xc0230000], |c, v| { c.gpr[3] = 0x4000; v.mem.write_u32(0x4000, 0x7FC0_0001).unwrap(); });
    assert_eq!(c.fpr[1], 0x7FF8_0000_2000_0000, "CONTROL: a QNaN keeps its payload");
}

/// PEM fmadd: infinity x zero is a VXIMZ invalid operation even when the addend frB is a QNaN; the QNaN is the result.
#[test]
fn fmadd_raises_vximz_for_inf_times_zero_with_a_nan_addend() {
    let fmadd = a_form(63, 29, 1, 2, 3, 4, 0); // frD=1 frA=2 frB=3 frC=4
    let qnan = 0x7FF8_0000_DEAD_BEEFu64;
    let c = run(fmadd, |c| { c.fpr[2] = f64::INFINITY.to_bits(); c.fpr[3] = qnan; c.fpr[4] = 0f64.to_bits(); });
    assert_ne!(c.fpscr & VXIMZ, 0, "VXIMZ raised");
    assert_eq!(c.fpr[1], qnan, "the QNaN operand is the result");
    // CONTROL: without inf x 0 a QNaN addend raises nothing.
    let c = run(fmadd, |c| { c.fpr[2] = 2f64.to_bits(); c.fpr[3] = qnan; c.fpr[4] = 3f64.to_bits(); });
    assert_eq!(c.fpscr & VXIMZ, 0);
    // CONTROL: an SNaN addend with inf x 0 sets both flags.
    let c = run(fmadd, |c| { c.fpr[2] = f64::INFINITY.to_bits(); c.fpr[3] = 0x7FF0_0000_0000_0001; c.fpr[4] = 0f64.to_bits(); });
    assert_eq!(c.fpscr & (VXIMZ | VXSNAN), VXIMZ | VXSNAN);
}

/// PEM zero-divide exception, enabled (FPSCR[ZE]=1): ZX is set and the target FPR is left unchanged.
#[test]
fn fdiv_by_zero_with_ze_leaves_the_target_unchanged() {
    let fdiv = a_form(63, 18, 1, 2, 3, 0, 0);
    let c = run(fdiv, |c| { c.fpscr = ZE; c.fpr[1] = 7f64.to_bits(); c.fpr[2] = 1f64.to_bits(); c.fpr[3] = 0f64.to_bits(); });
    assert_eq!(c.fpr[1], 7f64.to_bits(), "frD unchanged");
    assert_ne!(c.fpscr & ZX, 0);
    // CONTROL: ZE=0 writes the infinity.
    let c = run(fdiv, |c| { c.fpr[1] = 7f64.to_bits(); c.fpr[2] = 1f64.to_bits(); c.fpr[3] = 0f64.to_bits(); });
    assert_eq!(c.fpr_f64(1), f64::INFINITY);
}

/// PEM: Rc=1 sets CR1 from FPSCR[FX FEX VX OX] even when an enabled invalid operation suppressed the result write.
#[test]
fn record_form_updates_cr1_when_the_result_write_is_suppressed() {
    let fadd_rc = a_form(63, 21, 1, 2, 3, 0, 1);
    let c = run(fadd_rc, |c| { c.fpscr = VE; c.fpr[1] = 7f64.to_bits(); c.fpr[2] = 0x7FF0_0000_0000_0001; c.fpr[3] = 1f64.to_bits(); });
    assert_eq!(c.fpr[1], 7f64.to_bits(), "result suppressed (VE)");
    assert_eq!(c.cr_field(1), (FX | FEX | VX) >> 28, "CR1 = FX FEX VX OX");
    // CONTROL: Rc=0 leaves CR1 alone.
    let c = run(fadd_rc & !1, |c| { c.fpscr = VE; c.cr = 0; c.fpr[2] = 0x7FF0_0000_0000_0001; c.fpr[3] = 1f64.to_bits(); });
    assert_eq!(c.cr, 0);
}
