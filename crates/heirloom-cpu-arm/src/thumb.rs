// Adapted from HexRaysSA/rax src/isa/arm/decoder/thumb.rs (the 16-bit decode tree) and the Thumb paths of
// src/isa/arm/aarch32/instructions/{data,memory,control}.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! ARMv6 Thumb (Thumb-1): the 16-bit set plus the BL/BLX prefix-suffix pair. Every other 32-bit Thumb-2
//! encoding, CBZ/CBNZ and IT are ARMv6T2 and trap as Undefined.

use crate::access::Kind;
use crate::alu::{add_with_carry, shift_c, Shift};
use crate::cpu::{Cpu, Step};
use crate::media::{extend, rev16, revsh};
use crate::{Exit, Memory};

#[inline]
fn f(hw: u16, hi: u32, lo: u32) -> u32 {
    (hw as u32 >> lo) & ((1 << (hi - lo + 1)) - 1)
}

impl Cpu {
    pub(crate) fn thumb16<M: Memory>(&mut self, mem: &mut M, hw: u16) -> Step {
        match hw >> 11 {
            0b00000..=0b00010 => {
                // LSL/LSR/ASR (immediate); LSL #0 is MOVS Rd, Rm
                let (rm, rd) = (f(hw, 5, 3), f(hw, 2, 0));
                let op = f(hw, 12, 11);
                let (shift, amount) = crate::alu::decode_imm_shift(op, f(hw, 10, 6));
                let (value, carry) = shift_c(self.r(rm), shift, amount, self.c);
                self.set_nz(value);
                self.c = carry;
                self.set_r(rd, value);
                Ok(())
            }
            0b00011 => {
                // ADD/SUB register or 3-bit immediate
                let (rn, rd) = (f(hw, 5, 3), f(hw, 2, 0));
                let operand = if hw & (1 << 10) != 0 { f(hw, 8, 6) } else { self.r(f(hw, 8, 6)) };
                let sub = hw & (1 << 9) != 0;
                let result = self.add_sub_flags(self.r(rn), operand, sub);
                self.set_r(rd, result);
                Ok(())
            }
            0b00100..=0b00111 => {
                // MOV/CMP/ADD/SUB Rdn, #imm8
                let rdn = f(hw, 10, 8);
                let imm = f(hw, 7, 0);
                match f(hw, 12, 11) {
                    0 => {
                        self.set_nz(imm);
                        self.set_r(rdn, imm);
                    }
                    1 => {
                        self.add_sub_flags(self.r(rdn), imm, true);
                    }
                    op => {
                        let result = self.add_sub_flags(self.r(rdn), imm, op == 3);
                        self.set_r(rdn, result);
                    }
                }
                Ok(())
            }
            0b01000 => {
                if hw & (1 << 10) == 0 {
                    self.thumb_data_processing(hw)
                } else {
                    self.thumb_special(hw)
                }
            }
            0b01001 => {
                // LDR Rt, [PC, #imm8*4]
                let addr = (self.r(15) & !3).wrapping_add(f(hw, 7, 0) * 4);
                let value = self.ld32(mem, addr, Kind::Plain)?;
                self.set_r(f(hw, 10, 8), value);
                Ok(())
            }
            0b01010 | 0b01011 => {
                let addr = self.r(f(hw, 5, 3)).wrapping_add(self.r(f(hw, 8, 6)));
                self.thumb_load_store(mem, f(hw, 11, 9), addr, f(hw, 2, 0))
            }
            0b01100..=0b01111 => {
                // STR/LDR/STRB/LDRB Rt, [Rn, #imm5(*4)]
                let byte = hw & (1 << 12) != 0;
                let imm = f(hw, 10, 6) * if byte { 1 } else { 4 };
                let addr = self.r(f(hw, 5, 3)).wrapping_add(imm);
                let op = match (byte, hw & (1 << 11) != 0) {
                    (false, false) => 0b000,
                    (false, true) => 0b100,
                    (true, false) => 0b010,
                    (true, true) => 0b110,
                };
                self.thumb_load_store(mem, op, addr, f(hw, 2, 0))
            }
            0b10000 | 0b10001 => {
                // STRH/LDRH Rt, [Rn, #imm5*2]
                let addr = self.r(f(hw, 5, 3)).wrapping_add(f(hw, 10, 6) * 2);
                let op = if hw & (1 << 11) != 0 { 0b101 } else { 0b001 };
                self.thumb_load_store(mem, op, addr, f(hw, 2, 0))
            }
            0b10010 | 0b10011 => {
                // STR/LDR Rt, [SP, #imm8*4]
                let addr = self.r(13).wrapping_add(f(hw, 7, 0) * 4);
                let op = if hw & (1 << 11) != 0 { 0b100 } else { 0b000 };
                self.thumb_load_store(mem, op, addr, f(hw, 10, 8))
            }
            0b10100 => {
                // ADR Rd, label
                let value = (self.r(15) & !3).wrapping_add(f(hw, 7, 0) * 4);
                self.set_r(f(hw, 10, 8), value);
                Ok(())
            }
            0b10101 => {
                // ADD Rd, SP, #imm8*4
                let value = self.r(13).wrapping_add(f(hw, 7, 0) * 4);
                self.set_r(f(hw, 10, 8), value);
                Ok(())
            }
            0b10110 | 0b10111 => self.thumb_misc(mem, hw),
            0b11000 | 0b11001 => {
                // STMIA/LDMIA Rn!, {list}
                let rn = f(hw, 10, 8);
                let list = f(hw, 7, 0);
                let load = hw & (1 << 11) != 0;
                if list == 0 {
                    return Err(self.unpredictable());
                }
                let base = self.r(rn);
                let new_base = base.wrapping_add(4 * list.count_ones());
                // LDMIA writes back only when Rn is not in the list.
                let wback = !load || list & (1 << rn) == 0;
                self.transfer_multiple(mem, rn, list, base, load, wback.then_some(new_base))
            }
            0b11010 | 0b11011 => {
                let cond = f(hw, 11, 8);
                match cond {
                    0xE => Err(self.undefined()), // UDF
                    0xF => Err(Exit::Svc(f(hw, 7, 0))),
                    _ => {
                        if crate::alu::condition_passed(cond, self.n, self.z, self.c, self.v) {
                            let offset = ((f(hw, 7, 0) << 24) as i32 >> 23) as u32;
                            self.regs[15] = self.r(15).wrapping_add(offset);
                        }
                        Ok(())
                    }
                }
            }
            0b11100 => {
                let offset = ((f(hw, 10, 0) << 21) as i32 >> 20) as u32;
                self.regs[15] = self.r(15).wrapping_add(offset);
                Ok(())
            }
            0b11110 => self.thumb_bl_prefix(mem, hw),
            0b11101 | 0b11111 => self.thumb_bl_suffix(hw),
            _ => unreachable!("hw >> 11 is five bits"),
        }
    }

    /// ADDS/SUBS with the arithmetic flags; returns the result.
    fn add_sub_flags(&mut self, a: u32, b: u32, sub: bool) -> u32 {
        let (result, carry, overflow) =
            if sub { add_with_carry(a, !b, true) } else { add_with_carry(a, b, false) };
        self.set_nz(result);
        self.c = carry;
        self.v = overflow;
        result
    }

    /// 010000 oooo mmm ddd: the sixteen two-register operations, all flag-setting.
    fn thumb_data_processing(&mut self, hw: u16) -> Step {
        let (rm, rdn) = (f(hw, 5, 3), f(hw, 2, 0));
        let (a, b) = (self.r(rdn), self.r(rm));
        let logical = |cpu: &mut Self, value: u32| {
            cpu.set_nz(value);
            value
        };
        let shifted = |cpu: &mut Self, shift: Shift| {
            let (value, carry) = shift_c(a, shift, b & 0xFF, cpu.c);
            cpu.c = carry;
            cpu.set_nz(value);
            value
        };
        let result = match f(hw, 9, 6) {
            0x0 => logical(self, a & b),
            0x1 => logical(self, a ^ b),
            0x2 => shifted(self, Shift::Lsl),
            0x3 => shifted(self, Shift::Lsr),
            0x4 => shifted(self, Shift::Asr),
            0x5 => self.adc_sbc(a, b, false),
            0x6 => self.adc_sbc(a, b, true),
            0x7 => shifted(self, Shift::Ror),
            0x8 => {
                logical(self, a & b);
                return Ok(()); // TST
            }
            0x9 => self.add_sub_flags(0, b, true), // RSBS Rd, Rm, #0 (NEG)
            0xA => {
                self.add_sub_flags(a, b, true);
                return Ok(()); // CMP
            }
            0xB => {
                self.add_sub_flags(a, b, false);
                return Ok(()); // CMN
            }
            0xC => logical(self, a | b),
            0xD => logical(self, a.wrapping_mul(b)), // MULS: C and V unchanged on ARMv6
            0xE => logical(self, a & !b),
            _ => logical(self, !b),
        };
        self.set_r(rdn, result);
        Ok(())
    }

    fn adc_sbc(&mut self, a: u32, b: u32, sub: bool) -> u32 {
        let (result, carry, overflow) = add_with_carry(a, if sub { !b } else { b }, self.c);
        self.set_nz(result);
        self.c = carry;
        self.v = overflow;
        result
    }

    /// 010001 oo D mmmm ddd: ADD/CMP/MOV with high registers, BX/BLX Rm.
    fn thumb_special(&mut self, hw: u16) -> Step {
        let rm = f(hw, 6, 3);
        let rd = f(hw, 2, 0) | (f(hw, 7, 7) << 3);
        match f(hw, 9, 8) {
            0b00 => {
                let value = self.r(rd).wrapping_add(self.r(rm));
                self.write_alu(rd, value);
                Ok(())
            }
            0b01 => {
                if rd < 8 && rm < 8 || rd == 15 || rm == 15 {
                    return Err(self.unpredictable());
                }
                self.add_sub_flags(self.r(rd), self.r(rm), true);
                Ok(())
            }
            0b10 => {
                let value = self.r(rm);
                self.write_alu(rd, value);
                Ok(())
            }
            _ => {
                // BX Rm (bit 7 = 0) / BLX Rm (bit 7 = 1); bits 2:0 should be zero
                if f(hw, 2, 0) != 0 {
                    return Err(self.unpredictable());
                }
                let target = self.r(rm);
                if hw & (1 << 7) != 0 {
                    if rm == 15 {
                        return Err(self.unpredictable());
                    }
                    if target & 3 == 2 {
                        return Err(self.unpredictable());
                    }
                    self.set_r(14, self.cur.wrapping_add(2) | 1);
                }
                self.bx_write_pc(target)
            }
        }
    }

    /// The register- and immediate-offset single transfers. `op` uses the 0101 ooo encoding:
    /// STR, STRH, STRB, LDRSB, LDR, LDRH, LDRB, LDRSH.
    fn thumb_load_store<M: Memory>(&mut self, mem: &mut M, op: u32, addr: u32, rt: u32) -> Step {
        let value = match op {
            0b000 => return self.st32(mem, addr, self.r(rt), Kind::Plain),
            0b001 => return self.st16(mem, addr, self.r(rt), Kind::Plain),
            0b010 => return self.st8(mem, addr, self.r(rt)),
            0b011 => self.ld8(mem, addr)? as u8 as i8 as u32,
            0b100 => self.ld32(mem, addr, Kind::Plain)?,
            0b101 => self.ld16(mem, addr, Kind::Plain)?,
            0b110 => self.ld8(mem, addr)?,
            _ => self.ld16(mem, addr, Kind::Plain)? as u16 as i16 as u32,
        };
        self.set_r(rt, value);
        Ok(())
    }

    /// 1011 xxxx: SP adjust, the ARMv6 extends and reverses, PUSH/POP, SETEND, CPS, BKPT.
    fn thumb_misc<M: Memory>(&mut self, mem: &mut M, hw: u16) -> Step {
        let (rm, rd) = (f(hw, 5, 3), f(hw, 2, 0));
        match f(hw, 11, 8) {
            0b0000 => {
                let imm = f(hw, 6, 0) * 4;
                let sp = self.r(13);
                self.set_r(13, if hw & (1 << 7) != 0 { sp.wrapping_sub(imm) } else { sp.wrapping_add(imm) });
                Ok(())
            }
            0b0010 => {
                // SXTH, SXTB, UXTH, UXTB
                let (size, unsigned) = match f(hw, 7, 6) {
                    0 => (3, false),
                    1 => (2, false),
                    2 => (3, true),
                    _ => (2, true),
                };
                self.set_r(rd, extend(self.r(rm), 0, size, unsigned, None));
                Ok(())
            }
            0b0100 | 0b0101 => {
                // PUSH {list, LR?}
                let list = f(hw, 7, 0) | (f(hw, 8, 8) << 14);
                if list == 0 {
                    return Err(self.unpredictable());
                }
                let sp = self.r(13);
                let new_sp = sp.wrapping_sub(4 * list.count_ones());
                self.transfer_multiple(mem, 13, list, new_sp, false, Some(new_sp))
            }
            0b0110 => match f(hw, 7, 4) {
                0b0101 if f(hw, 2, 0) == 0 => {
                    self.big_endian = hw & (1 << 3) != 0; // SETEND
                    Ok(())
                }
                0b0110 | 0b0111 if hw & (1 << 3) == 0 => Ok(()), // CPS: no effect in user mode
                _ => Err(self.undefined()),
            },
            0b1010 => {
                let value = self.r(rm);
                let result = match f(hw, 7, 6) {
                    0 => value.swap_bytes(),
                    1 => rev16(value),
                    3 => revsh(value),
                    _ => return Err(self.undefined()),
                };
                self.set_r(rd, result);
                Ok(())
            }
            0b1100 | 0b1101 => {
                // POP {list, PC?}
                let list = f(hw, 7, 0) | (f(hw, 8, 8) << 15);
                if list == 0 {
                    return Err(self.unpredictable());
                }
                let sp = self.r(13);
                self.transfer_multiple(mem, 13, list, sp, true, Some(sp.wrapping_add(4 * list.count_ones())))
            }
            0b1110 => Err(Exit::Bkpt(f(hw, 7, 0) as u16)),
            _ => Err(self.undefined()), // CBZ/CBNZ, IT and the hints are ARMv6T2
        }
    }

    /// BL/BLX prefix (11110 imm11): LR = PC + (imm11 << 12). When the next halfword is the matching
    /// suffix it runs in the same step, so the pair retires as one instruction.
    fn thumb_bl_prefix<M: Memory>(&mut self, mem: &mut M, hw: u16) -> Step {
        let offset = ((f(hw, 10, 0) << 21) as i32 >> 9) as u32;
        let lr = self.r(15).wrapping_add(offset);
        let next = self.cur.wrapping_add(2);
        if let Ok(suffix) = mem.fetch_u16(next) {
            if matches!(suffix >> 11, 0b11101 | 0b11111) && !(suffix >> 11 == 0b11101 && suffix & 1 != 0) {
                self.set_r(14, lr);
                self.cur = next;
                self.insn = suffix as u32;
                self.regs[15] = next.wrapping_add(2);
                return self.thumb_bl_suffix(suffix);
            }
        }
        self.set_r(14, lr);
        Ok(())
    }

    /// BL suffix (11111 imm11) / BLX suffix (11101 imm10 0).
    fn thumb_bl_suffix(&mut self, hw: u16) -> Step {
        let target = self.r(14).wrapping_add(f(hw, 10, 0) << 1);
        let ret = self.cur.wrapping_add(2) | 1;
        if hw >> 11 == 0b11101 {
            if hw & 1 != 0 {
                return Err(self.undefined());
            }
            self.set_r(14, ret);
            self.thumb = false;
            self.regs[15] = target & !3;
        } else {
            self.set_r(14, ret);
            self.regs[15] = target & !1;
        }
        Ok(())
    }
}
