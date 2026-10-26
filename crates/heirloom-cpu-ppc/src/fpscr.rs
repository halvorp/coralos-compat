//! The floating-point status and control register (PEM 2.1.4). Bit numbers are the manual's (0 = MSB).
use crate::cpu::Cpu;

const fn b(n: u32) -> u32 { 1 << (31 - n) }
pub const FX: u32 = b(0);
pub const FEX: u32 = b(1);
pub const VX: u32 = b(2);
pub const OX: u32 = b(3);
pub const UX: u32 = b(4);
pub const ZX: u32 = b(5);
pub const XX: u32 = b(6);
pub const VXSNAN: u32 = b(7);
pub const VXISI: u32 = b(8);
pub const VXIDI: u32 = b(9);
pub const VXZDZ: u32 = b(10);
pub const VXIMZ: u32 = b(11);
pub const VXVC: u32 = b(12);
pub const FI: u32 = b(14);
pub const VXSOFT: u32 = b(21);
pub const VXSQRT: u32 = b(22);
pub const VXCVI: u32 = b(23);
pub const VE: u32 = b(24);
pub const OE: u32 = b(25);
pub const UE: u32 = b(26);
pub const ZE: u32 = b(27);
pub const XE: u32 = b(28);
pub const RN: u32 = 3;
/// FPRF = FPSCR[15:19] (C, FL, FG, FE, FU).
pub const FPRF_SHIFT: u32 = 12;
pub const FPRF_MASK: u32 = 0x1F << FPRF_SHIFT;
/// The sticky exception bits (cleared by mcrfs/mtfsb0, set only by hardware events or mtfsb1).
pub const STICKY: u32 = FX | OX | UX | ZX | XX | VXSNAN | VXISI | VXIDI | VXZDZ | VXIMZ | VXVC | VXSOFT | VXSQRT | VXCVI;
const VX_ALL: u32 = VXSNAN | VXISI | VXIDI | VXZDZ | VXIMZ | VXVC | VXSOFT | VXSQRT | VXCVI;

/// Recompute the derived bits: VX = OR of the invalid-operation flags; FEX = any exception whose enable is set.
pub fn fix(cpu: &mut Cpu) {
    let mut f = cpu.fpscr & !(VX | FEX);
    if f & VX_ALL != 0 { f |= VX; }
    if (f & VX != 0 && f & VE != 0) || (f & OX != 0 && f & OE != 0) || (f & UX != 0 && f & UE != 0)
        || (f & ZX != 0 && f & ZE != 0) || (f & XX != 0 && f & XE != 0) { f |= FEX; }
    cpu.fpscr = f;
}

/// Raise exception flag(s): FX is set when any sticky flag goes 0 -> 1.
pub fn raise(cpu: &mut Cpu, flags: u32) {
    if flags & !cpu.fpscr != 0 { cpu.fpscr |= FX; }
    cpu.fpscr |= flags;
    fix(cpu);
}

pub fn set_fprf(cpu: &mut Cpu, fprf: u32) { cpu.fpscr = (cpu.fpscr & !FPRF_MASK) | fprf << FPRF_SHIFT; }

/// FPRF code of a result (PEM Table 3-? "Floating-point result flags"): C FL FG FE FU.
/// `single`: classify denormal against the single-precision range (for the fXXXs instructions).
pub fn classify(x: f64, single: bool) -> u32 {
    let neg = x.is_sign_negative();
    if x.is_nan() { 0b10001 }
    else if x.is_infinite() { if neg { 0b01001 } else { 0b00101 } }
    else if x == 0.0 { if neg { 0b10010 } else { 0b00010 } }
    else {
        let min = if single { f32::MIN_POSITIVE as f64 } else { f64::MIN_POSITIVE };
        let denorm = x.abs() < min;
        match (neg, denorm) { (true, true) => 0b11000, (false, true) => 0b10100, (true, false) => 0b01000, (false, false) => 0b00100 }
    }
}

/// Rc=1 on an FP instruction: CR1 = FPSCR[FX FEX VX OX].
pub fn copy_to_cr1(cpu: &mut Cpu) { let v = cpu.fpscr >> 28; cpu.set_cr_field(1, v); }
