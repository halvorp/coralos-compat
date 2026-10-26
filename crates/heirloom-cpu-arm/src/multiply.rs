// Adapted from HexRaysSA/rax src/isa/arm/aarch32/instructions/misc.rs (exec_a32_hmul, exec_a32_dual,
// exec_a32_smmul, exec_a32_usad) and data.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! Multiplies: MUL/MLA, the long multiplies and UMAAL, the ARMv5TE halfword multiplies and the ARMv6
//! dual, most-significant-word and sum-of-absolute-differences multiplies.

use crate::a32::{bit, bits};
use crate::cpu::{Cpu, Step};

fn half(v: u32, top: bool) -> i64 {
    (if top { v >> 16 } else { v }) as u16 as i16 as i64
}

impl Cpu {
    fn any_pc(&self, regs: &[u32]) -> bool {
        regs.contains(&15)
    }

    /// cccc 0000 xxxx .... 1001: MUL, MLA, UMAAL, UMULL, UMLAL, SMULL, SMLAL.
    pub(crate) fn a32_multiply(&mut self, insn: u32) -> Step {
        let (hi, lo, rm, rn) = (bits(insn, 19, 16), bits(insn, 15, 12), bits(insn, 11, 8), insn & 0xF);
        let s = bit(insn, 20);
        let op = bits(insn, 23, 21);
        match op {
            0b000 | 0b001 => {
                // MUL Rd(hi), Rn, Rm / MLA Rd, Rn, Rm, Ra(lo)
                let acc = op == 0b001;
                if self.any_pc(&[hi, rm, rn]) || (acc && lo == 15) || (!acc && lo != 0) {
                    return Err(self.unpredictable());
                }
                let mut result = self.r(rn).wrapping_mul(self.r(rm));
                if acc {
                    result = result.wrapping_add(self.r(lo));
                }
                if s {
                    self.set_nz(result);
                }
                self.set_r(hi, result);
                Ok(())
            }
            0b010 if !s => {
                // UMAAL RdLo, RdHi, Rn, Rm
                if self.any_pc(&[hi, lo, rm, rn]) || hi == lo {
                    return Err(self.unpredictable());
                }
                let result = self.r(rn) as u64 * self.r(rm) as u64 + self.r(hi) as u64 + self.r(lo) as u64;
                self.set_r(lo, result as u32);
                self.set_r(hi, (result >> 32) as u32);
                Ok(())
            }
            0b100..=0b111 => {
                if self.any_pc(&[hi, lo, rm, rn]) || hi == lo {
                    return Err(self.unpredictable());
                }
                let signed = op & 2 != 0;
                let accumulate = op & 1 != 0;
                let product = if signed {
                    (self.r(rn) as i32 as i64).wrapping_mul(self.r(rm) as i32 as i64) as u64
                } else {
                    self.r(rn) as u64 * self.r(rm) as u64
                };
                let acc = if accumulate { (self.r(hi) as u64) << 32 | self.r(lo) as u64 } else { 0 };
                let result = product.wrapping_add(acc);
                if s {
                    self.n = result >> 63 != 0;
                    self.z = result == 0;
                }
                self.set_r(lo, result as u32);
                self.set_r(hi, (result >> 32) as u32);
                Ok(())
            }
            _ => Err(self.undefined()), // 0101 undefined, 0110 MLS (ARMv6T2), 0111 undefined
        }
    }

    /// cccc 0001 0xx0 .... 1yx0: SMLA<x><y>, SMLAW<y>, SMULW<y>, SMLAL<x><y>, SMUL<x><y>.
    pub(crate) fn a32_halfword_multiply(&mut self, insn: u32) -> Step {
        let (rd, ra, rm, rn) = (bits(insn, 19, 16), bits(insn, 15, 12), bits(insn, 11, 8), insn & 0xF);
        let (n_top, m_top) = (bit(insn, 5), bit(insn, 6));
        let (nv, mv) = (self.r(rn), self.r(rm));
        let op = bits(insn, 22, 21);
        let uses_ra = match op {
            0b00 | 0b10 => true,
            0b01 => !n_top, // bit 5 clear: SMLAW
            _ => false,
        };
        // SMUL<x><y> and SMULW<y> have Ra as a should-be-zero field.
        if self.any_pc(&[rd, rm, rn]) || (uses_ra && ra == 15) || (!uses_ra && ra != 0) {
            return Err(self.unpredictable());
        }
        match op {
            0b00 => {
                let result = half(nv, n_top) * half(mv, m_top) + self.r(ra) as i32 as i64;
                self.q |= result != result as i32 as i64;
                self.set_r(rd, result as u32);
            }
            0b01 => {
                let product = (nv as i32 as i64 * half(mv, m_top)) >> 16;
                if n_top {
                    self.set_r(rd, product as u32); // SMULW<y>
                } else {
                    let result = product + self.r(ra) as i32 as i64;
                    self.q |= result != result as i32 as i64;
                    self.set_r(rd, result as u32);
                }
            }
            0b10 => {
                // SMLAL<x><y> RdLo(ra), RdHi(rd)
                if rd == ra {
                    return Err(self.unpredictable());
                }
                let acc = ((self.r(rd) as u64) << 32 | self.r(ra) as u64) as i64;
                let result = acc.wrapping_add(half(nv, n_top) * half(mv, m_top)) as u64;
                self.set_r(ra, result as u32);
                self.set_r(rd, (result >> 32) as u32);
            }
            _ => self.set_r(rd, (half(nv, n_top) * half(mv, m_top)) as u32),
        }
        Ok(())
    }

    /// cccc 0111 0xxx: SMLAD/SMUAD/SMLSD/SMUSD, SMLALD/SMLSLD, SMMLA/SMMUL/SMMLS.
    pub(crate) fn a32_signed_multiply(&mut self, insn: u32) -> Step {
        let (rd, ra, rm, rn) = (bits(insn, 19, 16), bits(insn, 15, 12), bits(insn, 11, 8), insn & 0xF);
        let op1 = bits(insn, 22, 20);
        let op2 = bits(insn, 7, 5);
        if self.any_pc(&[rd, rm, rn]) {
            return Err(self.unpredictable());
        }
        let nv = self.r(rn);
        match (op1, op2) {
            (0b000, 0b000..=0b011) => {
                // SMLAD / SMLSD (Ra != 15), SMUAD / SMUSD (Ra == 15); bit 5 swaps Rm's halves.
                let mv = if bit(insn, 5) { self.r(rm).rotate_right(16) } else { self.r(rm) };
                let p1 = half(nv, false) * half(mv, false);
                let p2 = half(nv, true) * half(mv, true);
                let mut result = if bit(insn, 6) { p1 - p2 } else { p1 + p2 };
                if ra != 15 {
                    result += self.r(ra) as i32 as i64;
                }
                self.q |= result != result as i32 as i64;
                self.set_r(rd, result as u32);
                Ok(())
            }
            (0b100, 0b000..=0b011) => {
                // SMLALD / SMLSLD RdLo(ra), RdHi(rd)
                if ra == 15 || ra == rd {
                    return Err(self.unpredictable());
                }
                let mv = if bit(insn, 5) { self.r(rm).rotate_right(16) } else { self.r(rm) };
                let p1 = half(nv, false) * half(mv, false);
                let p2 = half(nv, true) * half(mv, true);
                let acc = ((self.r(rd) as u64) << 32 | self.r(ra) as u64) as i64;
                let result = acc.wrapping_add(if bit(insn, 6) { p1 - p2 } else { p1 + p2 }) as u64;
                self.set_r(ra, result as u32);
                self.set_r(rd, (result >> 32) as u32);
                Ok(())
            }
            (0b101, 0b000 | 0b001 | 0b110 | 0b111) => {
                // SMMLA / SMMUL (Ra == 15) / SMMLS; bit 5 rounds.
                let subtract = bit(insn, 7);
                if subtract && ra == 15 {
                    return Err(self.unpredictable());
                }
                let product = nv as i32 as i64 * self.r(rm) as i32 as i64;
                let acc = if ra == 15 { 0 } else { (self.r(ra) as i64) << 32 };
                let mut result = if subtract { acc.wrapping_sub(product) } else { acc.wrapping_add(product) };
                if bit(insn, 5) {
                    result = result.wrapping_add(0x8000_0000);
                }
                self.set_r(rd, (result >> 32) as u32);
                Ok(())
            }
            _ => Err(self.undefined()), // SDIV/UDIV (ARMv7) and the unallocated rest
        }
    }

    /// USAD8 / USADA8 (cccc 0111 1000 dddd aaaa mmmm 0001 nnnn).
    pub(crate) fn a32_usad8(&mut self, insn: u32) -> Step {
        let (rd, ra, rm, rn) = (bits(insn, 19, 16), bits(insn, 15, 12), bits(insn, 11, 8), insn & 0xF);
        if self.any_pc(&[rd, rm, rn]) {
            return Err(self.unpredictable());
        }
        let (n, m) = (self.r(rn), self.r(rm));
        let mut sum = (0..4).fold(0u32, |acc, i| {
            let a = (n >> (i * 8)) & 0xFF;
            let b = (m >> (i * 8)) & 0xFF;
            acc + a.abs_diff(b)
        });
        if ra != 15 {
            sum = sum.wrapping_add(self.r(ra));
        }
        self.set_r(rd, sum);
        Ok(())
    }
}
