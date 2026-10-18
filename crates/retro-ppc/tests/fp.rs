//! Floating point: arithmetic, fused multiply-add, single rounding, NaN / invalid / divide-by-zero / overflow flags,
//! conversions, compares, fsel, FPSCR moves and the FP load/store formats. Expected FPRF / FPSCR values are from
//! PEM Table 3-? (result flags) and the per-instruction exception lists. Each family has a negative control.
mod common;
use common::*;
use retro_ppc::Cpu;

// FPSCR bits in value form (manual bit n = 1 << (31 - n)).
const FX: u32 = 1 << 31;
const FEX: u32 = 1 << 30;
const VX: u32 = 1 << 29;
const OX: u32 = 1 << 28;
const ZX: u32 = 1 << 26;
const XX: u32 = 1 << 25;
const VXSNAN: u32 = 1 << 24;
const VXISI: u32 = 1 << 23;
const VXIDI: u32 = 1 << 22;
const VXZDZ: u32 = 1 << 21;
const VXIMZ: u32 = 1 << 20;
const VXVC: u32 = 1 << 19;
const VXSQRT: u32 = 1 << 9;
const VXCVI: u32 = 1 << 8;

const QNAN_DEFAULT: u64 = 0x7FF8_0000_0000_0000;
const SNAN: u64 = 0x7FF0_0000_0000_0001;

fn fpr_run(lines: &[&str], f: &[(usize, f64)]) -> Cpu {
    let (c, _, st) = prog(lines, |c, _| for &(r, v) in f { c.set_fpr_f64(r, v) });
    assert!(st.halted && st.trap.is_none(), "{lines:?}: {st:?}");
    c
}
fn bits_run(lines: &[&str], f: &[(usize, u64)]) -> Cpu {
    let (c, _, st) = prog(lines, |c, _| for &(r, v) in f { c.fpr[r] = v });
    assert!(st.halted && st.trap.is_none(), "{lines:?}: {st:?}");
    c
}
/// FPRF = FPSCR[15:19]: C FL FG FE FU.
fn fprf(c: &Cpu) -> u32 { c.fpscr >> 12 & 0x1F }
const NORM_POS: u32 = 0b00100;
const NORM_NEG: u32 = 0b01000;
const ZERO_POS: u32 = 0b00010;
const ZERO_NEG: u32 = 0b10010;
const INF_POS: u32 = 0b00101;
const INF_NEG: u32 = 0b01001;
const QNAN: u32 = 0b10001;
const DEN_POS: u32 = 0b10100;

#[test]
fn basic_arithmetic_and_result_class() {
    let c = fpr_run(&["fadd 1,2,3", "fsub 4,2,3", "fmul 5,2,3", "fdiv 6,2,3"], &[(2, 1.5), (3, 2.25)]);
    assert_eq!((c.fpr_f64(1), c.fpr_f64(4), c.fpr_f64(5), c.fpr_f64(6)), (3.75, -0.75, 3.375, 1.5 / 2.25));
    assert_eq!(c.fpscr & !(0xF << 12), 0, "no exception flags");
    let c = fpr_run(&["fsub 1,2,3"], &[(2, 1.5), (3, 2.25)]);
    assert_eq!(fprf(&c), NORM_NEG);
    let c = fpr_run(&["fsub 1,2,3"], &[(2, 2.0), (3, 2.0)]);
    assert_eq!((c.fpr[1], fprf(&c)), (0, ZERO_POS), "x - x is +0 in round-to-nearest");
    let c = fpr_run(&["fmul 1,2,3"], &[(2, -0.0), (3, 5.0)]);
    assert_eq!((c.fpr[1], fprf(&c)), (1 << 63, ZERO_NEG));
}

#[test]
fn single_precision_forms_round_to_single() {
    let tiny = 2f64.powi(-30);
    let c = fpr_run(&["fadds 1,2,3", "fadd 4,2,3"], &[(2, 1.0), (3, tiny)]);
    assert_eq!(c.fpr_f64(1), 1.0, "fadds rounds 1+2^-30 to single");
    assert_eq!(c.fpr_f64(4), 1.0 + tiny, "CONTROL: fadd keeps double precision");
    let c = fpr_run(&["fmuls 1,2,3", "fdivs 4,2,3", "fsubs 5,2,3"], &[(2, 1.0), (3, 3.0)]);
    assert_eq!(c.fpr_f64(1), 3.0);
    assert_eq!(c.fpr_f64(4), (1.0f32 / 3.0f32) as f64, "single quotient, not 1/3 in double");
    assert_ne!(c.fpr_f64(4), 1.0 / 3.0);
    assert_eq!(c.fpr_f64(5), -2.0);
}

#[test]
fn fused_multiply_add_family_is_fused() {
    let c = fpr_run(&["fmadd 1,2,3,4", "fmsub 5,2,3,4", "fnmadd 6,2,3,4", "fnmsub 7,2,3,4"], &[(2, 2.0), (3, 3.0), (4, 4.0)]);
    assert_eq!((c.fpr_f64(1), c.fpr_f64(5), c.fpr_f64(6), c.fpr_f64(7)), (10.0, 2.0, -10.0, -2.0));
    // a*c = 1 + 2^-26 + 2^-54 exactly; the 2^-54 survives only if the product is not rounded first.
    let a = 1.0 + 2f64.powi(-27);
    let c = fpr_run(&["fmadd 1,2,2,4", "fmul 5,2,2", "fadd 6,5,4"], &[(2, a), (4, -(1.0 + 2f64.powi(-26)))]);
    assert_eq!(c.fpr_f64(1), 2f64.powi(-54), "fused");
    assert_eq!(c.fpr_f64(6), 0.0, "CONTROL: fmul then fadd loses the 2^-54");
    let c = fpr_run(&["fmadds 1,2,3,4", "fnmsubs 5,2,3,4"], &[(2, 2.0), (3, 3.0), (4, 4.0)]);
    assert_eq!((c.fpr_f64(1), c.fpr_f64(5)), (10.0, -2.0));
}

#[test]
fn invalid_operations_produce_the_default_nan_and_the_right_flag() {
    let inf = f64::INFINITY;
    for (line, a, b, flag) in [
        ("fsub 1,2,3", inf, inf, VXISI),
        ("fadd 1,2,3", inf, -inf, VXISI),
        ("fdiv 1,2,3", inf, inf, VXIDI),
        ("fdiv 1,2,3", 0.0, 0.0, VXZDZ),
        ("fmul 1,2,3", inf, 0.0, VXIMZ),
    ] {
        let c = fpr_run(&[line], &[(2, a), (3, b)]);
        assert_eq!(c.fpr[1], QNAN_DEFAULT, "{line} {a} {b}");
        assert_eq!(c.fpscr & (FX | VX | flag), FX | VX | flag, "{line}");
        assert_eq!(fprf(&c), QNAN);
    }
    // CONTROL: same ops on valid operands raise nothing.
    let c = fpr_run(&["fsub 1,2,3", "fadd 4,2,3", "fdiv 5,2,3", "fmul 6,2,3"], &[(2, inf), (3, 1.0)]);
    assert_eq!(c.fpscr & (FX | VX), 0);
    assert_eq!((c.fpr_f64(1), c.fpr_f64(6)), (inf, inf));
    // fmadd: inf*0 and inf - inf in the add
    let c = fpr_run(&["fmadd 1,2,3,4"], &[(2, inf), (3, 0.0), (4, 1.0)]);
    assert_eq!((c.fpr[1], c.fpscr & VXIMZ), (QNAN_DEFAULT, VXIMZ));
    let c = fpr_run(&["fmadd 1,2,3,4"], &[(2, inf), (3, 1.0), (4, -inf)]);
    assert_eq!((c.fpr[1], c.fpscr & VXISI), (QNAN_DEFAULT, VXISI));
    let c = fpr_run(&["fmsub 1,2,3,4"], &[(2, inf), (3, 1.0), (4, inf)]);
    assert_eq!(c.fpscr & VXISI, VXISI, "inf - inf through fmsub");
    let c = fpr_run(&["fmadd 1,2,3,4"], &[(2, inf), (3, 1.0), (4, inf)]);
    assert_eq!(c.fpscr & VXISI, 0, "CONTROL: inf + inf is fine");
}

#[test]
fn nan_operands_propagate_quieted_and_snan_is_flagged() {
    let c = bits_run(&["fadd 1,2,3"], &[(2, SNAN), (3, 0x3FF0_0000_0000_0000)]);
    assert_eq!(c.fpr[1], SNAN | 1 << 51, "SNaN quieted, payload kept");
    assert_eq!(c.fpscr & (FX | VX | VXSNAN), FX | VX | VXSNAN);
    let qn = 0x7FF8_0000_0000_00AB;
    let c = bits_run(&["fadd 1,2,3"], &[(2, 0x3FF0_0000_0000_0000), (3, qn)]);
    assert_eq!(c.fpr[1], qn, "QNaN passes through");
    assert_eq!(c.fpscr & (FX | VX), 0, "CONTROL: a quiet NaN is not an exception");
    let q2 = 0x7FF8_0000_0000_00CD;
    let c = bits_run(&["fadd 1,2,3"], &[(2, qn), (3, q2)]);
    assert_eq!(c.fpr[1], qn, "frA's NaN wins over frB's");
    let c = bits_run(&["fmadd 1,2,3,4"], &[(2, 0x3FF0_0000_0000_0000), (3, q2), (4, qn)]);
    assert_eq!(c.fpr[1], qn, "order frA, frB, frC: the addend's NaN beats frC's");
}

#[test]
fn division_by_zero_and_overflow_flags() {
    let c = fpr_run(&["fdiv 1,2,3"], &[(2, 1.0), (3, 0.0)]);
    assert_eq!((c.fpr_f64(1), c.fpscr & (ZX | FX), fprf(&c)), (f64::INFINITY, ZX | FX, INF_POS));
    let c = fpr_run(&["fdiv 1,2,3"], &[(2, 1.0), (3, -0.0)]);
    assert_eq!((c.fpr_f64(1), fprf(&c)), (f64::NEG_INFINITY, INF_NEG), "sign follows the zero's sign");
    let c = fpr_run(&["fdiv 1,2,3"], &[(2, f64::INFINITY), (3, 0.0)]);
    assert_eq!(c.fpscr & ZX, 0, "CONTROL: inf/0 is exact infinity, not a divide-by-zero");
    let c = fpr_run(&["fmul 1,2,3"], &[(2, 1e308), (3, 10.0)]);
    assert_eq!((c.fpr_f64(1), c.fpscr & OX), (f64::INFINITY, OX));
    let c = fpr_run(&["fmul 1,2,3"], &[(2, 1e308), (3, 1.5)]);
    assert_eq!(c.fpscr & OX, 0, "CONTROL: 1.5e308 fits");
    let c = fpr_run(&["fadds 1,2,3"], &[(2, 3.0e38), (3, 3.0e38)]);
    assert_eq!((c.fpr_f64(1), c.fpscr & OX), (f64::INFINITY, OX), "6e38 overflows single");
    let c = fpr_run(&["fadd 1,2,3"], &[(2, 3.0e38), (3, 3.0e38)]);
    assert_eq!(c.fpscr & OX, 0, "CONTROL: the same sum is fine in double");
    // sticky: FX stays set after a later clean op
    let c = fpr_run(&["fdiv 1,2,3", "fadd 4,5,5"], &[(2, 1.0), (3, 0.0), (5, 1.0)]);
    assert_eq!(c.fpscr & (ZX | FX), ZX | FX);
}

#[test]
fn sqrt_reciprocal_estimates() {
    let c = fpr_run(&["fsqrt 1,2", "fres 3,4", "frsqrte 5,4"], &[(2, 9.0), (4, 4.0)]);
    assert_eq!((c.fpr_f64(1), c.fpr_f64(3), c.fpr_f64(5)), (3.0, 0.25, 0.5));
    let c = fpr_run(&["fsqrt 1,2"], &[(2, -1.0)]);
    assert_eq!((c.fpr[1], c.fpscr & VXSQRT), (QNAN_DEFAULT, VXSQRT));
    let c = fpr_run(&["fsqrt 1,2"], &[(2, -0.0)]);
    assert_eq!((c.fpr[1], c.fpscr & VXSQRT), (1 << 63, 0), "CONTROL: sqrt(-0) = -0, no exception");
    let c = fpr_run(&["fres 1,2"], &[(2, 0.0)]);
    assert_eq!((c.fpr_f64(1), c.fpscr & ZX), (f64::INFINITY, ZX));
}

#[test]
fn frsp_and_single_denormal_class() {
    let c = fpr_run(&["frsp 1,2"], &[(2, 1.0 + 2f64.powi(-30))]);
    assert_eq!(c.fpr_f64(1), 1.0);
    let c = fpr_run(&["frsp 1,2"], &[(2, 1e300)]);
    assert_eq!((c.fpr_f64(1), c.fpscr & OX), (f64::INFINITY, OX));
    let c = fpr_run(&["frsp 1,2"], &[(2, 1e-39)]);
    assert_eq!(fprf(&c), DEN_POS, "1e-39 is denormal as a single");
    assert_eq!(c.fpr_f64(1), (1e-39f32) as f64);
    let c = fpr_run(&["fadd 1,2,3"], &[(2, 1e-39), (3, 0.0)]);
    assert_eq!(fprf(&c), NORM_POS, "CONTROL: the same value is a normal double");
}

fn low(c: &Cpu, r: usize) -> u32 { c.fpr[r] as u32 }

#[test]
fn convert_to_integer_honours_the_rounding_mode_for_fctiw_only() {
    let c = fpr_run(&["fctiw 1,2", "fctiwz 3,2", "fctiw 4,5", "fctiw 6,7", "fctiwz 8,9"], &[(2, 2.5), (5, 3.5), (7, -2.5), (9, -2.9)]);
    assert_eq!((low(&c, 1), low(&c, 3)), (2, 2), "2.5 -> nearest-even 2 / toward zero 2");
    assert_eq!(low(&c, 4), 4, "3.5 -> 4");
    assert_eq!(low(&c, 6), (-2i32) as u32, "-2.5 -> -2");
    assert_eq!(low(&c, 8), (-2i32) as u32, "fctiwz truncates -2.9 to -2");
    assert_eq!(c.fpscr & XX, XX, "inexact");
    // RN = 2 (toward +inf) and 3 (toward -inf), set through mtfsfi on field 7.
    let c = fpr_run(&["mtfsfi 7,2", "fctiw 1,2", "fctiw 3,4"], &[(2, 2.1), (4, -2.9)]);
    assert_eq!((low(&c, 1), low(&c, 3)), (3, (-2i32) as u32));
    let c = fpr_run(&["mtfsfi 7,3", "fctiw 1,2", "fctiw 3,4"], &[(2, 2.9), (4, -2.1)]);
    assert_eq!((low(&c, 1), low(&c, 3)), (2, (-3i32) as u32));
    let c = fpr_run(&["mtfsfi 7,3", "fctiwz 1,2"], &[(2, 2.9)]);
    assert_eq!(low(&c, 1), 2, "CONTROL: fctiwz ignores RN");
    // exact integers raise nothing
    let c = fpr_run(&["fctiw 1,2"], &[(2, 7.0)]);
    assert_eq!((low(&c, 1), c.fpscr & XX), (7, 0));
}

#[test]
fn convert_to_integer_saturates_and_flags_invalid() {
    let c = fpr_run(&["fctiw 1,2", "fctiw 3,4"], &[(2, 1e10), (4, -1e10)]);
    assert_eq!((low(&c, 1), low(&c, 3)), (0x7FFF_FFFF, 0x8000_0000));
    assert_eq!(c.fpscr & (VX | VXCVI), VX | VXCVI);
    let c = fpr_run(&["fctiw 1,2"], &[(2, f64::NAN)]);
    assert_eq!((low(&c, 1), c.fpscr & VXCVI), (0x8000_0000, VXCVI));
    let c = fpr_run(&["fctiw 1,2"], &[(2, 2147483647.0)]);
    assert_eq!((low(&c, 1), c.fpscr & VXCVI), (0x7FFF_FFFF, 0), "CONTROL: INT_MAX is representable");
    let c = fpr_run(&["fctiw 1,2"], &[(2, 2147483647.5)]);
    assert_eq!(c.fpscr & VXCVI, VXCVI, "2147483647.5 rounds to 2^31: out of range");
}

#[test]
fn compares_set_cr_and_fpcc() {
    let c = fpr_run(&["fcmpu 1,2,3", "fcmpu 2,3,2", "fcmpu 3,2,2"], &[(2, 1.0), (3, 2.0)]);
    assert_eq!((c.cr_field(1), c.cr_field(2), c.cr_field(3)), (8, 4, 2));
    assert_eq!(c.fpscr >> 12 & 0xF, 2, "FPCC mirrors the last compare");
    let c = bits_run(&["fcmpu 1,2,3"], &[(2, 0x7FF8_0000_0000_0000), (3, 0)]);
    assert_eq!((c.cr_field(1), c.fpscr & (VXSNAN | VXVC)), (1, 0), "QNaN: unordered, no exception for fcmpu");
    let c = bits_run(&["fcmpo 1,2,3"], &[(2, 0x7FF8_0000_0000_0000), (3, 0)]);
    assert_eq!((c.cr_field(1), c.fpscr & (VXSNAN | VXVC)), (1, VXVC), "fcmpo flags VXVC for a QNaN");
    let c = bits_run(&["fcmpu 1,2,3"], &[(2, SNAN), (3, 0)]);
    assert_eq!((c.cr_field(1), c.fpscr & (VXSNAN | VXVC)), (1, VXSNAN));
    let c = fpr_run(&["fcmpo 1,2,3"], &[(2, 1.0), (3, 2.0)]);
    assert_eq!(c.fpscr & (VXVC | FX), 0, "CONTROL: ordered operands raise nothing");
    let c = fpr_run(&["fcmpu 1,2,3"], &[(2, -0.0), (3, 0.0)]);
    assert_eq!(c.cr_field(1), 2, "-0 == +0");
}

#[test]
fn fsel_and_the_bit_moves() {
    let (a, b) = (0x1111_1111_1111_1111u64, 0x2222_2222_2222_2222u64);
    let c = bits_run(&["fsel 1,2,3,4"], &[(2, 0.0f64.to_bits()), (3, a), (4, b)]);
    assert_eq!(c.fpr[1], a, "frA = +0 -> frC");
    let c = bits_run(&["fsel 1,2,3,4"], &[(2, (-0.0f64).to_bits()), (3, a), (4, b)]);
    assert_eq!(c.fpr[1], a, "frA = -0 counts as >= 0");
    let c = bits_run(&["fsel 1,2,3,4"], &[(2, (-1.0f64).to_bits()), (3, a), (4, b)]);
    assert_eq!(c.fpr[1], b, "negative -> frB");
    let c = bits_run(&["fsel 1,2,3,4"], &[(2, 0x7FF8_0000_0000_0000), (3, a), (4, b)]);
    assert_eq!(c.fpr[1], b, "NaN -> frB");
    let n = 0x7FF8_0000_0000_00AB;
    let c = bits_run(&["fmr 1,2", "fneg 3,2", "fabs 4,5", "fnabs 6,2"], &[(2, n), (5, n | 1 << 63)]);
    assert_eq!((c.fpr[1], c.fpr[3], c.fpr[4], c.fpr[6]), (n, n | 1 << 63, n, n | 1 << 63));
    assert_eq!(c.fpscr, 0, "moves never touch FPSCR (CONTROL: even with a NaN)");
}

#[test]
fn record_form_copies_fpscr_summary_to_cr1() {
    let c = fpr_run(&["fmul. 1,2,3"], &[(2, 1e308), (3, 10.0)]);
    assert_eq!(c.cr_field(1), 0b1001, "FX and OX");
    let c = fpr_run(&["fmul. 1,2,3"], &[(2, 1.0), (3, 10.0)]);
    assert_eq!(c.cr_field(1), 0, "CONTROL");
    let c = fpr_run(&["fmr. 1,2"], &[(2, 1.0)]);
    assert_eq!(c.cr_field(1), 0);
}

#[test]
fn fpscr_moves_and_sticky_clearing() {
    let c = fpr_run(&["mtfsb1 3", "mffs 1", "mcrfs 0,0", "mffs 2"], &[]);
    assert_eq!(c.fpr[1] as u32, FX | OX, "setting OX by hand also raises FX");
    assert_eq!(c.fpr[1] >> 32, 0xFFF8_0000, "high word is the documented undefined pattern");
    assert_eq!(c.cr_field(0), 0b1001, "mcrfs copies FX FEX VX OX to CR0");
    assert_eq!(c.fpr[2] as u32, 0, "and clears them in FPSCR");
    let c = fpr_run(&["mtfsb1 3", "mtfsb0 3", "mffs 1"], &[]);
    assert_eq!(c.fpr[1] as u32, FX, "mtfsb0 clears OX but FX stays until cleared itself");
    let c = fpr_run(&["mtfsf 0xFF,2", "mffs 1"], &[(2, f64::from_bits(0x0000_0000_0000_0003))]);
    assert_eq!(c.fpr[1] as u32, 3, "RN = 3 written");
    let c = fpr_run(&["mtfsf 0x01,2", "mffs 1"], &[(2, f64::from_bits(0xFFFF_FFFF_FFFF_FFF3))]);
    assert_eq!(c.fpr[1] as u32, 3, "only field 7 selected");
    let c = fpr_run(&["mtfsf 0xFF,2", "mffs 1"], &[(2, f64::from_bits(0x6000_0000))]);
    assert_eq!(c.fpr[1] as u32, 0, "FEX and VX are derived: they cannot be written directly");
}

#[test]
fn enabled_exceptions_set_fex_and_ve_suppresses_the_write() {
    let c = fpr_run(&["mtfsb1 27", "fdiv 1,2,3", "mffs 4"], &[(2, 1.0), (3, 0.0)]);
    assert_eq!(c.fpr[4] as u32 & (ZX | FEX | FX), ZX | FEX | FX, "ZE enabled: FEX set");
    let c = fpr_run(&["fdiv 1,2,3", "mffs 4"], &[(2, 1.0), (3, 0.0)]);
    assert_eq!(c.fpr[4] as u32 & FEX, 0, "CONTROL: ZE disabled, no FEX");
    // VE: an invalid operation leaves frD untouched
    let c = fpr_run(&["mtfsb1 24", "fsub 1,2,3"], &[(1, 7.0), (2, f64::INFINITY), (3, f64::INFINITY)]);
    assert_eq!(c.fpr_f64(1), 7.0);
    assert_eq!(c.fpscr & (VXISI | FEX), VXISI | FEX);
    let c = fpr_run(&["fsub 1,2,3"], &[(1, 7.0), (2, f64::INFINITY), (3, f64::INFINITY)]);
    assert_eq!(c.fpr[1], QNAN_DEFAULT, "CONTROL: VE clear writes the default NaN");
}

#[test]
fn arithmetic_under_a_directed_rounding_mode_is_a_named_trap() {
    let (_, _, st) = prog(&["mtfsfi 7,1", "fadd 1,2,3"], |_, _| {});
    match st.trap {
        Some(retro_ppc::Trap::Unimplemented { what, .. }) => assert!(what.contains("rounding mode"), "{what}"),
        t => panic!("{t:?}"),
    }
    // CONTROL: RN=0 runs.
    let (_, _, st) = prog(&["mtfsfi 7,0", "fadd 1,2,3"], |_, _| {});
    assert!(st.trap.is_none());
}

#[test]
fn fp_loads_and_stores_convert_formats() {
    let m = 0x4000;
    let (c, v, st) = prog(&["lfs 1,0(3)", "lfd 2,8(3)", "stfs 1,0x20(3)", "stfd 2,0x28(3)", "stfiwx 2,0,4", "lfsu 5,0x20(3)", "lfdx 6,3,7"], |c, v| {
        v.mem.write_u32(m, 0x3FC0_0000).unwrap(); // 1.5f
        v.mem.write_bytes(m + 8, &0x4009_21FB_5444_2D18u64.to_be_bytes()).unwrap(); // pi
        c.gpr[3] = m; c.gpr[4] = m + 0x40; c.gpr[7] = 8;
    });
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!(c.fpr_f64(1), 1.5);
    assert_eq!(c.fpr[2], 0x4009_21FB_5444_2D18);
    assert_eq!(v.mem.read_u32(m + 0x20).unwrap(), 0x3FC0_0000);
    assert_eq!(v.mem.read_u32(m + 0x28).unwrap(), 0x4009_21FB, "stfd high word first");
    assert_eq!(v.mem.read_u32(m + 0x2C).unwrap(), 0x5444_2D18);
    assert_eq!(v.mem.read_u32(m + 0x40).unwrap(), 0x5444_2D18, "stfiwx stores the low word of the FPR");
    assert_eq!(c.fpr_f64(5), 1.5);
    assert_eq!(c.gpr[3], m + 0x20, "lfsu updated RA");
    assert_eq!(c.fpr[6], 0x4009_21FB_5444_2D18, "lfdx");
    // stfs of 1+2^-30 stores 1.0f (the PEM store conversion truncates; see oracle_fixes.rs); lfs of a single denormal is exact.
    let (c, v, _) = prog(&["stfs 1,0(3)", "lfs 2,4(3)"], |c, v| { c.set_fpr_f64(1, 1.0 + 2f64.powi(-30)); c.gpr[3] = m; v.mem.write_u32(m + 4, 1).unwrap(); });
    assert_eq!(v.mem.read_u32(m).unwrap(), 0x3F80_0000, "rounded to 1.0f");
    assert_eq!(c.fpr_f64(2), 2f64.powi(-149), "smallest single denormal widens exactly");
    // update form with RA=0 is refused
    let (_, _, st) = prog(&["lfsu 1,0(0)"], |_, _| {});
    assert!(matches!(st.trap, Some(retro_ppc::Trap::InvalidForm { .. })), "{st:?}");
}
