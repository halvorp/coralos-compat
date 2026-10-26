// Adapted from HexRaysSA/rax src/isa/arm/decoder/aarch32/mod.rs (the A32 decode tree) and
// src/isa/arm/aarch32/instructions/{data,control,misc}.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! The A32 decode tree restricted to ARMv6K (ARM1136/ARM1176), data processing with every shifter
//! operand, branches, the status-register transfers, the misc space and the unconditional space.

use crate::alu::{add_with_carry, decode_imm_shift, expand_imm_c, shift_c, Shift};
use crate::cpu::{Cpu, Step};
use crate::{Exit, Memory, TrapKind};

#[inline]
pub(crate) fn bits(insn: u32, hi: u32, lo: u32) -> u32 {
    (insn >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

#[inline]
pub(crate) fn bit(insn: u32, n: u32) -> bool {
    (insn >> n) & 1 != 0
}

impl Cpu {
    /// A conditional (cond != 1111) instruction whose condition passed.
    pub(crate) fn a32<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        match bits(insn, 27, 25) {
            0b000 => {
                if bit(insn, 4) && bit(insn, 7) {
                    if bits(insn, 6, 5) == 0 {
                        if bit(insn, 24) {
                            self.a32_sync(mem, insn)
                        } else {
                            self.a32_multiply(insn)
                        }
                    } else {
                        self.a32_extra_load_store(mem, insn)
                    }
                } else if bits(insn, 24, 23) == 0b10 && !bit(insn, 20) {
                    if bit(insn, 7) {
                        self.a32_halfword_multiply(insn)
                    } else {
                        self.a32_misc(insn)
                    }
                } else {
                    self.a32_data_register(insn)
                }
            }
            0b001 => {
                if bits(insn, 24, 23) == 0b10 && !bit(insn, 20) {
                    // MSR (immediate) and the hints; 10000/10100 are MOVW/MOVT (ARMv6T2).
                    if bit(insn, 21) && !bit(insn, 22) && bits(insn, 19, 16) == 0 {
                        if bits(insn, 15, 8) != 0xF0 {
                            return Err(self.unpredictable()); // SBO/SBZ fields of the hint space
                        }
                        self.a32_hint(insn)
                    } else if bit(insn, 21) {
                        if bits(insn, 15, 12) != 0xF {
                            return Err(self.unpredictable()); // MSR's should-be-one field
                        }
                        self.a32_msr(bit(insn, 22), bits(insn, 19, 16), expand_imm_c(insn & 0xFFF, self.c).0)
                    } else {
                        Err(self.undefined())
                    }
                } else {
                    let (imm, carry) = expand_imm_c(insn & 0xFFF, self.c);
                    self.data_processing(insn, imm, carry)
                }
            }
            0b010 => self.a32_load_store_word(mem, insn),
            0b011 => {
                if bit(insn, 4) {
                    self.a32_media(insn)
                } else {
                    self.a32_load_store_word(mem, insn)
                }
            }
            0b100 => self.a32_load_store_multiple(mem, insn),
            0b101 => {
                let offset = ((insn << 8) as i32 >> 6) as u32;
                let target = self.r(15).wrapping_add(offset);
                if bit(insn, 24) {
                    self.set_r(14, self.cur.wrapping_add(4));
                }
                self.branch_write_pc(target);
                Ok(())
            }
            0b110 => self.a32_coprocessor_load_store(mem, insn),
            _ => {
                if bit(insn, 24) {
                    Err(Exit::Svc(insn & 0x00FF_FFFF))
                } else {
                    self.a32_coprocessor(insn)
                }
            }
        }
    }

    /// Data processing (register) and (register-shifted register).
    fn a32_data_register(&mut self, insn: u32) -> Step {
        let rm = insn & 0xF;
        let ty = bits(insn, 6, 5);
        let (value, carry) = if bit(insn, 4) {
            // Register-shifted register: any of Rd/Rn/Rm/Rs = PC is UNPREDICTABLE.
            let rs = bits(insn, 11, 8);
            if rm == 15 || rs == 15 || bits(insn, 15, 12) == 15 || bits(insn, 19, 16) == 15 {
                return Err(self.unpredictable());
            }
            shift_c(self.r(rm), Shift::from_bits(ty), self.r(rs) & 0xFF, self.c)
        } else {
            let (shift, amount) = decode_imm_shift(ty, bits(insn, 11, 7));
            shift_c(self.r(rm), shift, amount, self.c)
        };
        self.data_processing(insn, value, carry)
    }

    /// The sixteen data-processing operations on (Rn, shifter operand).
    fn data_processing(&mut self, insn: u32, op2: u32, shifter_carry: bool) -> Step {
        let opcode = bits(insn, 24, 21);
        let s = bit(insn, 20);
        let rn = bits(insn, 19, 16);
        let rd = bits(insn, 15, 12);
        // Should-be-zero fields: Rd of TST/TEQ/CMP/CMN, Rn of MOV/MVN.
        if ((0x8..=0xB).contains(&opcode) && rd != 0) || ((opcode == 0xD || opcode == 0xF) && rn != 0) {
            return Err(self.unpredictable());
        }
        if s && rd == 15 {
            // SUBS PC, LR and friends: an exception return, UNPREDICTABLE in user mode.
            return Err(self.trap(TrapKind::Privileged));
        }
        let a = self.r(rn);
        let (result, carry, overflow) = match opcode {
            0x0 | 0x8 => (a & op2, shifter_carry, self.v),
            0x1 | 0x9 => (a ^ op2, shifter_carry, self.v),
            0x2 | 0xA => add_with_carry(a, !op2, true),
            0x3 => add_with_carry(!a, op2, true),
            0x4 | 0xB => add_with_carry(a, op2, false),
            0x5 => add_with_carry(a, op2, self.c),
            0x6 => add_with_carry(a, !op2, self.c),
            0x7 => add_with_carry(!a, op2, self.c),
            0xC => (a | op2, shifter_carry, self.v),
            0xD => (op2, shifter_carry, self.v),
            0xE => (a & !op2, shifter_carry, self.v),
            _ => (!op2, shifter_carry, self.v),
        };
        if s {
            self.set_nz(result);
            self.c = carry;
            self.v = overflow;
        }
        if !(0x8..=0xB).contains(&opcode) {
            self.write_alu(rd, result);
        }
        Ok(())
    }

    /// Miscellaneous instructions (cccc 0001 0xx0 ... 0xxx). A should-be-one/zero field that is not is
    /// UNPREDICTABLE, and refused.
    fn a32_misc(&mut self, insn: u32) -> Step {
        let op = bits(insn, 22, 21);
        let rm = insn & 0xF;
        let rd = bits(insn, 15, 12);
        let fixed_ok = match (bits(insn, 6, 4), op) {
            (0b000, 0b00 | 0b10) => insn & 0x000F_0FFF == 0x000F_0000, // MRS
            (0b000, 0b01 | 0b11) => insn & 0x0000_FF00 == 0x0000_F000, // MSR (register)
            (0b001..=0b011, 0b01) => insn & 0x000F_FF00 == 0x000F_FF00, // BX, BXJ, BLX
            (0b001, 0b11) => insn & 0x000F_0F00 == 0x000F_0F00,          // CLZ
            (0b101, _) => insn & 0x0000_0F00 == 0,                       // QADD..QDSUB
            _ => true,
        };
        if !fixed_ok {
            return Err(self.unpredictable());
        }
        match (bits(insn, 6, 4), op) {
            (0b000, 0b00) | (0b000, 0b10) => {
                // MRS Rd, CPSR / SPSR
                if bit(insn, 22) {
                    return Err(self.trap(TrapKind::Privileged));
                }
                if rd == 15 {
                    return Err(self.unpredictable());
                }
                let value = self.cpsr() & !(1 << 5); // execution-state bits other than E read as zero
                self.set_r(rd, value);
                Ok(())
            }
            (0b000, 0b01) | (0b000, 0b11) => {
                if rm == 15 {
                    return Err(self.unpredictable());
                }
                self.a32_msr(bit(insn, 22), bits(insn, 19, 16), self.r(rm))
            }
            // BX, and BXJ (Jazelle disabled: BXJ behaves as BX).
            (0b001, 0b01) | (0b010, 0b01) => {
                let target = self.r(rm);
                self.bx_write_pc(target)
            }
            (0b001, 0b11) => {
                if rd == 15 || rm == 15 {
                    return Err(self.unpredictable());
                }
                self.set_r(rd, self.r(rm).leading_zeros());
                Ok(())
            }
            (0b011, 0b01) => {
                if rm == 15 {
                    return Err(self.unpredictable());
                }
                let target = self.r(rm);
                if target & 3 == 2 {
                    return Err(self.unpredictable());
                }
                self.set_r(14, self.cur.wrapping_add(4));
                self.bx_write_pc(target)
            }
            (0b101, _) => self.a32_saturating_add(insn),
            (0b111, 0b01) => {
                if insn >> 28 != 0xE {
                    return Err(self.unpredictable());
                }
                Err(Exit::Bkpt((bits(insn, 19, 8) << 4 | (insn & 0xF)) as u16))
            }
            (0b111, 0b11) => Err(self.trap(TrapKind::Privileged)), // SMC
            _ => Err(self.undefined()),
        }
    }

    /// QADD/QSUB/QDADD/QDSUB.
    fn a32_saturating_add(&mut self, insn: u32) -> Step {
        let (rn, rd, rm) = (bits(insn, 19, 16), bits(insn, 15, 12), insn & 0xF);
        if rn == 15 || rd == 15 || rm == 15 {
            return Err(self.unpredictable());
        }
        let m = self.r(rm) as i32 as i64;
        let mut n = self.r(rn) as i32 as i64;
        let op = bits(insn, 22, 21);
        let mut sat = false;
        if op & 2 != 0 {
            let (doubled, s) = crate::alu::signed_sat(2 * n, 32);
            n = doubled;
            sat |= s;
        }
        let (result, s) = crate::alu::signed_sat(if op & 1 != 0 { m - n } else { m + n }, 32);
        sat |= s;
        self.q |= sat;
        self.set_r(rd, result as u32);
        Ok(())
    }

    /// MSR from user mode: only the APSR's flags (mask bit 3) and GE (mask bit 2) are written; the
    /// control and extension fields are ignored. MSR SPSR is privileged.
    pub(crate) fn a32_msr(&mut self, spsr: bool, mask: u32, value: u32) -> Step {
        if spsr {
            return Err(self.trap(TrapKind::Privileged));
        }
        if mask == 0 {
            return Err(self.unpredictable());
        }
        self.set_apsr(value, mask & 8 != 0, mask & 4 != 0);
        Ok(())
    }

    /// The cond = 1111 space on ARMv6K.
    pub(crate) fn a32_unconditional(&mut self, insn: u32) -> Step {
        if insn & 0xFFF1_FE20 == 0xF100_0000 {
            return Ok(()); // CPS: no effect in user mode
        }
        if insn & 0xFFFF_FDFF == 0xF101_0000 {
            self.big_endian = bit(insn, 9); // SETEND
            return Ok(());
        }
        if insn == 0xF57F_F01F {
            self.clear_exclusive(); // CLREX
            return Ok(());
        }
        if insn & 0x0D70_F000 == 0x0550_F000 {
            return Ok(()); // PLD: a hint
        }
        if insn & 0x0E50_0000 == 0x0840_0000 || insn & 0x0E50_0000 == 0x0810_0000 {
            return Err(self.trap(TrapKind::Privileged)); // SRS, RFE
        }
        if bits(insn, 27, 25) == 0b101 {
            // BLX (immediate): to Thumb, H supplies bit 1.
            let offset = ((insn << 8) as i32 >> 6) as u32 | (bit(insn, 24) as u32) << 1;
            let target = self.r(15).wrapping_add(offset);
            self.set_r(14, self.cur.wrapping_add(4));
            self.thumb = true;
            self.regs[15] = target;
            return Ok(());
        }
        if bits(insn, 27, 25) == 0b110 || bits(insn, 27, 24) == 0b1110 {
            // LDC2/STC2/MCRR2/MRRC2/CDP2/MCR2/MRC2: no ARMv6 coprocessor here implements them.
            let cp = bits(insn, 11, 8) as u8;
            if matches!(cp, 10 | 11 | 15) {
                return Err(self.undefined());
            }
            return Err(self.trap(TrapKind::Coprocessor { cp }));
        }
        Err(self.undefined())
    }

    /// The hints in the MSR-immediate hole: NOP, YIELD, WFE, WFI, SEV (ARMv6K).
    pub(crate) fn a32_hint(&mut self, insn: u32) -> Step {
        match insn & 0xFF {
            2 | 3 => Err(Exit::Wait),
            _ => Ok(()),
        }
    }
}
