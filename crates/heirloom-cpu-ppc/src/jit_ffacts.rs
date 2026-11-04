//! Tier 2's FPR FACTS (h86 jitsize increment 2): what the code emitted so far in a block (or region node) proves about FPRs
//! and FPSCR[RN], so later FP forms skip tests that cannot fail.
//! - SINGLE (bit r): FPR r holds a value tier 0's fused multiply-add takes as single-representable (a single's value, or not
//!   finite) - fmadds skips that operand's round trip.
//! - EXACT (bit r): FPR r holds exactly a single that is not a signalling NaN - stfs stores cvtsd2ss of it (the PowerPC's bit
//!   selection gives the same bits) without the round trip.
//! - RN: FPSCR[RN] is 0 - an FP form skips its RN test.
//! A fact holds on the straight-line path after the instruction that made it, because that instruction's slow paths do not
//! come back (they LEAVE after tier 1's handler: jit_emit `slow_leave`) - its fast path is the only way on, and the fast path
//! writes a finite single (lfs: a non-NaN single) and passed the RN test. Every other write to an FPR clears its facts;
//! tier 1's handler on the hot path (a Generic, an FP form with no inline code) clears all of them; a block / node starts
//! with none. Slow paths of other forms (loads, stores, integer forms) run one instruction that writes no FPR and FPSCR[RN].
use crate::t1op::{Op, K};

/// What instruction `op` does to the facts when its inline code ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fx {
    /// No FPR written.
    None,
    /// Writes a finite single to FPR d on its only way on (SINGLE + EXACT), and tested FPSCR[RN] unless `lfs`.
    Single { d: u8, rn: bool },
    /// FPR d = a sign change of FPR s (fmr / fneg / fabs / fnabs): s's facts move to d.
    Copy { d: u8, s: u8 },
    /// Writes FPR d with anything else; `rn` = its fast path passed the RN test (double arithmetic).
    Kill { d: u8, rn: bool },
}

/// The facts' effect of `op` (its inline form; tier 1's handler on the hot path clears everything instead).
pub(crate) fn effect(op: &Op) -> Fx {
    let w = op.w;
    let (rt, rb, rc) = (((w >> 21) & 31) as u8, ((w >> 11) & 31) as u8, w & 1 != 0);
    let (xo5, xo10) = ((w >> 1) & 31, (w >> 1) & 0x3FF);
    match op.k {
        K::Lfs => Fx::Single { d: op.d, rn: false },
        K::Lfd => Fx::Kill { d: op.d, rn: false },
        K::Fp59 if !rc && matches!(xo5, 18 | 20 | 21 | 25 | 28..=31) => Fx::Single { d: rt, rn: true },
        K::Fp59 => Fx::Kill { d: rt, rn: false },
        K::Fp63 if matches!(xo10, 0 | 32) => Fx::None,
        K::Fp63 if !rc && matches!(xo10, 40 | 72 | 136 | 264) => Fx::Copy { d: rt, s: rb },
        K::Fp63 if !rc && xo10 == 12 => Fx::Single { d: rt, rn: true },
        K::Fp63 if !rc && matches!(xo5, 18 | 20 | 21 | 25) => Fx::Kill { d: rt, rn: true },
        K::Fp63 => Fx::Kill { d: rt, rn: false },
        // the X-form FP loads tier 2 runs inline (lfsx / lfsux / lfdx / lfdux)
        K::Generic if w >> 26 == 31 && matches!(xo10, 535 | 567 | 599 | 631) => Fx::Kill { d: rt, rn: false },
        _ => Fx::None,
    }
}

/// The forms whose slow paths must leave (their Fx makes a fact on the fast path only).
pub(crate) fn leaves(fx: Fx) -> bool { matches!(fx, Fx::Single { .. } | Fx::Kill { rn: true, .. }) }

/// The facts at one point of a block.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Facts { pub(crate) single: u32, pub(crate) exact: u32, pub(crate) rn: bool }

impl Facts {
    pub(crate) fn apply(&mut self, fx: Fx) {
        match fx {
            Fx::None => {}
            Fx::Single { d, rn } => { self.single |= 1 << d; self.exact |= 1 << d; self.rn |= rn; }
            Fx::Copy { d, s } => {
                let bit = |m: u32| (m >> s & 1) << d;
                self.single = self.single & !(1 << d) | bit(self.single);
                self.exact = self.exact & !(1 << d) | bit(self.exact);
            }
            Fx::Kill { d, rn } => { self.single &= !(1 << d); self.exact &= !(1 << d); self.rn |= rn; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fp(op: u32, d: u32, a: u32, b: u32, c: u32, xo: u32) -> u32 { op << 26 | d << 21 | a << 16 | b << 11 | c << 6 | xo << 1 }

    #[test]
    fn single_results_make_facts_moves_carry_them_and_other_writes_clear_them() {
        let k = |k: K, w: u32, d: u8| Op { k, d, a: 0, b: 0, imm: 0, w };
        let mut f = Facts::default();
        f.apply(effect(&k(K::Lfs, 48 << 26 | 1 << 21, 1)));
        assert_eq!(f, Facts { single: 1 << 1, exact: 1 << 1, rn: false }, "lfs: a single, RN untested");
        f.apply(effect(&k(K::Fp59, fp(59, 2, 1, 1, 1, 29), 0)));
        assert_eq!(f, Facts { single: 0b110, exact: 0b110, rn: true }, "fmadds f2: a single, RN tested");
        f.apply(effect(&k(K::Fp63, fp(63, 3, 0, 2, 0, 40), 0)));
        assert_eq!(f.single, 0b1110, "fneg f3, f2 carries f2's facts");
        f.apply(effect(&k(K::Fp63, fp(63, 2, 1, 3, 0, 21), 0)));
        assert_eq!((f.single, f.exact), (0b1010, 0b1010), "fadd (double) f2 clears f2");
        f.apply(effect(&k(K::Lfd, 50 << 26 | 1 << 21, 1)));
        assert_eq!(f.single, 0b1000, "lfd f1 clears f1");
        assert!(leaves(effect(&k(K::Fp59, fp(59, 2, 1, 1, 1, 21), 0))) && !leaves(effect(&k(K::Fp63, fp(63, 0, 1, 2, 0, 0), 0))));
        assert_eq!(effect(&k(K::Fp59, fp(59, 2, 1, 1, 1, 21) | 1, 0)), Fx::Kill { d: 2, rn: false }, "fadds. is tier 1's");
    }
}
