//! Toolbox Utilities that are pure arithmetic: the Fixed (16.16) and Fract (2.30) conversions and products, the bit routines
//! on longwords and on bit strings, `HiWord` / `LoWord`, and `Random`. Inside Macintosh I, "Toolbox Utilities", and IV
//! ("Fixed-Point Arithmetic", "Fixed Point Conversions"); the overflow answers (the largest or smallest value of the sign)
//! follow the book, cross-read against Executor's `toolutil.cpp` (MIT).
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::quickdraw::OFF_RAND_SEED;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const POS_OVERFLOW: i32 = 0x7FFF_FFFF;
pub const NEG_OVERFLOW: i32 = i32::MIN;

/// `FixRatio(numer, denom)`: `numer / denom` as a Fixed; a zero denominator gives the overflow value of numer's sign.
pub fn fix_ratio(n: i16, d: i16) -> i32 {
    if d == 0 { return if n < 0 { NEG_OVERFLOW + 1 } else { POS_OVERFLOW }; }
    ((i32::from(n) << 16) / i32::from(d)).clamp(NEG_OVERFLOW + 1, POS_OVERFLOW)
}
/// `FixMul`: the product, truncated toward zero, saturating.
pub fn fix_mul(a: i32, b: i32) -> i32 {
    let neg = (a < 0) != (b < 0);
    let p = (i64::from(a).unsigned_abs() * i64::from(b).unsigned_abs()) >> 16;
    if p > POS_OVERFLOW as u64 { return if neg { NEG_OVERFLOW } else { POS_OVERFLOW }; }
    if neg { -(p as i32) } else { p as i32 }
}
/// `FixDiv`: `x / y` as a Fixed, truncated; division by zero is the overflow value of x's sign.
pub fn fix_div(x: i32, y: i32) -> i32 {
    if y == 0 { return if x < 0 { NEG_OVERFLOW } else { POS_OVERFLOW }; }
    let q = (i64::from(x) << 16) / i64::from(y);
    q.clamp(i64::from(NEG_OVERFLOW), i64::from(POS_OVERFLOW)) as i32
}
/// `FracMul` (2.30): `x * y`, saturating; the book's one quirk, $80000000 * $80000000, is 0.
pub fn frac_mul(x: i32, y: i32) -> i32 {
    if x == i32::MIN && y == i32::MIN { return 0; }
    ((i64::from(x) * i64::from(y)) >> 30).clamp(i64::from(NEG_OVERFLOW), i64::from(POS_OVERFLOW)) as i32
}
pub fn frac_div(x: i32, y: i32) -> i32 {
    if y == 0 { return if x < 0 { NEG_OVERFLOW } else { POS_OVERFLOW }; }
    ((i64::from(x) << 30) / i64::from(y)).clamp(i64::from(NEG_OVERFLOW), i64::from(POS_OVERFLOW)) as i32
}
pub fn long2fix(x: i32) -> i32 { if x > 0x7FFF { POS_OVERFLOW } else if x < -0x8000 { NEG_OVERFLOW } else { x << 16 } }
/// `Fix2Long` rounds to the nearest integer (halves up).
pub fn fix2long(x: i32) -> i32 { (x.wrapping_add(0x8000)) >> 16 }
pub fn fix2frac(x: i32) -> i32 { if x > 0x1FFFF { POS_OVERFLOW } else if x < -0x20000 { NEG_OVERFLOW } else { x << 14 } }
pub fn frac2fix(x: i32) -> i32 { (x.wrapping_add(1 << 13)) >> 14 }
/// `FixRound`: nearest integer as an INTEGER.
pub fn fix_round(x: i32) -> i16 { ((x >> 16) + i32::from(x & 0x8000 != 0)) as i16 }

/// `Random`'s next seed and the value it returns (the Park-Miller generator the Toolbox uses: `seed * 16807 mod (2^31 - 1)`).
pub fn random_step(seed: u32) -> (u32, i16) {
    let s = u64::from(if seed >= 0x8000_0000 { (seed & 0x7FFF_FFFF) + 1 } else { seed });
    let mut n = ((s * 16807) % 0x7FFF_FFFF) as u32;
    if n == 0x7FFF_FFFF { n = 0; }
    (n, if n as i16 == i16::MIN { 0 } else { n as i16 })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::{B, L, P, W};
    m.register(0xA869, &[W, W], Some(L), |_, _, c| val(c, fix_ratio(c.args.get(0) as i16, c.args.get(1) as i16)))?;
    m.register(0xA868, &[L, L], Some(L), |_, _, c| val(c, fix_mul(c.args.get(0) as i32, c.args.get(1) as i32)))?;
    m.register(0xA84D, &[L, L], Some(L), |_, _, c| val(c, fix_div(c.args.get(0) as i32, c.args.get(1) as i32)))?;
    m.register(0xA84A, &[L, L], Some(L), |_, _, c| val(c, frac_mul(c.args.get(0) as i32, c.args.get(1) as i32)))?;
    m.register(0xA84B, &[L, L], Some(L), |_, _, c| val(c, frac_div(c.args.get(0) as i32, c.args.get(1) as i32)))?;
    m.register(0xA83F, &[L], Some(L), |_, _, c| val(c, long2fix(c.args.get(0) as i32)))?;
    m.register(0xA840, &[L], Some(L), |_, _, c| val(c, fix2long(c.args.get(0) as i32)))?;
    m.register(0xA841, &[L], Some(L), |_, _, c| val(c, fix2frac(c.args.get(0) as i32)))?;
    m.register(0xA842, &[L], Some(L), |_, _, c| val(c, frac2fix(c.args.get(0) as i32)))?;
    m.register(0xA86C, &[L], Some(W), |_, _, c| val(c, i32::from(fix_round(c.args.get(0) as i32))))?;
    m.register(0xA86A, &[L], Some(W), |_, _, c| val(c, (c.args.get(0) >> 16) as i32))?;
    m.register(0xA86B, &[L], Some(W), |_, _, c| val(c, (c.args.get(0) & 0xFFFF) as i32))?;
    m.register(0xA858, &[L, L], Some(L), |_, _, c| val(c, (c.args.get(0) & c.args.get(1)) as i32))?;
    m.register(0xA85B, &[L, L], Some(L), |_, _, c| val(c, (c.args.get(0) | c.args.get(1)) as i32))?;
    m.register(0xA859, &[L, L], Some(L), |_, _, c| val(c, (c.args.get(0) ^ c.args.get(1)) as i32))?;
    m.register(0xA85A, &[L], Some(L), |_, _, c| val(c, !c.args.get(0) as i32))?;
    m.register(0xA85C, &[L, W], Some(L), |_, _, c| {
        // BitShift(a, count): positive shifts left, negative right (logical); 32 or more is 0.
        let (a, n) = (c.args.get(0), c.args.get(1) as i16);
        let k = i32::from(n).unsigned_abs() % 64;
        val(c, if k >= 32 { 0 } else if n < 0 { (a >> k) as i32 } else { (a << k) as i32 })
    })?;
    m.register(0xA85D, &[P, L], Some(B), bit_tst::<U>)?;
    m.register(0xA85E, &[P, L], None, bit_set::<U>)?;
    m.register(0xA85F, &[P, L], None, bit_clr::<U>)?;
    m.register(0xA861, &[], Some(W), random::<U>)
}

fn val(c: &Call, v: i32) -> Result<Done, Refusal> { Ok(Done { result: v as u32, log: format!("[toolbox] {} -> {v:#x}", c.name) }) }

/// The byte holding bit `n` of the string at `p` and the mask for it: bit 0 is the HIGH bit of the first byte.
fn bit_at(p: u32, n: u32) -> (u32, u8) { (p.wrapping_add(((n as i32) >> 3) as u32), 0x80 >> (n & 7)) }

fn bit_tst<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (a, mask) = bit_at(c.args.get(0), c.args.get(1));
    let b = m.mem.read_u8(a).map_err(Refusal::mem("BitTst"))?;
    Ok(Done { result: u32::from(b & mask != 0), log: format!("[toolbox] BitTst {a:#x} {:#x}", mask) })
}
fn bit_set<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (a, mask) = bit_at(c.args.get(0), c.args.get(1));
    let b = m.mem.read_u8(a).map_err(Refusal::mem("BitSet"))?;
    m.mem.write_u8(a, b | mask).map_err(Refusal::mem("BitSet"))?;
    Ok(Done { result: 0, log: format!("[toolbox] BitSet {a:#x} {mask:#x}") })
}
fn bit_clr<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (a, mask) = bit_at(c.args.get(0), c.args.get(1));
    let b = m.mem.read_u8(a).map_err(Refusal::mem("BitClr"))?;
    m.mem.write_u8(a, b & !mask).map_err(Refusal::mem("BitClr"))?;
    Ok(Done { result: 0, log: format!("[toolbox] BitClr {a:#x} {mask:#x}") })
}

/// `Random`: advances `qd.randSeed` (InitGraf seeds it with 1) and returns the new value.
fn random<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    if m.qd_globals == 0 { return Err(Refusal("Random: InitGraf has not run (qd.randSeed does not exist yet)".into())); }
    let at = m.qd_globals - OFF_RAND_SEED;
    let (seed, v) = random_step(m.mem.read_u32(at).map_err(Refusal::mem("randSeed"))?);
    m.mem.write_u32(at, seed).map_err(Refusal::mem("randSeed"))?;
    Ok(Done { result: v as u16 as u32, log: format!("[toolbox] Random -> {v}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_arithmetic_and_its_overflow_answers() {
        assert_eq!(fix_ratio(1, 2), 0x8000);
        assert_eq!(fix_ratio(-3, 2), -0x18000);
        assert_eq!(fix_ratio(5, 0), POS_OVERFLOW, "x/0 is the overflow value");
        assert_eq!(fix_mul(0x30000, 0x28000), 0x78000); // 3 * 2.5
        assert_eq!(fix_mul(-0x30000, 0x28000), -0x78000);
        assert_eq!(fix_mul(0x7FFF_0000, 0x7FFF_0000), POS_OVERFLOW, "saturates");
        assert_eq!(fix_div(0x78000, 0x30000), 0x28000);
        assert_eq!(fix_div(1, 0), POS_OVERFLOW);
        assert_eq!(fix_div(-1, 0), NEG_OVERFLOW);
        assert_eq!(long2fix(3), 0x30000);
        assert_eq!(long2fix(0x8000), POS_OVERFLOW);
        assert_eq!(fix2long(0x2_8000), 3, "halves round up");
        assert_eq!(fix2long(-0x2_8000), -2);
        assert_eq!(fix_round(0x1_7FFF), 1);
        assert_eq!(fix_round(0x1_8000), 2);
        assert_eq!(frac_mul(1 << 30, 1 << 30), 1 << 30, "1.0 * 1.0");
        assert_eq!(frac_mul(i32::MIN, i32::MIN), 0, "the book's deliberate quirk");
        assert_eq!(fix2frac(0x10000), 0x4000_0000);
        assert_eq!(frac2fix(1 << 30), 0x10000);
    }

    #[test]
    fn random_is_the_park_miller_sequence_from_seed_1() {
        let (s1, v1) = random_step(1);
        assert_eq!((s1, v1), (16807, 16807));
        let (s2, _) = random_step(s1);
        assert_eq!(s2, 282_475_249); // 16807^2
        // NEGATIVE CONTROL: a seed with the top bit set is folded in first (the Toolbox does), it does not wrap to a different stream.
        assert_eq!(random_step(0x8000_0001).0, random_step(2).0);
    }
}
