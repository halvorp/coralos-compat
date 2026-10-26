// Adapted from HexRaysSA/rax src/isa/arm/aarch32/instructions/misc.rs (exec_a32_parallel, exec_a32_pkh,
// exec_a32_extend, exec_a32_sat16) and the media decode in src/isa/arm/decoder/aarch32/mod.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! The ARMv6 media space (cccc 011x xxxx .... xxx1): SIMD-in-GPR add/subtract, packing, saturation,
//! sign/zero extension, byte reversal and SEL. The signed multiplies and USAD8 dispatch from here too.
//! The Thumb extend and reverse instructions reuse the helpers.

use crate::a32::{bit, bits};
use crate::alu::{signed_sat, unsigned_sat};
use crate::cpu::{Cpu, Step};

/// One lane of the parallel add/subtract: (prefix, a, b, subtract, lane width) -> (lane result, GE bit).
fn lane_op(prefix: u32, a: u32, b: u32, sub: bool, width: u32) -> (u32, Option<bool>) {
    let sh = 64 - width;
    let (avs, bvs) = (((a as i64) << sh) >> sh, ((b as i64) << sh) >> sh);
    let (avu, bvu) = (a as i64, b as i64);
    let signed = if sub { avs - bvs } else { avs + bvs };
    let unsigned = if sub { avu - bvu } else { avu + bvu };
    match prefix {
        0b001 => (signed as u32, Some(signed >= 0)), // S
        0b010 => (signed_sat(signed, width).0 as u32, None), // Q
        0b011 => ((signed >> 1) as u32, None), // SH
        0b101 => (unsigned as u32, Some(if sub { unsigned >= 0 } else { unsigned >= 1 << width })), // U
        0b110 => (unsigned_sat(unsigned, width).0 as u32, None), // UQ
        _ => ((unsigned >> 1) as u32, None), // UH
    }
}

/// ExtendAdd for the (S|U)XT(A)(B|H|B16) family: `size` 0 = B16, 2 = B, 3 = H.
pub(crate) fn extend(m: u32, rotation: u32, size: u32, unsigned: bool, add: Option<u32>) -> u32 {
    let rotated = m.rotate_right(rotation);
    let byte = |v: u32| if unsigned { v & 0xFF } else { v as u8 as i8 as u32 };
    match size {
        2 => add.unwrap_or(0).wrapping_add(byte(rotated)),
        3 => {
            let h = if unsigned { rotated & 0xFFFF } else { rotated as u16 as i16 as u32 };
            add.unwrap_or(0).wrapping_add(h)
        }
        _ => {
            let (lo, hi) = (byte(rotated) & 0xFFFF, byte(rotated >> 16) & 0xFFFF);
            let n = add.unwrap_or(0);
            ((n & 0xFFFF).wrapping_add(lo) & 0xFFFF) | (((n >> 16).wrapping_add(hi) & 0xFFFF) << 16)
        }
    }
}

pub(crate) fn rev16(v: u32) -> u32 {
    ((v & 0x00FF_00FF) << 8) | ((v >> 8) & 0x00FF_00FF)
}

pub(crate) fn revsh(v: u32) -> u32 {
    (v as u16).swap_bytes() as i16 as i32 as u32
}

impl Cpu {
    pub(crate) fn a32_media(&mut self, insn: u32) -> Step {
        let op1 = bits(insn, 24, 20);
        let op2 = bits(insn, 7, 5);
        match op1 {
            0b00001..=0b00011 | 0b00101..=0b00111 => self.a32_parallel(insn),
            0b01000..=0b01111 => self.a32_pack_sat_extend(insn),
            0b10000..=0b10111 => self.a32_signed_multiply(insn),
            0b11000 if op2 == 0 => self.a32_usad8(insn),
            _ => Err(self.undefined()), // SBFX/BFI/BFC/UBFX (ARMv6T2), UDF and the unallocated rest
        }
    }

    fn media_regs(&self, insn: u32) -> Result<(u32, u32, u32), crate::Exit> {
        let (rn, rd, rm) = (bits(insn, 19, 16), bits(insn, 15, 12), insn & 0xF);
        if rd == 15 || rm == 15 {
            return Err(self.unpredictable());
        }
        Ok((rn, rd, rm))
    }

    /// SADD16 ... UHSUB8: prefix in op1[2:0], operation in op2.
    fn a32_parallel(&mut self, insn: u32) -> Step {
        let (rn, rd, rm) = self.media_regs(insn)?;
        if rn == 15 {
            return Err(self.unpredictable());
        }
        if bits(insn, 11, 8) != 0xF {
            return Err(self.unpredictable()); // should-be-one field
        }
        let prefix = bits(insn, 22, 20);
        let (n, m) = (self.r(rn), self.r(rm));
        let h = |v: u32, i: u32| (v >> (16 * i)) & 0xFFFF;
        let b = |v: u32, i: u32| (v >> (8 * i)) & 0xFF;
        // (a, b, subtract) per lane, and the lane width.
        let (lanes, width): ([(u32, u32, bool); 4], u32) = match bits(insn, 7, 5) {
            0b000 => ([(h(n, 0), h(m, 0), false), (h(n, 1), h(m, 1), false), (0, 0, false), (0, 0, false)], 16),
            0b001 => ([(h(n, 0), h(m, 1), true), (h(n, 1), h(m, 0), false), (0, 0, false), (0, 0, false)], 16),
            0b010 => ([(h(n, 0), h(m, 1), false), (h(n, 1), h(m, 0), true), (0, 0, false), (0, 0, false)], 16),
            0b011 => ([(h(n, 0), h(m, 0), true), (h(n, 1), h(m, 1), true), (0, 0, false), (0, 0, false)], 16),
            0b100 => ([0, 1, 2, 3].map(|i| (b(n, i), b(m, i), false)), 8),
            0b111 => ([0, 1, 2, 3].map(|i| (b(n, i), b(m, i), true)), 8),
            _ => return Err(self.undefined()),
        };
        let count = 32 / width;
        let mask = (1u32 << width) - 1;
        let (mut result, mut ge, mut sets_ge) = (0u32, 0u8, false);
        for (i, &(a, bv, sub)) in lanes.iter().take(count as usize).enumerate() {
            let (val, g) = lane_op(prefix, a, bv, sub, width);
            result |= (val & mask) << (i as u32 * width);
            if let Some(g) = g {
                sets_ge = true;
                if g {
                    ge |= if width == 8 { 1 << i } else { 0b11 << (2 * i) };
                }
            }
        }
        if sets_ge {
            self.ge = ge;
        }
        self.set_r(rd, result);
        Ok(())
    }

    /// op1 = 01xxx: PKH, SSAT/USAT(16), the extends, REV/REV16/REVSH, SEL.
    fn a32_pack_sat_extend(&mut self, insn: u32) -> Step {
        let op1 = bits(insn, 22, 20);
        let op2 = bits(insn, 7, 5);
        let (rn, rd, rm) = self.media_regs(insn)?;
        if bit(insn, 21) && op2 & 1 == 0 {
            // SSAT (op1 0101x) / USAT (op1 0111x)
            return self.a32_saturate(insn);
        }
        // Should-be-one/zero fields: bits 11:8 of SEL/SSAT16/USAT16/REV*, bits 19:16 of REV*, the extends'
        // bits 9:8.
        let fixed_ok = match (op1, op2) {
            (0b000, 0b101) | (0b010 | 0b110, 0b001) => bits(insn, 11, 8) == 0xF,
            (0b011 | 0b111, 0b001 | 0b101) => bits(insn, 11, 8) == 0xF && rn == 0xF,
            (_, 0b011) => bits(insn, 9, 8) == 0,
            _ => true,
        };
        if !fixed_ok {
            return Err(self.unpredictable());
        }
        match (op1, op2) {
            (0b000, 0b000 | 0b010 | 0b100 | 0b110) => {
                // PKHBT (bit 6 = 0) / PKHTB (bit 6 = 1)
                if rn == 15 {
                    return Err(self.unpredictable());
                }
                let imm5 = bits(insn, 11, 7);
                let (n, m) = (self.r(rn), self.r(rm));
                let result = if bit(insn, 6) {
                    let shifted = ((m as i32) >> if imm5 == 0 { 31 } else { imm5 }) as u32;
                    (n & 0xFFFF_0000) | (shifted & 0xFFFF)
                } else {
                    ((m << imm5) & 0xFFFF_0000) | (n & 0xFFFF)
                };
                self.set_r(rd, result);
                Ok(())
            }
            (0b000, 0b101) => {
                // SEL: each byte from Rn where its GE bit is set, else from Rm.
                if rn == 15 {
                    return Err(self.unpredictable());
                }
                let (n, m) = (self.r(rn), self.r(rm));
                let mask = (0..4).fold(0u32, |acc, i| acc | if self.ge >> i & 1 != 0 { 0xFF << (8 * i) } else { 0 });
                self.set_r(rd, (n & mask) | (m & !mask));
                Ok(())
            }
            (0b010 | 0b110, 0b001) => {
                // SSAT16 / USAT16
                let n = self.r(rm);
                let sat = bits(insn, 19, 16);
                let unsigned = op1 == 0b110;
                let mut out = 0u32;
                for i in 0..2 {
                    let x = ((n >> (16 * i)) as u16 as i16) as i64;
                    let (v, s) = if unsigned { unsigned_sat(x, sat) } else { signed_sat(x, sat + 1) };
                    self.q |= s;
                    out |= (v as u32 & 0xFFFF) << (16 * i);
                }
                self.set_r(rd, out);
                Ok(())
            }
            (_, 0b011) => {
                // (S|U)XTA(B16|B|H) / (S|U)XT(B16|B|H) when Rn == 15
                let size = match op1 & 0b011 {
                    0b000 => 0,
                    0b010 => 2,
                    0b011 => 3,
                    _ => return Err(self.undefined()),
                };
                let add = if rn == 15 { None } else { Some(self.r(rn)) };
                let value = extend(self.r(rm), bits(insn, 11, 10) * 8, size, op1 & 0b100 != 0, add);
                self.set_r(rd, value);
                Ok(())
            }
            (0b011, 0b001) => self.set_rd_checked(rd, self.r(rm).swap_bytes()), // REV
            (0b011, 0b101) => self.set_rd_checked(rd, rev16(self.r(rm))),       // REV16
            (0b111, 0b101) => self.set_rd_checked(rd, revsh(self.r(rm))),       // REVSH
            _ => Err(self.undefined()), // RBIT (ARMv6T2) and the unallocated rest
        }
    }

    fn set_rd_checked(&mut self, rd: u32, value: u32) -> Step {
        self.set_r(rd, value);
        Ok(())
    }

    /// SSAT Rd, #sat, Rn{, shift} (cccc 0110 101s ssss dddd iiii ix01 nnnn) and USAT (0110 111s).
    fn a32_saturate(&mut self, insn: u32) -> Step {
        let rd = bits(insn, 15, 12);
        let rn = insn & 0xF;
        if rd == 15 || rn == 15 {
            return Err(self.unpredictable());
        }
        let imm5 = bits(insn, 11, 7);
        let n = self.r(rn);
        let operand = if bit(insn, 6) {
            ((n as i32) >> if imm5 == 0 { 31 } else { imm5 }) as i64 // ASR #32 equals ASR #31 here
        } else {
            (n << imm5) as i32 as i64
        };
        let sat = bits(insn, 20, 16);
        let (value, saturated) = if bit(insn, 22) { unsigned_sat(operand, sat) } else { signed_sat(operand, sat + 1) };
        self.q |= saturated;
        self.set_r(rd, value as u32);
        Ok(())
    }
}
