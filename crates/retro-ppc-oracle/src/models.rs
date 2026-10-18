//! Independent reference models, written from the PowerPC Programming Environments Manual (PEM, MPCFPE32B) rule
//! text, for the places where ppc-rs is wrong and so cannot be the oracle. A case is excluded from the ppc-rs
//! comparison only when retro-ppc matches the model.
use crate::world::Snap;

const VE: u32 = 1 << 7;

fn op(w: u32) -> u32 { w >> 26 }
fn is_nan(b: u64) -> bool { b >> 52 & 0x7FF == 0x7FF && b & ((1 << 52) - 1) != 0 }
fn is_snan(b: u64) -> bool { is_nan(b) && b & (1 << 51) == 0 }

/// The FPR operands an op-59/63 arithmetic instruction reads, in the NaN precedence order frA, frB, frC
/// (PEM 3.3.6.? "Invalid operation exception" / NaN operand rule: the result is the first NaN operand, quieted).
pub fn operands(w: u32) -> &'static [u32] {
    // shifts of frA, frB, frC
    const A: u32 = 16;
    const B: u32 = 11;
    const C: u32 = 6;
    if !matches!(op(w), 59 | 63) { return &[]; }
    match w >> 1 & 0x1F {
        18 | 20 | 21 => &[A, B],
        25 => &[A, C],
        22 | 24 | 26 => &[B],
        28..=31 => &[A, B, C],
        _ => &[],
    }
}

/// PEM NaN rule for arithmetic: with at least one NaN operand, the result is the FIRST NaN operand in the order
/// frA, frB, frC, made quiet, with its sign and payload kept (fmsub/fnmadd/fnmsub do not negate a NaN). `None` when
/// no operand is a NaN, or when the result is not written (a signalling NaN with FPSCR[VE] set: PEM 3.3.6.1).
pub fn nan_result(w: u32, pre: &Snap) -> Option<u64> {
    if op(w) == 63 && matches!(w >> 1 & 0x3FF, 0 | 32 | 12 | 14 | 15 | 40 | 72 | 136 | 264 | 583 | 711 | 134 | 70 | 38 | 64) { return None; }
    if op(w) == 63 && w >> 1 & 0x1F == 23 { return None; } // fsel
    let regs: Vec<u64> = operands(w).iter().map(|s| pre.fpr[(w >> s & 31) as usize]).collect();
    let n = *regs.iter().find(|b| is_nan(**b))?;
    if regs.iter().any(|b| is_snan(*b)) && pre.fpscr & VE != 0 { return None; }
    Some(n | 1 << 51)
}

/// PEM stfs / "Floating-point store single": the word stored for the double `d`.
///  * FRS[1:11] > 896 or FRS[1:63] = 0: WORD[0:1] <- FRS[0:1]; WORD[2:31] <- FRS[5:34]   (truncation, no rounding)
///  * 874 <= FRS[1:11] <= 896: the value denormalized and truncated to a multiple of 2^-149
///  * anything smaller is architecturally undefined (`None`).
/// Computed as ARITHMETIC (clear the 29 dropped fraction bits then convert exactly; scale by 2^149 and truncate toward
/// zero), not by restating the shift amounts of the implementation under test, so a shared misreading of the
/// pseudocode's bit slices cannot cancel out. The bit-slice form survives only for exponents past the single range,
/// where the PEM defines the word purely as that slice.
pub fn stfs_word(d: u64) -> Option<u32> {
    let exp = (d >> 52 & 0x7FF) as u32;
    let sign = (d >> 32) as u32 & 0x8000_0000;
    if d & !(1 << 63) == 0 { return Some(sign); }
    if exp > 1150 { return Some(((d >> 32) as u32 & 0xC000_0000) | (d >> 29) as u32 & 0x3FFF_FFFF); } // inf/NaN/overflow
    if exp > 896 {
        // normal single range: drop the low 29 fraction bits; the remainder is exactly a single.
        return Some((f64::from_bits(d & !0x1FFF_FFFF) as f32).to_bits());
    }
    if exp >= 874 {
        // single denormal range: truncate |d| to a multiple of 2^-149 (scaling by a power of two is exact).
        let units = (f64::from_bits(d & !(1 << 63)) * 2f64.powi(149)) as u32;
        return Some(sign | units);
    }
    None
}

/// PEM lfs conversion: the double a loaded single word becomes. A NaN or infinity keeps its fraction unchanged
/// (no quieting, FPSCR untouched); everything else, denormals included, widens exactly.
pub fn lfs_double(word: u32) -> u64 {
    let (sign, exp, frac) = ((word >> 31) as u64, word >> 23 & 0xFF, word as u64 & 0x7F_FFFF);
    if exp == 0xFF { sign << 63 | 0x7FF << 52 | frac << 29 } else { (f32::from_bits(word) as f64).to_bits() }
}
pub fn is_snan_single(word: u32) -> bool { word >> 23 & 0xFF == 0xFF && word & 0x7F_FFFF != 0 && word & 0x40_0000 == 0 }

/// PEM FPRF for a denormalized number: C plus FG (positive) or FL (negative) (Table "Floating-point result flags").
pub fn fprf_denormal(bits: u64) -> u32 { 1 << 16 | if bits >> 63 != 0 { 1 << 15 } else { 1 << 14 } }

/// PEM fctiw: NaN converts to 0x8000_0000 (the most negative integer) with VXCVI set (VE=0).
pub const FCTIW_NAN: u32 = 0x8000_0000;
