// Adapted from HexRaysSA/rax src/isa/arm/aarch32/instructions/memory.rs and decode.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! A32 loads and stores: word/byte, the extra halfword/signed/doubleword forms, LDM/STM in all four
//! modes, SWP/SWPB and the ARMv6K exclusives. Every form performs its memory accesses before it writes a
//! register, so a fault or trap leaves the register file untouched.

use crate::a32::{bit, bits};
use crate::access::Kind;
use crate::alu::{decode_imm_shift, shift_c};
use crate::cpu::{Cpu, Step};
use crate::{Exit, Memory, TrapKind};

impl Cpu {
    fn misaligned_trap(&self, addr: u32, write: bool) -> Exit {
        self.trap(TrapKind::Unaligned { addr, write })
    }

    /// (address used, written-back base) for the P/U/W addressing modes.
    fn addressing(&self, insn: u32, base: u32, offset: u32) -> (u32, u32) {
        let indexed = if bit(insn, 23) { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        (if bit(insn, 24) { indexed } else { base }, indexed)
    }

    /// LDR/STR/LDRB/STRB (and the T variants), immediate or scaled-register offset.
    pub(crate) fn a32_load_store_word<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        let (rn, rt) = (bits(insn, 19, 16), bits(insn, 15, 12));
        let (load, byte) = (bit(insn, 20), bit(insn, 22));
        let wback = !bit(insn, 24) || bit(insn, 21);
        let offset = if bit(insn, 25) {
            let rm = insn & 0xF;
            if rm == 15 {
                return Err(self.unpredictable());
            }
            let (shift, amount) = decode_imm_shift(bits(insn, 6, 5), bits(insn, 11, 7));
            shift_c(self.r(rm), shift, amount, self.c).0
        } else {
            insn & 0xFFF
        };
        if (wback && (rn == 15 || rn == rt)) || (byte && rt == 15) {
            return Err(self.unpredictable());
        }
        let (addr, new_base) = self.addressing(insn, self.r(rn), offset);
        if load {
            let value = if byte { self.ld8(mem, addr)? } else { self.ld32(mem, addr, Kind::Plain)? };
            if wback {
                self.set_r(rn, new_base);
            }
            if rt == 15 {
                if addr & 3 != 0 {
                    return Err(self.unpredictable());
                }
                return self.bx_write_pc(value);
            }
            self.set_r(rt, value);
        } else {
            let value = self.r(rt);
            if byte {
                self.st8(mem, addr, value)?;
            } else {
                self.st32(mem, addr, value, Kind::Plain)?;
            }
            if wback {
                self.set_r(rn, new_base);
            }
        }
        Ok(())
    }

    /// STRH/LDRH/LDRSB/LDRSH/LDRD/STRD (cccc 000P UIWL nnnn tttt iiii 1xx1 iiii).
    pub(crate) fn a32_extra_load_store<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        let (rn, rt) = (bits(insn, 19, 16), bits(insn, 15, 12));
        let load = bit(insn, 20);
        let op2 = bits(insn, 6, 5);
        let dual = !load && op2 >= 2;
        if !bit(insn, 24) && bit(insn, 21) {
            return Err(self.unpredictable()); // the ARMv6T2 unprivileged forms
        }
        let wback = !bit(insn, 24) || bit(insn, 21);
        let (offset, rm) = if bit(insn, 22) {
            (bits(insn, 11, 8) << 4 | (insn & 0xF), None)
        } else {
            let rm = insn & 0xF;
            if rm == 15 || bits(insn, 11, 8) != 0 {
                return Err(self.unpredictable());
            }
            (self.r(rm), Some(rm))
        };
        let (addr, new_base) = self.addressing(insn, self.r(rn), offset);
        if dual {
            // op2 = 10: LDRD, 11: STRD
            if rt & 1 != 0 || rt == 14 {
                return Err(self.unpredictable());
            }
            let rt2 = rt + 1;
            if wback && (rn == 15 || rn == rt || rn == rt2) {
                return Err(self.unpredictable());
            }
            let addr = self.check_multi(addr, op2 == 3)?;
            if op2 == 2 {
                if rm == Some(rt) || rm == Some(rt2) {
                    return Err(self.unpredictable());
                }
                let lo = self.ld32(mem, addr, Kind::Multi)?;
                let hi = self.ld32(mem, addr.wrapping_add(4), Kind::Multi)?;
                if wback {
                    self.set_r(rn, new_base);
                }
                self.set_r(rt, lo);
                self.set_r(rt2, hi);
            } else {
                let (lo, hi) = (self.r(rt), self.r(rt2));
                self.st32(mem, addr, lo, Kind::Multi)?;
                self.st32(mem, addr.wrapping_add(4), hi, Kind::Multi)?;
                if wback {
                    self.set_r(rn, new_base);
                }
            }
            return Ok(());
        }
        if rt == 15 || (wback && (rn == 15 || rn == rt)) {
            return Err(self.unpredictable());
        }
        if load {
            let value = match op2 {
                1 => self.ld16(mem, addr, Kind::Plain)?,
                2 => self.ld8(mem, addr)? as u8 as i8 as u32,
                _ => self.ld16(mem, addr, Kind::Plain)? as u16 as i16 as u32,
            };
            if wback {
                self.set_r(rn, new_base);
            }
            self.set_r(rt, value);
        } else {
            self.st16(mem, addr, self.r(rt), Kind::Plain)?;
            if wback {
                self.set_r(rn, new_base);
            }
        }
        Ok(())
    }

    /// LDM/STM (IA, IB, DA, DB), with or without writeback.
    pub(crate) fn a32_load_store_multiple<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        let rn = bits(insn, 19, 16);
        let list = insn & 0xFFFF;
        let (load, wback) = (bit(insn, 20), bit(insn, 21));
        if bit(insn, 22) {
            return Err(self.trap(TrapKind::Privileged)); // user-bank transfer / exception return
        }
        if rn == 15 || list == 0 {
            return Err(self.unpredictable());
        }
        let count = list.count_ones();
        let base = self.r(rn);
        let (start, new_base) = match (bit(insn, 24), bit(insn, 23)) {
            (false, true) => (base, base.wrapping_add(4 * count)),
            (true, true) => (base.wrapping_add(4), base.wrapping_add(4 * count)),
            (false, false) => (base.wrapping_sub(4 * count).wrapping_add(4), base.wrapping_sub(4 * count)),
            (true, false) => (base.wrapping_sub(4 * count), base.wrapping_sub(4 * count)),
        };
        self.transfer_multiple(mem, rn, list, start, load, wback.then_some(new_base))
    }

    /// The register transfers shared by A32 LDM/STM and Thumb LDMIA/STMIA/PUSH/POP. `new_base` is the
    /// written-back base, if any. A loaded PC interworks.
    pub(crate) fn transfer_multiple<M: Memory>(
        &mut self,
        mem: &mut M,
        rn: u32,
        list: u32,
        start: u32,
        load: bool,
        new_base: Option<u32>,
    ) -> Step {
        let start = self.check_multi(start, !load)?;
        let in_list = list & (1 << rn) != 0;
        if load {
            let mut values = [0u32; 16];
            let mut addr = start;
            for (r, value) in values.iter_mut().enumerate() {
                if list & (1 << r) != 0 {
                    *value = self.ld32(mem, addr, Kind::Multi)?;
                    addr = addr.wrapping_add(4);
                }
            }
            if list & (1 << 15) != 0 && values[15] & 3 == 2 {
                return Err(self.unpredictable());
            }
            if let Some(b) = new_base {
                self.set_r(rn, b); // a loaded base (below) wins, as on ARM11
            }
            for (r, &value) in values.iter().enumerate().take(15) {
                if list & (1 << r) != 0 {
                    self.regs[r] = value;
                }
            }
            if list & (1 << 15) != 0 {
                return self.bx_write_pc(values[15]);
            }
        } else {
            if new_base.is_some() && in_list && list & ((1 << rn) - 1) != 0 {
                return Err(self.unpredictable()); // base stored but not the lowest register
            }
            let mut addr = start;
            for r in 0..16 {
                if list & (1 << r) != 0 {
                    self.st32(mem, addr, self.r(r), Kind::Multi)?;
                    addr = addr.wrapping_add(4);
                }
            }
            if let Some(b) = new_base {
                self.set_r(rn, b);
            }
        }
        Ok(())
    }

    /// cccc 0001 xxxx .... 1001: SWP/SWPB and LDREX/STREX{,B,H,D}.
    pub(crate) fn a32_sync<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        let (rn, rt, rm) = (bits(insn, 19, 16), bits(insn, 15, 12), insn & 0xF);
        let op = bits(insn, 23, 20);
        if op & 0b1011 == 0 {
            // SWP (op 0000) / SWPB (op 0100): Rt = [Rn]; [Rn] = Rt2(rm)
            if rt == 15 || rm == 15 || rn == 15 || rn == rt || rn == rm || bits(insn, 11, 8) != 0 {
                return Err(self.unpredictable());
            }
            let addr = self.r(rn);
            let value = self.r(rm);
            let old = if op == 0 {
                let a = self.check_multi(addr, false)?;
                let old = self.ld32(mem, a, Kind::Multi)?.rotate_right(8 * (addr & 3));
                self.st32(mem, a, value, Kind::Multi)?;
                old
            } else {
                let old = self.ld8(mem, addr)?;
                self.st8(mem, addr, value)?;
                old
            };
            self.set_r(rt, old);
            return Ok(());
        }
        if op & 0b1000 == 0 || bits(insn, 11, 8) != 0xF {
            return Err(self.undefined());
        }
        if op & 1 != 0 && insn & 0xF != 0xF {
            return Err(self.unpredictable()); // LDREX's should-be-one Rt2 field
        }
        let size: u8 = match op >> 1 & 3 {
            0b00 => 4,
            0b01 => 8,
            0b10 => 1,
            _ => 2,
        };
        let addr = self.r(rn);
        if rn == 15 {
            return Err(self.unpredictable());
        }
        if addr & (size as u32 - 1) != 0 {
            return Err(self.misaligned_trap(addr, op & 1 == 0));
        }
        if op & 1 != 0 {
            // LDREX*: Rt = [Rn]
            if rt == 15 || (size == 8 && (rt & 1 != 0 || rt == 14)) {
                return Err(self.unpredictable());
            }
            let (lo, hi) = match size {
                1 => (self.ld8(mem, addr)?, 0),
                2 => (self.ld16(mem, addr, Kind::Exclusive)?, 0),
                4 => (self.ld32(mem, addr, Kind::Exclusive)?, 0),
                _ => (self.ld32(mem, addr, Kind::Exclusive)?, self.ld32(mem, addr.wrapping_add(4), Kind::Exclusive)?),
            };
            self.set_r(rt, lo);
            if size == 8 {
                self.set_r(rt + 1, hi);
            }
            self.mark_exclusive(addr, size);
        } else {
            // STREX*: Rd(rt) = 0 on success, 1 on failure; stores Rt(rm)
            let (rd, rt) = (rt, rm);
            let rt2 = rt + 1;
            if rd == 15
                || rt == 15
                || rd == rn
                || rd == rt
                || (size == 8 && (rt & 1 != 0 || rt == 14 || rd == rt2))
            {
                return Err(self.unpredictable());
            }
            if self.exclusive_holds(addr, size) {
                match size {
                    1 => self.st8(mem, addr, self.r(rt))?,
                    2 => self.st16(mem, addr, self.r(rt), Kind::Exclusive)?,
                    4 => self.st32(mem, addr, self.r(rt), Kind::Exclusive)?,
                    _ => {
                        self.st32(mem, addr, self.r(rt), Kind::Exclusive)?;
                        self.st32(mem, addr.wrapping_add(4), self.r(rt2), Kind::Exclusive)?;
                    }
                }
                self.set_r(rd, 0);
            } else {
                self.set_r(rd, 1);
            }
            self.clear_exclusive();
        }
        Ok(())
    }
}
