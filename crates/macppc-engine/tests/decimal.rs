//! MathLib decimal conversions on the spec's test vectors (macos9-specs/MathLib/{dec2num,num2dec,nan}).
use macppc_engine::decimal::{dec2num_bits, nan_bits, num2dec, DECIMAL_SIZE};
use macppc_engine::symbols::Binding;
use macppc_engine::MacPpc;

fn rec(sgn: u8, exp: i16, sig: &str) -> [u8; DECIMAL_SIZE] {
    let mut d = [0u8; DECIMAL_SIZE];
    d[0] = sgn; d[2..4].copy_from_slice(&exp.to_be_bytes()); d[4] = sig.len() as u8; d[5..5 + sig.len()].copy_from_slice(sig.as_bytes());
    d
}

#[test]
fn dec2num_vectors() {
    assert_eq!(dec2num_bits(&rec(0, 0, "1")), 0x3FF0_0000_0000_0000);
    assert_eq!(dec2num_bits(&rec(1, -1, "15")), 0xBFF8_0000_0000_0000);
    assert_eq!(f64::from_bits(dec2num_bits(&rec(0, 2, "5"))), 500.0);
    assert_eq!((dec2num_bits(&rec(0, 0, "0")), dec2num_bits(&rec(1, 0, "0")), dec2num_bits(&rec(0, 0, ""))), (0, 1 << 63, 0));
    assert_eq!((dec2num_bits(&rec(0, 0, "I")), dec2num_bits(&rec(1, 0, "I"))), (0x7FF0_0000_0000_0000, 0xFFF0_0000_0000_0000));
    assert_eq!(dec2num_bits(&rec(0, 0, "N")), 0x7FF8_02A0_0000_0000, "the default NaN, code 21");
    assert_eq!(dec2num_bits(&rec(0, 0, "NFF")), 0x7FFF_E000_0000_0000);
    assert_eq!(dec2num_bits(&rec(0, 400, "1")), 0x7FF0_0000_0000_0000, "overflow");
    assert_eq!(dec2num_bits(&rec(0, -400, "1")), 0, "underflow");
    assert_eq!(f64::from_bits(dec2num_bits(&rec(0, -35, "123456789012345678901234567890123456"))), 1.23456789012345678901234567890123456f64);
}

#[test]
fn nan_vectors() {
    for (tag, bits) in [("", 0x7FF8_0000_0000_0000u64), ("0", 0x7FF8_0000_0000_0000), ("1", 0x7FF8_0020_0000_0000), ("21", 0x7FF8_02A0_0000_0000), ("255", 0x7FF8_1FE0_0000_0000),
                        ("1000", 0x7FF8_1FE0_0000_0000), ("-3", 0x7FF8_0000_0000_0000), ("abc", 0x7FF8_0000_0000_0000)] {
        assert_eq!(nan_bits(tag.as_bytes()), bits, "nan({tag:?})");
    }
    // INF / NAN spellings are k = -32768: nothing is added; NEGATIVE control: a plain number adds
    assert_eq!(nan_bits(b"INF"), 0x7FF8_0000_0000_0000);
    assert_ne!(nan_bits(b"7"), nan_bits(b"INF"));
}

#[test]
fn num2dec_vectors() {
    let sig = |d: &[u8; DECIMAL_SIZE]| String::from_utf8_lossy(&d[5..5 + usize::from(d[4])]).into_owned();
    let exp = |d: &[u8; DECIMAL_SIZE]| i16::from_be_bytes([d[2], d[3]]);
    let d = num2dec(0, 5, 3.14159265).unwrap();
    assert_eq!((d[0], sig(&d).as_str(), exp(&d)), (0, "31416", -4));
    let d = num2dec(1, 2, 2.5).unwrap();
    assert_eq!((sig(&d).as_str(), exp(&d)), ("250", -2));
    let d = num2dec(0, 3, -0.0).unwrap();
    assert_eq!((d[0], sig(&d).as_str()), (1, "0"));
    let d = num2dec(0, 1, 9.6).unwrap();
    assert_eq!((sig(&d).as_str(), exp(&d)), ("1", 1), "rounding carry");
    assert_eq!(sig(&num2dec(0, 5, f64::INFINITY).unwrap()), "I");
    // NaN code 21 round-trips through dec2num
    let nan21 = f64::from_bits(0x7FF8_02A0_0000_0000);
    let d = num2dec(0, 5, nan21).unwrap();
    assert!(sig(&d).starts_with('N'));
    assert_eq!(dec2num_bits(&d), 0x7FF8_02A0_0000_0000);
    // NEGATIVE controls: a negative FIXED digit count and a bad style are refused by name; a value that rounds to zero keeps exp = -digits
    assert!(num2dec(1, -2, 1234.0).unwrap_err().contains("not implemented"));
    assert!(num2dec(7, 3, 1.0).unwrap_err().contains("style 7"));
    let d = num2dec(1, 2, 0.001).unwrap();
    assert_eq!((sig(&d).as_str(), exp(&d)), ("0", -2));
}

#[test]
fn the_names_are_bound_for_carbonlib_and_mathlib() {
    let mut m = MacPpc::new(guestui_abi::RecordingGuestUi::new()).unwrap();
    for n in ["dec2num", "num2dec", "nan"] {
        assert!(!matches!(m.bind_from("CarbonLib", n), Binding::Unbound(_)), "{n}");
        assert!(!matches!(m.bind_from("MathLib", n), Binding::Unbound(_)), "{n}");
    }
}
