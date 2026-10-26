// Adapted from HexRaysSA/rax src/isa/arm/aarch32/cpu.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! The ARM ARM's shared arithmetic pseudocode: AddWithCarry, Shift_C, ARMExpandImm_C, ConditionPassed and
//! the saturation helpers.

/// Shift types as encoded in bits [6:5] of a shifted-register operand, plus RRX.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Lsl,
    Lsr,
    Asr,
    Ror,
    Rrx,
}

impl Shift {
    pub fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Shift::Lsl,
            1 => Shift::Lsr,
            2 => Shift::Asr,
            _ => Shift::Ror,
        }
    }
}

/// DecodeImmShift: the immediate-shift encoding's (type, imm5) to the shift actually applied.
pub fn decode_imm_shift(ty: u32, imm5: u32) -> (Shift, u32) {
    match (Shift::from_bits(ty), imm5) {
        (Shift::Lsr, 0) => (Shift::Lsr, 32),
        (Shift::Asr, 0) => (Shift::Asr, 32),
        (Shift::Ror, 0) => (Shift::Rrx, 1),
        (s, n) => (s, n),
    }
}

/// AddWithCarry: (result, carry_out, overflow).
pub fn add_with_carry(x: u32, y: u32, carry_in: bool) -> (u32, bool, bool) {
    let unsigned_sum = x as u64 + y as u64 + carry_in as u64;
    let signed_sum = x as i32 as i64 + y as i32 as i64 + carry_in as i64;
    let result = unsigned_sum as u32;
    (result, result as u64 != unsigned_sum, result as i32 as i64 != signed_sum)
}

/// Shift_C: an amount of 0 returns the value and carry unchanged (register-specified shifts rely on it).
pub fn shift_c(value: u32, shift: Shift, amount: u32, carry_in: bool) -> (u32, bool) {
    if amount == 0 && shift != Shift::Rrx {
        return (value, carry_in);
    }
    match shift {
        Shift::Lsl => match amount {
            1..=31 => (value << amount, (value >> (32 - amount)) & 1 != 0),
            32 => (0, value & 1 != 0),
            _ => (0, false),
        },
        Shift::Lsr => match amount {
            1..=31 => (value >> amount, (value >> (amount - 1)) & 1 != 0),
            32 => (0, value >> 31 != 0),
            _ => (0, false),
        },
        Shift::Asr => {
            if amount >= 32 {
                let sign = value >> 31 != 0;
                (if sign { u32::MAX } else { 0 }, sign)
            } else {
                (((value as i32) >> amount) as u32, (value >> (amount - 1)) & 1 != 0)
            }
        }
        Shift::Ror => {
            let result = value.rotate_right(amount % 32);
            (result, result >> 31 != 0)
        }
        Shift::Rrx => ((value >> 1) | ((carry_in as u32) << 31), value & 1 != 0),
    }
}

/// ARMExpandImm_C: the rotated 8-bit immediate of data-processing (immediate) and MSR (immediate).
pub fn expand_imm_c(imm12: u32, carry_in: bool) -> (u32, bool) {
    let rotation = ((imm12 >> 8) & 0xF) * 2;
    if rotation == 0 {
        (imm12 & 0xFF, carry_in)
    } else {
        let result = (imm12 & 0xFF).rotate_right(rotation);
        (result, result >> 31 != 0)
    }
}

/// ConditionPassed for a 4-bit condition; 0b1111 is "always" (the caller routes the unconditional space).
pub fn condition_passed(cond: u32, n: bool, z: bool, c: bool, v: bool) -> bool {
    let result = match cond >> 1 {
        0 => z,
        1 => c,
        2 => n,
        3 => v,
        4 => c && !z,
        5 => n == v,
        6 => !z && n == v,
        _ => true,
    };
    if cond & 1 != 0 && cond != 0xF {
        !result
    } else {
        result
    }
}

/// SignedSatQ: saturate `x` to a `bits`-wide signed integer (1..=32), with the saturated flag.
pub fn signed_sat(x: i64, bits: u32) -> (i64, bool) {
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    if x > max {
        (max, true)
    } else if x < min {
        (min, true)
    } else {
        (x, false)
    }
}

/// UnsignedSatQ: saturate `x` to a `bits`-wide unsigned integer (0..=31), with the saturated flag.
pub fn unsigned_sat(x: i64, bits: u32) -> (i64, bool) {
    let max = (1i64 << bits) - 1;
    if x > max {
        (max, true)
    } else if x < 0 {
        (0, true)
    } else {
        (x, false)
    }
}
