// Adapted from HexRaysSA/rax src/isa/arm/aarch32/instructions/fp_access.rs and the VFP transfer decode
// in src/isa/arm/decoder/aarch32/vfp.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! VFPv2 transfers: VMOV between core and VFP registers (single, scalar, pairs), VMRS/VMSR, and the
//! loads and stores VLDR/VSTR, VLDM/VSTM (VPUSH/VPOP) including the FLDMX/FSTMX forms.

use crate::a32::{bit, bits};
use crate::access::Kind;
use crate::cpu::{Cpu, Step};
use crate::vfp::{vreg, FPSCR_MASK, FPSID};
use crate::{Memory, TrapKind};

impl Cpu {
    /// MCR/MRC on cp10/cp11: VMOV Sn <-> Rt, VMRS/VMSR, VMOV Dn[x] <-> Rt.
    pub(crate) fn vfp_move_core(&mut self, insn: u32) -> Step {
        let load = bit(insn, 20); // to the core register
        let rt = bits(insn, 15, 12);
        let vn = bits(insn, 19, 16);
        if bits(insn, 3, 0) != 0 {
            return Err(self.undefined());
        }
        let double = bit(insn, 8);
        match (double, bits(insn, 23, 21)) {
            (false, 0b000) if bits(insn, 6, 5) == 0 => {
                let n = (vn << 1) | bit(insn, 7) as u32;
                if rt == 15 {
                    return Err(self.unpredictable());
                }
                if load {
                    self.set_r(rt, self.vfp.s[n as usize]);
                } else {
                    self.vfp.s[n as usize] = self.r(rt);
                }
                Ok(())
            }
            (false, 0b111) if bits(insn, 7, 5) == 0 => self.vfp_system_register(load, vn, rt),
            (true, 0b000 | 0b001) if bits(insn, 6, 5) == 0 && !bit(insn, 7) => {
                // VMOV.32 Dn[x], Rt / Rt, Dn[x]
                if rt == 15 {
                    return Err(self.unpredictable());
                }
                let s = (2 * vn + bit(insn, 21) as u32) as usize;
                if load {
                    self.set_r(rt, self.vfp.s[s]);
                } else {
                    self.vfp.s[s] = self.r(rt);
                }
                Ok(())
            }
            _ => Err(self.undefined()), // the byte/halfword scalars and VDUP are NEON
        }
    }

    /// VMRS Rt, <reg> / VMSR <reg>, Rt. User mode reaches FPSID (read) and FPSCR.
    fn vfp_system_register(&mut self, load: bool, reg: u32, rt: u32) -> Step {
        match (reg, load) {
            (0b0000, true) if rt != 15 => {
                self.set_r(rt, FPSID);
                Ok(())
            }
            (0b0001, true) => {
                if rt == 15 {
                    // VMRS APSR_nzcv, FPSCR
                    let f = self.vfp.fpscr;
                    self.n = f & (1 << 31) != 0;
                    self.z = f & (1 << 30) != 0;
                    self.c = f & (1 << 29) != 0;
                    self.v = f & (1 << 28) != 0;
                } else {
                    self.set_r(rt, self.vfp.fpscr);
                }
                Ok(())
            }
            (0b0001, false) if rt != 15 => {
                self.vfp.fpscr = self.r(rt) & FPSCR_MASK;
                Ok(())
            }
            _ => Err(self.trap(TrapKind::VfpSystemRegister { reg: reg as u8 })),
        }
    }

    /// MCRR/MRRC on cp10/cp11: VMOV Sm, Sm+1 <-> Rt, Rt2 and VMOV Dm <-> Rt, Rt2.
    pub(crate) fn vfp_move_two(&mut self, insn: u32) -> Step {
        let load = bit(insn, 20);
        let (rt2, rt) = (bits(insn, 19, 16), bits(insn, 15, 12));
        let double = bit(insn, 8);
        if bits(insn, 7, 6) != 0 || !bit(insn, 4) {
            return Err(self.undefined());
        }
        let Some(m) = vreg(double, insn & 0xF, bit(insn, 5)) else {
            return Err(self.undefined());
        };
        if rt == 15 || rt2 == 15 || (load && rt == rt2) || (!double && m == 31) {
            return Err(self.unpredictable());
        }
        let first = if double { 2 * m } else { m } as usize;
        if load {
            self.set_r(rt, self.vfp.s[first]);
            self.set_r(rt2, self.vfp.s[first + 1]);
        } else {
            self.vfp.s[first] = self.r(rt);
            self.vfp.s[first + 1] = self.r(rt2);
        }
        Ok(())
    }

    /// LDC/STC on cp10/cp11: VLDR/VSTR and VLDM/VSTM (IA, IA!, DB!).
    pub(crate) fn vfp_load_store<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        let (p, u, w, load) = (bit(insn, 24), bit(insn, 23), bit(insn, 21), bit(insn, 20));
        let rn = bits(insn, 19, 16);
        let double = bit(insn, 8);
        let imm8 = insn & 0xFF;
        let Some(first) = vreg(double, bits(insn, 15, 12), bit(insn, 22)) else {
            return Err(self.undefined());
        };
        // Work in single-register units: a double is two consecutive S registers.
        let (start_s, words, span) = if p && !w {
            // VLDR/VSTR: one register at Align(Rn, 4) +/- imm8*4
            let n = if double { 2 } else { 1 };
            let base = self.r(rn) & if rn == 15 { !3 } else { !0 };
            let addr = if u { base.wrapping_add(imm8 * 4) } else { base.wrapping_sub(imm8 * 4) };
            return self.vfp_transfer(mem, load, if double { 2 * first } else { first }, n, addr, None, double);
        } else {
            match (p, u, w) {
                (false, true, _) | (true, false, true) => {}
                _ => return Err(self.undefined()),
            }
            let regs = if double { imm8 / 2 } else { imm8 };
            let limit = if double { 16 } else { 32 };
            if regs == 0 || first + regs > limit || (w && rn == 15) {
                return Err(self.unpredictable());
            }
            (if double { 2 * first } else { first }, if double { regs * 2 } else { regs }, imm8 * 4)
        };
        let base = self.r(rn);
        let (addr, new_base) =
            if u { (base, base.wrapping_add(span)) } else { (base.wrapping_sub(span), base.wrapping_sub(span)) };
        self.vfp_transfer(mem, load, start_s, words, addr, w.then_some((rn, new_base)), double)
    }

    /// Move `words` consecutive S registers from `start` to/from memory at `addr` (word-aligned), then
    /// write the base back. Doubles keep their low word at the lower address unless CPSR.E is set.
    #[allow(clippy::too_many_arguments)]
    fn vfp_transfer<M: Memory>(
        &mut self,
        mem: &mut M,
        load: bool,
        start: u32,
        words: u32,
        addr: u32,
        wback: Option<(u32, u32)>,
        double: bool,
    ) -> Step {
        let addr = self.check_multi(addr, !load)?;
        // In a big-endian double the most significant word comes first.
        let slot = |i: u32| if double && self.big_endian { i ^ 1 } else { i };
        if load {
            let mut values = [0u32; 32];
            for i in 0..words {
                values[slot(i) as usize] = self.ld32(mem, addr.wrapping_add(4 * i), Kind::Multi)?;
            }
            self.vfp.s[start as usize..(start + words) as usize].copy_from_slice(&values[..words as usize]);
        } else {
            for i in 0..words {
                let v = self.vfp.s[(start + slot(i)) as usize];
                self.st32(mem, addr.wrapping_add(4 * i), v, Kind::Multi)?;
            }
        }
        if let Some((rn, b)) = wback {
            self.set_r(rn, b);
        }
        Ok(())
    }
}
