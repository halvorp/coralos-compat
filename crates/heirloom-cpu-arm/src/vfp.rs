// Register-file layout and FPSCR field map adapted from HexRaysSA/rax src/isa/arm/aarch32/vfp.rs and the VFP
// decode in src/isa/arm/decoder/aarch32/vfp.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! VFPv2 data processing: VMLA/VMLS/VNMLA/VNMLS, VMUL/VNMUL, VADD/VSUB, VDIV, VMOV/VABS/VNEG/VSQRT,
//! VCMP{E} (also with zero) and the VCVT forms VFPv2 has (single <-> double, integer <-> float, VCVTR),
//! including FPSCR.LEN/STRIDE short vectors.

use crate::a32::{bit, bits};
use crate::cpu::{Cpu, Step};
use crate::softfloat::{Env, Fmt, F32, F64};
use crate::TrapKind;

/// FPSID as VFP11 (ARM1176JZF-S) reports it.
pub const FPSID: u32 = 0x4101_20B5;

/// FPSCR bits a VFPv2 implementation holds: NZCV, DN, FZ, RMode, Stride, Len, the trap enables and the
/// cumulative flags.
pub(crate) const FPSCR_MASK: u32 = 0xF3F7_9F9F;

/// The VFPv2 register file: S0-S31, aliased as D0-D15 (D[i] = S[2i+1]:S[2i]), and FPSCR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vfp {
    pub s: [u32; 32],
    pub fpscr: u32,
}

impl Vfp {
    pub fn d(&self, i: usize) -> u64 {
        self.s[2 * i] as u64 | (self.s[2 * i + 1] as u64) << 32
    }

    pub fn set_d(&mut self, i: usize, v: u64) {
        self.s[2 * i] = v as u32;
        self.s[2 * i + 1] = (v >> 32) as u32;
    }

    pub(crate) fn get(&self, double: bool, r: u32) -> u64 {
        if double {
            self.d(r as usize)
        } else {
            self.s[r as usize] as u64
        }
    }

    pub(crate) fn put(&mut self, double: bool, r: u32, v: u64) {
        if double {
            self.set_d(r as usize, v)
        } else {
            self.s[r as usize] = v as u32
        }
    }
}

/// A VFP register field: single = Vx:b, double = b:Vx (VFPv2 has D0-D15 only, so b must be clear).
pub(crate) fn vreg(double: bool, v: u32, b: bool) -> Option<u32> {
    if double {
        (!b).then_some(v)
    } else {
        Some(v << 1 | b as u32)
    }
}

#[derive(Clone, Copy)]
enum Op {
    Mla,
    Mls,
    Nmls,
    Nmla,
    Mul,
    Nmul,
    Add,
    Sub,
    Div,
    Mov,
    Abs,
    Neg,
    Sqrt,
}

impl Op {
    fn monadic(self) -> bool {
        matches!(self, Op::Mov | Op::Abs | Op::Neg | Op::Sqrt)
    }

    fn apply(self, env: &mut Env, fmt: Fmt, d: u64, n: u64, m: u64, sign: u64) -> u64 {
        match self {
            Op::Mla => {
                let p = env.mul(fmt, n, m);
                env.add(fmt, d, p)
            }
            Op::Mls => {
                let p = env.mul(fmt, n, m);
                env.add(fmt, d, p ^ sign)
            }
            Op::Nmls => {
                let p = env.mul(fmt, n, m);
                env.add(fmt, d ^ sign, p)
            }
            Op::Nmla => {
                let p = env.mul(fmt, n, m);
                env.add(fmt, d ^ sign, p ^ sign)
            }
            Op::Mul => env.mul(fmt, n, m),
            Op::Nmul => env.mul(fmt, n, m) ^ sign,
            Op::Add => env.add(fmt, n, m),
            Op::Sub => env.sub(fmt, n, m),
            Op::Div => env.div(fmt, n, m),
            Op::Mov => m,
            Op::Abs => m & !sign,
            Op::Neg => m ^ sign,
            Op::Sqrt => env.sqrt(fmt, m),
        }
    }
}

impl Cpu {
    /// Fold an operation's flags into FPSCR, or trap if one of them is enabled.
    pub(crate) fn vfp_commit_flags(&mut self, flags: u32) -> Step {
        let fpscr = self.vfp.fpscr;
        let enabled = (fpscr >> 8) & 0x1F | ((fpscr >> 15) & 1) << 7;
        if flags & enabled != 0 {
            return Err(self.trap(TrapKind::VfpException { flags: (flags & 0x9F) as u8 }));
        }
        self.vfp.fpscr |= flags;
        Ok(())
    }

    /// CDP on cp10 (single) / cp11 (double).
    pub(crate) fn vfp_data_processing(&mut self, insn: u32) -> Step {
        let double = bit(insn, 8);
        let opc = (bit(insn, 23) as u32) << 2 | bits(insn, 21, 20);
        let op6 = bit(insn, 6);
        let op = match (opc, op6) {
            (0b000, false) => Op::Mla,
            (0b000, true) => Op::Mls,
            (0b001, false) => Op::Nmls,
            (0b001, true) => Op::Nmla,
            (0b010, false) => Op::Mul,
            (0b010, true) => Op::Nmul,
            (0b011, false) => Op::Add,
            (0b011, true) => Op::Sub,
            (0b100, false) => Op::Div,
            (0b111, true) => match (bits(insn, 19, 16), bit(insn, 7)) {
                (0b0000, false) => Op::Mov,
                (0b0000, true) => Op::Abs,
                (0b0001, false) => Op::Neg,
                (0b0001, true) => Op::Sqrt,
                _ => return self.vfp_compare_convert(insn),
            },
            _ => return Err(self.undefined()), // VFMA/VFNMA (VFPv4), VMOV immediate (VFPv3)
        };
        let (Some(d), Some(n), Some(m)) = (
            vreg(double, bits(insn, 15, 12), bit(insn, 22)),
            vreg(double, bits(insn, 19, 16), bit(insn, 7) && !op.monadic()),
            vreg(double, insn & 0xF, bit(insn, 5)),
        ) else {
            return Err(self.undefined());
        };
        let fmt = if double { F64 } else { F32 };
        let sign = if double { 1 << 63 } else { 1 << 31 };
        let (len, stride) = self.vector_shape(double)?;
        let bank = if double { 4 } else { 8 };
        let step = |base: u32, i: u32| (base & !(bank - 1)) | ((base + i * stride) & (bank - 1));
        let vector = len > 1 && d >= bank;
        let m_scalar = !vector || m < bank;
        let count = if vector { len } else { 1 };
        // Elements execute in order (a later element sees an earlier one's result, as on VFP11), on a
        // copy of the register file so that an enabled exception can still trap with nothing changed.
        let mut env = Env::new(self.vfp.fpscr);
        let mut regs = self.vfp.clone();
        for i in 0..count {
            let (di, ni) = (step(d, i), step(n, i));
            let mi = if m_scalar { m } else { step(m, i) };
            let (dv, nv, mv) = (regs.get(double, di), regs.get(double, ni), regs.get(double, mi));
            regs.put(double, di, op.apply(&mut env, fmt, dv, nv, mv, sign));
        }
        self.vfp_commit_flags(env.flags)?;
        self.vfp.s = regs.s;
        Ok(())
    }

    /// FPSCR.LEN/STRIDE as (length, stride); combinations that leave the register bank are UNPREDICTABLE.
    fn vector_shape(&self, double: bool) -> Result<(u32, u32), crate::Exit> {
        let len = ((self.vfp.fpscr >> 16) & 7) + 1;
        let stride = match (self.vfp.fpscr >> 20) & 3 {
            0 => 1,
            3 => 2,
            _ if len == 1 => 1,
            _ => return Err(self.unpredictable()),
        };
        let bank = if double { 4 } else { 8 };
        if len > 1 && len * stride > bank {
            return Err(self.unpredictable());
        }
        Ok((len, stride))
    }

    /// The scalar-only extension operations: VCMP{E}, VCVT.
    fn vfp_compare_convert(&mut self, insn: u32) -> Step {
        let double = bit(insn, 8);
        let fmt = if double { F64 } else { F32 };
        let (vd, vm) = (bits(insn, 15, 12), insn & 0xF);
        let (dbit, mbit) = (bit(insn, 22), bit(insn, 5));
        let n7 = bit(insn, 7);
        let mut env = Env::new(self.vfp.fpscr);
        let reg = |dbl: bool, v: u32, b: bool| vreg(dbl, v, b);
        match bits(insn, 19, 16) {
            0b0100 | 0b0101 => {
                // VCMP{E} Vd, Vm / Vd, #0.0
                let (Some(d), Some(m)) = (reg(double, vd, dbit), reg(double, vm, mbit)) else {
                    return Err(self.undefined());
                };
                let a = self.vfp.get(double, d);
                let b = if bit(insn, 16) { 0 } else { self.vfp.get(double, m) };
                let nzcv = env.compare(fmt, a, b, n7);
                self.vfp_commit_flags(env.flags)?;
                self.vfp.fpscr = (self.vfp.fpscr & 0x0FFF_FFFF) | nzcv;
                Ok(())
            }
            0b0111 if n7 => {
                // VCVT.F64.F32 Dd, Sm (cp10) / VCVT.F32.F64 Sd, Dm (cp11)
                let (Some(d), Some(m)) = (reg(!double, vd, dbit), reg(double, vm, mbit)) else {
                    return Err(self.undefined());
                };
                let (from, to) = if double { (F64, F32) } else { (F32, F64) };
                let r = env.convert(from, to, self.vfp.get(double, m));
                self.vfp_commit_flags(env.flags)?;
                self.vfp.put(!double, d, r);
                Ok(())
            }
            0b1000 => {
                // VCVT.F32/F64.{U32,S32} Vd, Sm (n7 = signed)
                let (Some(d), Some(m)) = (reg(double, vd, dbit), reg(false, vm, mbit)) else {
                    return Err(self.undefined());
                };
                let r = env.int_to_float(fmt, self.vfp.s[m as usize], n7);
                self.vfp_commit_flags(env.flags)?;
                self.vfp.put(double, d, r);
                Ok(())
            }
            0b1100 | 0b1101 => {
                // VCVT{R}.{U32,S32}.F32/F64 Sd, Vm (opc2 bit 0 = signed, n7 = round towards zero)
                let (Some(d), Some(m)) = (reg(false, vd, dbit), reg(double, vm, mbit)) else {
                    return Err(self.undefined());
                };
                let r = env.float_to_int(fmt, self.vfp.get(double, m), bit(insn, 16), n7);
                self.vfp_commit_flags(env.flags)?;
                self.vfp.s[d as usize] = r;
                Ok(())
            }
            _ => Err(self.undefined()), // half-precision and fixed-point VCVT are VFPv3
        }
    }
}
