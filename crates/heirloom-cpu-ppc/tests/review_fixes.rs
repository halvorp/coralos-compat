//! Regressions for the adversarial review: single-precision fused multiply-add rounds once, mtfsf/mtfsfi copy FX
//! and the sticky bits as given, tlbsync is privileged, lfs/stfs keep signalling NaNs, and the reserved or invalid
//! integer forms are refused by name. Every test has a control that must NOT trap or must keep the old behaviour.
mod common;
use common::*;
use heirloom_cpu_ppc::{Cpu, Trap};

const FMADDS_1_2_3_4: u32 = 0xec2220fa; // frD=1 frA=2 frC=3 frB=4
const MTFSF_80_2: u32 = 0xfd00158e;
const MTFSFI_0_1: u32 = 0xfc00110c;

fn fp(word: u32, regs: &[(usize, u64)], fpscr: u32) -> Cpu {
    let (c, _, st) = run_words(&[word], |c, _| { c.fpscr = fpscr; for &(r, v) in regs { c.fpr[r] = v } });
    assert!(st.halted && st.trap.is_none(), "{st:?}");
    c
}
fn trap_of(word: u32, gpr: &[(usize, u32)]) -> Option<Trap> {
    let (_, _, st) = run_words(&[word], |c, _| for &(r, v) in gpr { c.gpr[r] = v });
    st.trap
}

#[test]
fn fmadds_rounds_once_where_double_rounding_is_one_ulp_wrong() {
    // Witness from the review: a = 2^-24 (1 + 2^-23), c = -(1 - 2^-23), b = 1 + 2^-23, all exact singles. The exact
    // sum is 1 + 2^-24 + 2^-70, just above the single midpoint, so the single result is 0x3f800001. Rounding to
    // f64 first makes it an exact tie, which then rounds to even (0x3f800000).
    let a = 2f64.powi(-24) * (1.0 + 2f64.powi(-23));
    let c = -(1.0 - 2f64.powi(-23));
    let b = 1.0 + 2f64.powi(-23);
    let c1 = fp(FMADDS_1_2_3_4, &[(2, a.to_bits()), (3, c.to_bits()), (4, b.to_bits())], 0);
    assert_eq!(c1.fpr_f64(1) as f32, f32::from_bits(0x3f80_0001), "single rounding");
    // CONTROL: the double-precision fmadd of the same operands is the f64 result (the tie is real in f64).
    let fmadd = 0xfc2220fa; // fmadd 1,2,3,4
    let c2 = fp(fmadd, &[(2, a.to_bits()), (3, c.to_bits()), (4, b.to_bits())], 0);
    assert_eq!(c2.fpr_f64(1), 1.0 + 2f64.powi(-24));
    // CONTROL: an exact case and the negated forms keep working.
    let c3 = fp(FMADDS_1_2_3_4, &[(2, 2f64.to_bits()), (3, 3f64.to_bits()), (4, 4f64.to_bits())], 0);
    assert_eq!(c3.fpr_f64(1), 10.0);
    let fnmadds = 0xec2220fe;
    let c4 = fp(fnmadds, &[(2, a.to_bits()), (3, c.to_bits()), (4, b.to_bits())], 0);
    assert_eq!(c4.fpr_f64(1) as f32, -f32::from_bits(0x3f80_0001));
}

#[test]
fn mtfsf_and_mtfsfi_copy_fx_and_sticky_bits_exactly() {
    // frB low word = OX only (FX=0): FPSCR must be exactly OX, not OX|FX.
    let c = fp(MTFSF_80_2, &[(2, 0x1000_0000)], 0);
    assert_eq!(c.fpscr, 0x1000_0000);
    let c = fp(MTFSFI_0_1, &[], 0);
    assert_eq!(c.fpscr, 0x1000_0000);
    // And FX written as 1 stays 1.
    let c = fp(MTFSF_80_2, &[(2, 0x8000_0000)], 0);
    assert_eq!(c.fpscr, 0x8000_0000);
    // CONTROL: mtfsb1 (a hardware-style set) DOES raise FX when OX rises: mtfsb1 3 = 63<<26|3<<21|38<<1.
    let c = fp(63 << 26 | 3 << 21 | 38 << 1, &[], 0);
    assert_eq!(c.fpscr, 0x9000_0000);
}

#[test]
fn tlbsync_is_privileged_but_user_hints_still_run() {
    assert!(matches!(trap_of(31 << 26 | 566 << 1, &[]), Some(Trap::Privileged { what: "tlbsync", .. })));
    assert_eq!(trap_of(31 << 26 | 598 << 1, &[]), None, "CONTROL: sync");
}

#[test]
fn lfs_and_stfs_do_not_quiet_signalling_nans() {
    let lfs = 0xc0230000u32; // lfs 1,0(3)
    let (c, _, st) = run_words(&[lfs], |c, v| { c.gpr[3] = 0x4000; v.mem.write_u32(0x4000, 0x7F80_0001).unwrap(); });
    assert!(st.trap.is_none());
    assert_eq!(c.fpr[1], 0x7ff0_0000_2000_0000, "SNaN payload and clear quiet bit kept");
    let (c, _, _) = run_words(&[lfs], |c, v| { c.gpr[3] = 0x4000; v.mem.write_u32(0x4000, 0xFFC0_0001).unwrap(); });
    assert_eq!(c.fpr[1], 0xfff8_0000_2000_0000, "QNaN and sign kept");
    // CONTROL: ordinary values, denormal singles and infinity still widen exactly.
    for (bits, want) in [(0x3f80_0000u32, 1.0f64), (0x0000_0001, f32::from_bits(1) as f64), (0x7f80_0000, f64::INFINITY)] {
        let (c, _, _) = run_words(&[lfs], |c, v| { c.gpr[3] = 0x4000; v.mem.write_u32(0x4000, bits).unwrap(); });
        assert_eq!(c.fpr_f64(1), want);
    }
    let stfs = 0xd0230000u32; // stfs 1,0(3)
    let (_, v, _) = run_words(&[stfs], |c, _| { c.gpr[3] = 0x4000; c.fpr[1] = 0x7FF0_0000_2000_0000; });
    assert_eq!(v.mem.read_u32(0x4000).unwrap(), 0x7F80_0001, "SNaN stays signalling");
    let (_, v, _) = run_words(&[stfs], |c, _| { c.gpr[3] = 0x4000; c.fpr[1] = 1.5f64.to_bits(); });
    assert_eq!(v.mem.read_u32(0x4000).unwrap(), 1.5f32.to_bits(), "CONTROL: plain value");
}

#[test]
fn invalid_forms_are_refused_by_name() {
    let invalid = |t: Option<Trap>| matches!(t, Some(Trap::InvalidForm { .. }));
    assert!(invalid(trap_of(46 << 26, &[])), "lmw r0,0(0): RA=0 is in the range for RT=0");
    assert!(!invalid(trap_of(46 << 26 | 3 << 21 | 0x4000, &[])), "CONTROL: lmw r3,0x4000(0) is fine");
    // lswi r5,r6,8 loads r5,r6: RA=r6 is overwritten.
    assert!(invalid(trap_of(0x7CA644AA, &[(6, 0x4000)])));
    assert_eq!(trap_of(0x7CA744AA, &[(7, 0x4000)]), None, "CONTROL: RA=r7 is outside r5..r6");
    // lswx r5,r6,r7 with XER count 8: RB=r7 is outside, RA=r6 inside; and RB inside via lswx r5,r8,r6.
    let lswx = |rt: u32, ra: u32, rb: u32| 31 << 26 | rt << 21 | ra << 16 | rb << 11 | 533 << 1;
    let (_, _, st) = run_words(&[lswx(5, 8, 6)], |c, _| { c.xer = 8; c.gpr[8] = 0x4000; c.gpr[6] = 0; });
    assert!(invalid(st.trap), "RB inside the range");
    let (_, _, st) = run_words(&[lswx(5, 8, 9)], |c, _| { c.xer = 8; c.gpr[8] = 0x4000; c.gpr[9] = 0; });
    assert_eq!(st.trap, None, "CONTROL");
    let (_, _, st) = run_words(&[lswx(5, 6, 9)], |c, _| { c.xer = 0; c.gpr[6] = 0x4000; });
    assert_eq!(st.trap, None, "CONTROL: a zero-length lswx loads nothing, so nothing overlaps");
    // mulhw / mulhwu with the reserved OE bit.
    assert!(invalid(trap_of(0x7ca32496, &[])), "mulhw with OE");
    assert!(invalid(trap_of(0x7ca32416, &[])), "mulhwu with OE");
    assert_eq!(trap_of(0x7ca32096, &[]), None, "CONTROL: mulhw");
}
