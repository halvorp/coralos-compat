//! mathieeesingbas / mathieeedoubbas, driven by tests/guest/c/mathieee.c (doubles in D0:D1, D2:D3).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/mathieee.bin");

#[test]
fn single_and_double_arithmetic_follow_the_aros_register_conventions() {
    let mut a = machine();
    let r = run(&mut a, PROG);
    assert_eq!(r[0], 1, "both libraries open");
    assert_eq!((r[1], r[2], r[3] as i32, r[4] as i32, r[5], r[6]), (0x4070_0000, 0xBF40_0000, -2, -1, 0, 0x40E0_0000), "singles: add, sub (y - z), fix truncates, cmp, equal cmp (negative control), flt");
    assert_eq!((r[7], r[8]), (0x400E_0000, 0), "double add 1.5 + 2.25 = 3.75");
    assert_eq!((r[9], r[10], r[11] as i32), (0xBFE8_0000, 0x3FF8_0000, -2), "double sub is y - z, div is y / z, fix truncates toward zero");
    assert_eq!((r[12], r[13], r[14] as i32, r[15] as i32, r[16]), (0xC008_0000, 0xC000_0000, -1, -1, 0x401C_0000), "floor, ceil, cmp, tst, flt");
    assert_eq!((r[17], r[18]), (0x7FF0_0000, 0), "x / 0 is +infinity, not a fault");
    assert_eq!((r[19], r[20]), (0xBFF8_0000, 0x4006_0000), "neg, abs");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}
