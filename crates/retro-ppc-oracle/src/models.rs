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
///  * 874 <= FRS[1:11] <= 896: denormalize: the implicit 1 and FRS[12:63] shifted right by 897 - exp, truncated
///  * anything smaller is architecturally undefined (`None`).
pub fn stfs_word(d: u64) -> Option<u32> {
    let exp = (d >> 52 & 0x7FF) as u32;
    if exp > 896 || d & !(1 << 63) == 0 {
        return Some(((d >> 32) as u32 & 0xC000_0000) | (d >> 29) as u32 & 0x3FFF_FFFF);
    }
    if exp >= 874 {
        let frac = 1u64 << 52 | (d & ((1 << 52) - 1));
        let sh = 897 - exp + 29; // 29 drops the 52->23 fraction width, 897-exp the denormalizing shift
        return Some((d >> 32) as u32 & 0x8000_0000 | (frac >> sh) as u32);
    }
    None
}

/// PEM fctiw: NaN converts to 0x8000_0000 (the most negative integer) with VXCVI set (VE=0).
pub const FCTIW_NAN: u32 = 0x8000_0000;
