//! Whole programs, assembled by LLVM from tests/asm/*.s, run to completion: a bit-serial CRC-32 (checked against the
//! published check value), recursive fib with a real stack frame, and two FP idioms Mac code uses.
mod common;
use common::*;

const BUF: u32 = 0x4000;

#[test]
fn crc32_of_123456789_is_the_published_check_value() {
    let run = |bytes: &[u8]| {
        let (c, _, st) = run_words(programs::CRC32, |c, v| {
            v.mem.write_bytes(BUF, bytes).unwrap();
            c.gpr[3] = BUF; c.gpr[4] = bytes.len() as u32;
        });
        assert!(st.halted && st.trap.is_none(), "{st:?}");
        (c.gpr[5], st.insns)
    };
    let (crc, insns) = run(b"123456789");
    assert_eq!(crc, 0xCBF4_3926, "CRC-32/ISO-HDLC check value (catalogue of parametrised CRC algorithms)");
    assert!(insns > 400, "it really iterated: {insns}");
    // CONTROLS: one flipped byte changes it; the empty message gives 0; a known second vector.
    assert_ne!(run(b"123456780").0, crc);
    assert_eq!(run(b"").0, 0);
    assert_eq!(run(b"a").0, 0xE8B7_BE43, "CRC-32 of \"a\"");
}

#[test]
fn recursive_fibonacci_uses_the_stack_frame_correctly() {
    let fib = |n: u32| {
        let (c, _, st) = run_words(programs::FIB, |c, _| c.gpr[3] = n);
        assert!(st.halted && st.trap.is_none(), "n={n}: {st:?}");
        assert_eq!(c.gpr[1], STACK, "stack pointer balanced (n={n})");
        c.gpr[3]
    };
    let want = [0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55];
    for (n, w) in want.iter().enumerate() { assert_eq!(fib(n as u32), *w, "fib({n})"); }
    assert_eq!(fib(20), 6765);
    assert_ne!(fib(20), 6764, "CONTROL: the check can fail");
}

#[test]
fn int_to_double_idiom_and_fctiwz_round_trip() {
    for (n, back) in [(41i32, 61u32), (-123456, (-185184i32) as u32), (0, 0), (i32::MIN, 0x8000_0000), (7, 10)] {
        let (c, v, st) = run_words(programs::INT2DBL, |c, v| {
            c.gpr[3] = n as u32; c.gpr[10] = BUF;
            v.mem.write_bytes(BUF + 8, &0x4330_0000_8000_0000u64.to_be_bytes()).unwrap();
            v.mem.write_bytes(BUF + 24, &1.5f64.to_be_bytes()).unwrap();
        });
        assert!(st.halted && st.trap.is_none(), "{st:?}");
        assert_eq!(c.fpr_f64(1), n as f64, "(double){n}");
        assert_eq!(v.mem.read_u32(BUF + 20).unwrap(), back, "trunc(1.5*{n})");
    }
}

#[test]
fn horner_polynomial_with_fused_multiply_add() {
    for (x, want) in [(2.0, 39.0), (-1.0, 3.0), (0.0, 5.0), (0.5, 3.0 * 0.125 + 2.0 * 0.25 + 0.5 + 5.0)] {
        let (c, v, st) = run_words(programs::HORNER, |c, v| {
            c.gpr[10] = BUF; c.set_fpr_f64(2, x);
            for (i, k) in [3.0f64, 2.0, 1.0, 5.0].iter().enumerate() { v.mem.write_bytes(BUF + 8 * i as u32, &k.to_be_bytes()).unwrap(); }
        });
        assert!(st.halted && st.trap.is_none(), "{st:?}");
        assert_eq!(c.fpr_f64(1), want, "p({x})");
        let stored = f64::from_bits((v.mem.read_u32(BUF + 32).unwrap() as u64) << 32 | v.mem.read_u32(BUF + 36).unwrap() as u64);
        assert_eq!(stored, want);
    }
}
