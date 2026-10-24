//! utility.library SDivMod32 / UDivMod32: the quotient in D0 and the remainder in D1, as the m68k assembly callers read them.
mod common;
use common::*;

#[test]
fn div_mod32_returns_quotient_in_d0_and_remainder_in_d1_and_refuses_division_by_zero() {
    let mut a = machine();
    let r = common::run_refusing(&mut a, include_bytes!("guest/divmod.bin"));
    assert_eq!((r[0], r[1]), (14, 2), "100 / 7");
    assert_eq!((r[2] as i32, r[3] as i32), (-14, -2), "-100 / 7: the remainder takes the dividend's sign (DIVS)");
    assert_eq!((r[4] as i32, r[5]), (-14, 2), "100 / -7");
    assert_eq!((r[6], r[7]), (0x0FFF_FFFF, 0), "UDivMod32 is unsigned: 0xFFFFFFF0 / 16");
    assert_eq!((r[8], r[9]), (429_496_729, 5), "UDivMod32 0xFFFFFFFF / 10");
    assert_eq!((r[10], r[11]), (1, 1), "the program ran past the zero divide");
    assert_eq!((r[12], r[13]), (0, 0), "NEGATIVE CONTROL: a zero divisor is refused, the call answers 0 and no quotient");
    assert!(a.refused.iter().any(|x| x.contains("SDivMod32(0x5, 0): division by zero")), "{:?}", a.refused);
}
