//! VFP behaviour the Unicorn vectors cannot check: short vectors with stride 2 and double-precision
//! monadic vectors (Unicorn's QEMU 5.0 base gets both wrong), the trap-enable bits Unicorn holds RAZ,
//! FPEXC, and a few worked IEEE cases. Expected values follow the ARMv6 ARM (DDI 0100I, part C).

mod common;
use common::{a32, trap_kind};
use heirloom_cpu_arm::{Exit, TrapKind};

const ONE: u32 = 0x3F80_0000;
const TWO: u32 = 0x4000_0000;
const F64_ONE: u64 = 0x3FF0_0000_0000_0000;

fn f(x: f32) -> u32 {
    x.to_bits()
}

#[test]
fn stride_two_vector_add_wraps_inside_the_bank() {
    // VADD.F32 s8, s16, s24 with LEN = 3 (field 2), STRIDE = 2 (field 3):
    // s8 = s16 + s24, s10 = s18 + s26, s12 = s20 + s28.
    let (mut cpu, mut mem) = a32(&[0xEE38_4A0C]);
    cpu.vfp.fpscr = 2 << 16 | 3 << 20;
    for i in 0..8 {
        cpu.vfp.s[16 + i] = f(i as f32);
        cpu.vfp.s[24 + i] = f(100.0 * i as f32);
    }
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(cpu.vfp.s[8], f(0.0));
    assert_eq!(cpu.vfp.s[10], f(202.0));
    assert_eq!(cpu.vfp.s[12], f(404.0));
    assert_eq!((cpu.vfp.s[9], cpu.vfp.s[11], cpu.vfp.s[14]), (0, 0, 0), "odd lanes and s14 untouched");
}

#[test]
fn stride_two_vector_wraps_past_the_bank_end() {
    // VMUL.F32 s14, s14, s30 (s30 is not in bank 0, so it steps too): LEN 2, STRIDE 2:
    // s14 = s14 * s30; s8 (14 + 2 wraps to 8) = s8 * s24 (30 + 2 wraps to 24).
    let (mut cpu, mut mem) = a32(&[0xEE27_7A0F]);
    cpu.vfp.fpscr = 1 << 16 | 3 << 20;
    cpu.vfp.s[14] = f(3.0);
    cpu.vfp.s[30] = f(5.0);
    cpu.vfp.s[8] = f(7.0);
    cpu.vfp.s[24] = f(11.0);
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!((cpu.vfp.s[14], cpu.vfp.s[8]), (f(15.0), f(77.0)));
}

#[test]
fn scalar_operand_in_bank_zero_is_reused_by_every_element() {
    // VMUL.F32 s16, s24, s0 with LEN = 4: s16..s19 = s24..s27 * s0.
    let (mut cpu, mut mem) = a32(&[0xEE2C_8A00]);
    cpu.vfp.fpscr = 3 << 16;
    cpu.vfp.s[0] = TWO;
    for i in 0..4 {
        cpu.vfp.s[24 + i] = f(1.0 + i as f32);
    }
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(&cpu.vfp.s[16..20], &[f(2.0), f(4.0), f(6.0), f(8.0)]);
}

#[test]
fn bank_zero_destination_is_scalar_even_with_len_set() {
    let (mut cpu, mut mem) = a32(&[0xEE30_0A01]); // VADD.F32 s0, s0, s2
    cpu.vfp.fpscr = 7 << 16;
    cpu.vfp.s[0] = ONE;
    cpu.vfp.s[2] = ONE;
    cpu.vfp.s[1] = 0x1234;
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!((cpu.vfp.s[0], cpu.vfp.s[1]), (TWO, 0x1234));
}

#[test]
fn double_precision_monadic_ops_are_vectors_too() {
    // VNEG.F64 d4, d8 with LEN = 3: d4..d6 = -d8..-d10. VABS.F64 d12, d0 (scalar source): d12..d14 = |d0|.
    let (mut cpu, mut mem) = a32(&[0xEEB1_4B48, 0xEEB0_CBC0]);
    cpu.vfp.fpscr = 2 << 16;
    for i in 0..3 {
        cpu.vfp.set_d(8 + i, F64_ONE + i as u64);
    }
    cpu.vfp.set_d(0, F64_ONE | 1 << 63);
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent);
    for i in 0..3 {
        assert_eq!(cpu.vfp.d(4 + i), (F64_ONE + i as u64) | 1 << 63);
        assert_eq!(cpu.vfp.d(12 + i), F64_ONE);
    }
    assert_eq!(cpu.vfp.d(7), 0);
}

#[test]
fn vector_shapes_that_leave_the_bank_are_refused() {
    // Double precision, LEN = 3, STRIDE = 2: 6 registers in a 4-register bank.
    let (mut cpu, mut mem) = a32(&[0xEE34_4B08]); // VADD.F64 d4, d4, d8
    cpu.vfp.fpscr = 2 << 16 | 3 << 20;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Unpredictable);
}

#[test]
fn enabled_exception_traps_and_changes_nothing() {
    // VDIV.F32 s0, s1, s2 with 1/0 and DZE (bit 9) set: a trap, s0 and FPSCR unchanged.
    let (mut cpu, mut mem) = a32(&[0xEE80_0A81]);
    cpu.vfp.fpscr = 1 << 9;
    cpu.vfp.s[1] = ONE;
    cpu.vfp.s[0] = 0x5555;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::VfpException { flags: 1 << 1 });
    assert_eq!((cpu.vfp.s[0], cpu.vfp.fpscr), (0x5555, 1 << 9));
    // Without the enable: +inf and DZC.
    cpu.vfp.fpscr = 0;
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!((cpu.vfp.s[0], cpu.vfp.fpscr), (0x7F80_0000, 1 << 1));
}

#[test]
fn trap_enables_round_trip_through_vmsr() {
    // VMSR FPSCR, r0; VMRS r1, FPSCR: VFP11 keeps the enables (bits 15, 12:8).
    let (mut cpu, mut mem) = a32(&[0xEEE1_0A10, 0xEEF1_1A10]);
    cpu.regs[0] = 0xFFFF_FFFF;
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent);
    assert_eq!(cpu.regs[1], 0xF3F7_9F9F);
}

#[test]
fn fpexc_and_mvfr_are_not_user_registers() {
    for word in [0xEEF8_0A10u32, 0xEEE8_0A10, 0xEEF7_0A10, 0xEEF6_0A10] {
        let (mut cpu, mut mem) = a32(&[word]);
        assert!(matches!(trap_kind(cpu.step(&mut mem)), TrapKind::VfpSystemRegister { .. }), "{word:#010x}");
    }
    let (mut cpu, mut mem) = a32(&[0xEEF0_0A10]); // VMRS r0, FPSID
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(cpu.regs[0], 0x4101_20B5);
}

#[test]
fn vmrs_apsr_nzcv_copies_the_compare_result() {
    // VCMP.F32 s0, s1 with 1 < 2, then VMRS APSR_nzcv, FPSCR: N set, Z C V clear.
    let (mut cpu, mut mem) = a32(&[0xEEB4_0A60, 0xEEF1_FA10]);
    cpu.vfp.s[0] = ONE;
    cpu.vfp.s[1] = TWO;
    cpu.z = true;
    cpu.c = true;
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent);
    assert!(cpu.n && !cpu.z && !cpu.c && !cpu.v);
}

#[test]
fn ieee_worked_cases() {
    // VSQRT.F64 d0, d1 of 2.0 is correctly rounded; VCVT.S32.F64 rounds towards zero; VCVTR uses RMode.
    let (mut cpu, mut mem) = a32(&[0xEEB1_0BC1, 0xEEBD_4BC2, 0xEEBD_5B42]);
    cpu.vfp.set_d(1, 2.0f64.to_bits());
    cpu.vfp.set_d(2, (-2.5f64).to_bits());
    assert_eq!(cpu.run(&mut mem, 3), Exit::BudgetSpent);
    assert_eq!(cpu.vfp.d(0), 0x3FF6_A09E_667F_3BCD, "sqrt(2)");
    assert_eq!(cpu.vfp.s[8], (-2i32) as u32, "VCVT: towards zero");
    assert_eq!(cpu.vfp.s[10], (-2i32) as u32, "VCVTR: ties to even");
    assert_eq!(cpu.vfp.fpscr & 0x1F, 1 << 4, "IXC");
}
