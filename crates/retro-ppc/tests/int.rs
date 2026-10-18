//! Integer instruction families with exact CR / XER results. Expected values are hand-computed from the
//! PowerPC Programming Environments Manual (PEM) instruction pages (add, subf, mullw, divw, sraw, rlwinm, ...)
//! and each family has a negative control (the neighbouring case that must NOT behave the same).
mod common;
use common::*;
use retro_ppc::{XER_CA, XER_OV, XER_SO};

const LT: u32 = 8;
const GT: u32 = 4;
const EQ: u32 = 2;
const SO: u32 = 1;

#[test]
fn addi_addis_treat_ra0_as_literal_zero() {
    let c = go(&["li 3,-1", "lis 4,0x1234", "addi 5,0,7", "addi 6,6,5", "addis 7,0,-1"], &[(0, 99), (6, 10)]);
    assert_eq!((c.gpr[3], c.gpr[4], c.gpr[5], c.gpr[6], c.gpr[7]), (0xFFFF_FFFF, 0x1234_0000, 7, 15, 0xFFFF_0000));
    // CONTROL: `add` (register form) with RA=r0 uses the register, here 99.
    let c = go(&["add 5,0,6"], &[(0, 99), (6, 1)]);
    assert_eq!(c.gpr[5], 100);
}

#[test]
fn carry_chain_addc_adde_makes_a_64_bit_add() {
    // 0x00000000_FFFFFFFF + 0x00000000_00000001 = 0x00000001_00000000
    let c = go(&["addc 5,3,4", "adde 6,7,8"], &[(3, 0xFFFF_FFFF), (4, 1), (7, 0), (8, 0)]);
    assert_eq!((c.gpr[5], c.gpr[6]), (0, 1));
    assert_eq!(c.xer & XER_CA, 0, "the high add produced no carry");
    // CONTROL: no carry out of the low word -> the high word stays 0.
    let c = go(&["addc 5,3,4", "adde 6,7,8"], &[(3, 1), (4, 1), (7, 0), (8, 0)]);
    assert_eq!((c.gpr[5], c.gpr[6]), (2, 0));
}

#[test]
fn overflow_summary_is_sticky_and_cr0_reads_it() {
    let c = go(&["addo. 5,3,4", "addo. 7,8,9"], &[(3, 0x7FFF_FFFF), (4, 1), (8, 1), (9, 2)]);
    assert_eq!(c.gpr[5], 0x8000_0000);
    assert_eq!(c.xer & (XER_OV | XER_SO), XER_SO, "OV cleared by the second add; SO is sticky");
    assert_eq!(c.cr_field(0), GT | SO, "second add: positive result, SO copied from XER");
    // first add alone: OV and SO both set, CR0 = LT|SO
    let c = go(&["addo. 5,3,4"], &[(3, 0x7FFF_FFFF), (4, 1)]);
    assert_eq!((c.xer & XER_OV != 0, c.xer & XER_SO != 0, c.cr_field(0)), (true, true, LT | SO));
    // CONTROL: without the o suffix OV is never touched, with it a non-overflowing add reports OV=0.
    let c = go(&["add. 5,3,4"], &[(3, 0x7FFF_FFFF), (4, 1)]);
    assert_eq!(c.xer & (XER_OV | XER_SO), 0);
    assert_eq!(c.cr_field(0), LT);
}

#[test]
fn subtraction_is_rb_minus_ra_with_borrow_as_not_carry() {
    assert_eq!(go(&["subf 5,3,4"], &[(3, 3), (4, 10)]).gpr[5], 7);
    let c = go(&["subfc 5,3,4"], &[(3, 5), (4, 3)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFE, 0), "3-5 borrows: CA=0");
    let c = go(&["subfc 5,3,4"], &[(3, 3), (4, 5)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (2, XER_CA), "CONTROL 5-3 does not borrow: CA=1");
    let c = go(&["subfic 5,3,0"], &[(3, 1)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFF, 0));
    let c = go(&["subfic 5,3,0"], &[(3, 0)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0, XER_CA));
}

#[test]
fn extended_arithmetic_uses_the_incoming_carry() {
    let c = gox(&["adde 5,3,4"], &[(3, 1), (4, 1)], XER_CA);
    assert_eq!(c.gpr[5], 3);
    let c = gox(&["adde 5,3,4"], &[(3, 1), (4, 1)], 0);
    assert_eq!(c.gpr[5], 2, "CONTROL: same operands without CA");
    let c = gox(&["addze 5,3"], &[(3, 0xFFFF_FFFF)], XER_CA);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0, XER_CA));
    let c = gox(&["addme 5,3"], &[(3, 0)], XER_CA); // 0 + (-1) + 1 = 0, carry out
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0, XER_CA));
    let c = gox(&["subfze 5,3"], &[(3, 5)], XER_CA); // ~5 + 0 + 1 = -5
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFB, 0));
    let c = gox(&["subfme 5,3"], &[(3, 5)], XER_CA); // ~5 + (-1) + 1 = ~5 = -6
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFA, XER_CA));
    let c = gox(&["subfe 5,3,4"], &[(3, 1), (4, 5)], XER_CA); // ~1 + 5 + 1 = 5-1 = 4
    assert_eq!((c.gpr[5], c.xer & XER_CA), (4, XER_CA));
}

#[test]
fn addic_sets_carry_and_dot_form_sets_cr0() {
    let c = go(&["addic 4,3,1"], &[(3, 0xFFFF_FFFF)]);
    assert_eq!((c.gpr[4], c.xer & XER_CA), (0, XER_CA));
    let c = go(&["addic. 4,3,-1"], &[(3, 0)]);
    assert_eq!((c.gpr[4], c.xer & XER_CA, c.cr_field(0)), (0xFFFF_FFFF, 0, LT));
    // CONTROL: plain addi never touches CA.
    let c = go(&["addi 4,3,1"], &[(3, 0xFFFF_FFFF)]);
    assert_eq!((c.gpr[4], c.xer & XER_CA), (0, 0));
}

#[test]
fn neg_and_its_overflow_case() {
    assert_eq!(go(&["neg 4,3"], &[(3, 1)]).gpr[4], 0xFFFF_FFFF);
    assert_eq!(go(&["neg 4,3"], &[(3, 0)]).gpr[4], 0);
    let c = go(&["nego 4,3"], &[(3, 0x8000_0000)]);
    assert_eq!((c.gpr[4], c.xer & XER_OV != 0), (0x8000_0000, true));
    let c = go(&["nego 4,3"], &[(3, 5)]);
    assert!(c.xer & XER_OV == 0, "CONTROL: -5 does not overflow");
}

#[test]
fn multiplies() {
    let c = go(&["mullwo 5,3,4", "mulhwu 6,3,4"], &[(3, 0x10000), (4, 0x10000)]);
    assert_eq!((c.gpr[5], c.gpr[6], c.xer & XER_OV != 0), (0, 1, true));
    let c = go(&["mullwo 5,3,4"], &[(3, 0x7FFF), (4, 0x10000)]);
    assert!(c.xer & XER_OV == 0, "CONTROL: 0x7FFF0000 fits");
    assert_eq!(go(&["mulhw 5,3,4"], &[(3, 0xFFFF_FFFF), (4, 0xFFFF_FFFF)]).gpr[5], 0, "(-1)*(-1)=1, high word 0");
    assert_eq!(go(&["mulhw 5,3,4"], &[(3, 0xFFFF_FFFF), (4, 1)]).gpr[5], 0xFFFF_FFFF, "-1*1, high word is the sign");
    assert_eq!(go(&["mulhwu 5,3,4"], &[(3, 0xFFFF_FFFF), (4, 0xFFFF_FFFF)]).gpr[5], 0xFFFF_FFFE);
    assert_eq!(go(&["mulli 5,3,-3"], &[(3, 7)]).gpr[5], (-21i32) as u32);
}

#[test]
fn divides_truncate_toward_zero_and_flag_division_by_zero() {
    assert_eq!(go(&["divw 5,3,4"], &[(3, (-7i32) as u32), (4, 2)]).gpr[5], (-3i32) as u32);
    assert_eq!(go(&["divwu 5,3,4"], &[(3, 7), (4, 2)]).gpr[5], 3);
    assert_eq!(go(&["divwu 5,3,4"], &[(3, 0xFFFF_FFFE), (4, 2)]).gpr[5], 0x7FFF_FFFF, "unsigned");
    let c = go(&["divwo 5,3,4"], &[(3, 5), (4, 0)]);
    assert!(c.xer & XER_OV != 0, "divide by zero sets OV");
    let c = go(&["divwo 5,3,4"], &[(3, 0x8000_0000), (4, 0xFFFF_FFFF)]);
    assert!(c.xer & XER_OV != 0, "INT_MIN / -1 overflows");
    let c = go(&["divwo 5,3,4"], &[(3, 6), (4, 3)]);
    assert!(c.xer & XER_OV == 0, "CONTROL: 6/3 is fine");
}

#[test]
fn logical_ops_and_immediates() {
    let (a, b) = (0xF0F0_1234u32, 0x0FF0_00FFu32);
    let r = |l: &str| go(&[l], &[(3, a), (4, b)]).gpr[5];
    assert_eq!(r("and 5,3,4"), a & b);
    assert_eq!(r("or 5,3,4"), a | b);
    assert_eq!(r("xor 5,3,4"), a ^ b);
    assert_eq!(r("nand 5,3,4"), !(a & b));
    assert_eq!(r("nor 5,3,4"), !(a | b));
    assert_eq!(r("eqv 5,3,4"), !(a ^ b));
    assert_eq!(r("andc 5,3,4"), a & !b);
    assert_eq!(r("orc 5,3,4"), a | !b);
    let c = go(&["ori 5,3,0x00FF", "oris 6,3,0x0F0F", "xori 7,3,0xFFFF", "xoris 8,3,0xFFFF"], &[(3, a)]);
    assert_eq!((c.gpr[5], c.gpr[6], c.gpr[7], c.gpr[8]), (a | 0xFF, a | 0x0F0F_0000, a ^ 0xFFFF, a ^ 0xFFFF_0000));
    // andi. / andis. always record CR0; ori does not.
    let c = go(&["andi. 5,3,0x0F00"], &[(3, 0x0000_F0F0)]);
    assert_eq!((c.gpr[5], c.cr_field(0)), (0, EQ));
    let c = go(&["andis. 5,3,0x8000"], &[(3, 0x8000_0000)]);
    assert_eq!((c.gpr[5], c.cr_field(0)), (0x8000_0000, LT));
    let c = go(&["ori 5,3,0"], &[(3, 0)]);
    assert_eq!(c.cr_field(0), 0, "CONTROL: ori leaves CR0 alone");
    let c = go(&["or. 5,3,4"], &[(3, 0), (4, 0)]);
    assert_eq!(c.cr_field(0), EQ);
}

#[test]
fn sign_extension_and_count_leading_zeros() {
    let c = go(&["extsb 5,3", "extsh 6,3", "cntlzw 7,3"], &[(3, 0x0000_80FF)]);
    assert_eq!((c.gpr[5], c.gpr[6], c.gpr[7]), (0xFFFF_FFFF, 0xFFFF_80FF, 16));
    assert_eq!(go(&["cntlzw 5,3"], &[(3, 0)]).gpr[5], 32);
    assert_eq!(go(&["cntlzw 5,3"], &[(3, 1)]).gpr[5], 31);
    assert_eq!(go(&["cntlzw 5,3"], &[(3, 0x8000_0000)]).gpr[5], 0);
}

#[test]
fn shifts_use_six_bit_counts() {
    let r = |l: &str, s: u32, n: u32| go(&[l], &[(3, s), (4, n)]).gpr[5];
    assert_eq!(r("slw 5,3,4", 0x0000_00F1, 4), 0x0000_0F10);
    assert_eq!(r("slw 5,3,4", 1, 31), 0x8000_0000);
    assert_eq!(r("slw 5,3,4", 1, 32), 0, "count 32 shifts everything out");
    assert_eq!(r("slw 5,3,4", 1, 63), 0, "count uses 6 bits: 63 >= 32");
    assert_eq!(r("slw 5,3,4", 1, 64), 1, "CONTROL: 64 & 63 == 0, no shift");
    assert_eq!(r("srw 5,3,4", 0x8000_0000, 31), 1);
    assert_eq!(r("srw 5,3,4", 0x8000_0000, 32), 0);
}

#[test]
fn algebraic_right_shift_carry_means_ones_shifted_out_of_a_negative() {
    let c = go(&["sraw 5,3,4"], &[(3, 0xFFFF_FFF5), (4, 2)]); // -11 >> 2 = -3, bits 01 shifted out
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFD, XER_CA));
    let c = go(&["sraw 5,3,4"], &[(3, 0xFFFF_FFF4), (4, 2)]); // -12 >> 2 = -3, only zeros out
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFD, 0), "CONTROL: nothing lost");
    let c = go(&["sraw 5,3,4"], &[(3, 0x7FFF_FFFF), (4, 4)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0x07FF_FFFF, 0), "positive never sets CA");
    let c = go(&["sraw 5,3,4"], &[(3, 0x8000_0000), (4, 40)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFF, XER_CA), "count >= 32 on a negative");
    let c = go(&["sraw 5,3,4"], &[(3, 0x8000_0000), (4, 0)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0x8000_0000, 0), "count 0 leaves CA clear");
    let c = go(&["srawi 5,3,1"], &[(3, 0xFFFF_FFFF)]);
    assert_eq!((c.gpr[5], c.xer & XER_CA), (0xFFFF_FFFF, XER_CA));
    let c = go(&["srawi. 5,3,4"], &[(3, 0x1000_0000)]);
    assert_eq!((c.gpr[5], c.cr_field(0)), (0x0100_0000, GT));
}

#[test]
fn rotate_and_mask_instructions() {
    // rlwinm: rotate left then AND with MASK(mb,me); wrapping masks when mb > me.
    assert_eq!(go(&["rlwinm 4,3,8,24,31"], &[(3, 0x1234_5678)]).gpr[4], 0x12, "extract the top byte");
    assert_eq!(go(&["rlwinm 4,3,0,28,3"], &[(3, 0xFFFF_FFFF)]).gpr[4], 0xF000_000F, "wrapping mask");
    assert_eq!(go(&["rlwinm 4,3,0,0,31"], &[(3, 0xFFFF_FFFF)]).gpr[4], 0xFFFF_FFFF);
    assert_eq!(go(&["rlwinm 4,3,4,0,27"], &[(3, 0x1234_5678)]).gpr[4], 0x2345_6780, "slwi 4");
    assert_eq!(go(&["rlwinm 4,3,28,4,31"], &[(3, 0x1234_5678)]).gpr[4], 0x0123_4567, "srwi 4");
    // rlwimi inserts under the mask and keeps rA elsewhere.
    let c = go(&["rlwimi 4,3,16,8,23"], &[(3, 0x1122_3344), (4, 0xAABB_CCDD)]);
    assert_eq!(c.gpr[4], 0xAA44_11DD);
    // rlwnm takes the count from rB (low 5 bits).
    assert_eq!(go(&["rlwnm 4,3,5,0,31"], &[(3, 0x8000_0001), (5, 36)]).gpr[4], 0x0000_0018);
    assert_eq!(go(&["rlwnm 4,3,5,0,31"], &[(3, 0x8000_0001), (5, 32)]).gpr[4], 0x8000_0001, "CONTROL: 32 & 31 = 0");
    let c = go(&["rlwinm. 4,3,0,0,31"], &[(3, 0x8000_0000)]);
    assert_eq!(c.cr_field(0), LT);
}

#[test]
fn compares_write_the_selected_cr_field_with_so() {
    let c = go(&["cmpw 1,3,4", "cmplw 2,3,4", "cmpwi 7,3,-1", "cmplwi 3,3,0xFFFF"], &[(3, 0xFFFF_FFFF), (4, 1)]);
    assert_eq!((c.cr_field(1), c.cr_field(2), c.cr_field(7), c.cr_field(3)), (LT, GT, EQ, GT));
    assert_eq!(c.cr_field(0), 0, "CONTROL: untouched fields stay 0");
    let c = gox(&["cmpw 5,3,4"], &[(3, 7), (4, 7)], XER_SO);
    assert_eq!(c.cr_field(5), EQ | SO, "SO from XER is copied into the field");
}

#[test]
fn condition_register_logic_and_moves() {
    // crset/crclr/cror/crxor on single bits; bit 0 is CR0[LT].
    let c = go(&["creqv 2,2,2", "creqv 5,5,5", "crxor 5,5,5", "cror 8,2,5", "crand 9,2,5", "crnor 10,2,5", "crandc 11,2,5"], &[]);
    assert!(c.cr_bit(2) && !c.cr_bit(5));
    assert_eq!((c.cr_bit(8), c.cr_bit(9), c.cr_bit(10), c.cr_bit(11)), (true, false, false, true));
    let c = go(&["creqv 2,2,2", "crorc 8,5,2", "crnand 9,2,2", "creqv 10,2,5"], &[]);
    assert_eq!((c.cr_bit(8), c.cr_bit(9), c.cr_bit(10)), (false, false, false), "orc(0,1)=0 nand(1,1)=0 eqv(1,0)=0");
    // mfcr / mtcrf / mcrf
    let c = prog(&["mtcrf 0x81,3", "mfcr 4", "mcrf 2,0"], |c, _| c.gpr[3] = 0xA5A5_A5A5).0;
    assert_eq!(c.gpr[4], 0xA000_0005, "only fields 0 and 7 were written");
    assert_eq!(c.cr_field(2), 0xA, "mcrf copied CR0");
    let c = prog(&["mtcrf 0xFF,3", "mfcr 4"], |c, _| c.gpr[3] = 0xA5A5_A5A5).0;
    assert_eq!(c.gpr[4], 0xA5A5_A5A5, "CONTROL: all eight fields");
    // mcrxr (not in LLVM's PowerPC tables: raw word 31/512, BF=1): XER[0:3] -> CR1, cleared.
    let (c, _, st) = run_words(&[0x7C80_0400], |c, _| c.xer = XER_SO | XER_CA | 0x12);
    assert!(st.trap.is_none());
    assert_eq!((c.cr_field(1), c.xer), (SO << 3 | 2, 0x12), "SO=8, CA=2 -> 0b1010");
}

#[test]
fn spr_moves_and_time_base() {
    let c = go(&["mtlr 3", "mflr 4", "mtctr 5", "mfctr 6", "mtxer 7", "mfxer 8", "mtlr 9"], &[(3, 0x1234), (5, 99), (7, 0xFFFF_FFFF), (9, EXIT)]);
    assert_eq!((c.gpr[4], c.gpr[6], c.gpr[8]), (0x1234, 99, 0xE000_007F), "XER keeps only SO OV CA and the byte count");
    let c = go(&["mftb 3", "mftb 4", "mftbu 5"], &[]);
    assert!(c.gpr[4] > c.gpr[3], "the time base advances with retired instructions");
    assert_eq!(c.gpr[5], 0);
}

#[test]
fn trap_word_conditions() {
    let (_, _, st) = prog(&["twi 4,3,5"], |c, _| c.gpr[3] = 5);
    assert!(matches!(st.trap, Some(retro_ppc::Trap::TrapWord { pc: 0x1000, .. })), "{st:?}");
    let (_, _, st) = prog(&["twi 4,3,5"], |c, _| c.gpr[3] = 6);
    assert!(st.trap.is_none(), "CONTROL: eq condition not met");
    let (_, _, st) = prog(&["tw 2,3,4"], |c, _| { c.gpr[3] = 1; c.gpr[4] = 0xFFFF_FFFF }); // 1 <u 0xFFFFFFFF
    assert!(st.trap.is_some());
    let (_, _, st) = prog(&["tw 16,3,4"], |c, _| { c.gpr[3] = 1; c.gpr[4] = 0xFFFF_FFFF }); // signed: 1 < -1 is false
    assert!(st.trap.is_none());
    let (_, _, st) = prog(&["trap"], |_, _| {});
    assert!(st.trap.is_some(), "tw 31,0,0 always traps");
}
