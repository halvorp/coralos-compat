//! The OS-trap REGISTER calling convention, implemented once (the Pascal one is [`crate::pascal`]).
//!
//! Operating System traps (`$A000-$A7FF`, bit 11 clear) take their inputs in registers, classically D0 and A0 (and
//! A1, D1 for a few), and return a result code in D0 (and often a pointer or handle in A0). The dispatcher then
//! tests D0 so the program can `bne` straight after the trap; [`set_flags`] does that (a `tst.w d0`: N and Z from
//! the low word, V and C cleared, X kept). Which registers a particular call uses is that call's own business; the
//! handler sees them in [`Regs`] and writes its results back into the same struct.
use heirloom_cpu_68k::Cpu;

pub const NO_ERR: i16 = 0;
/// paramErr: a handler refused the call (the refusal text is in `Mac::refused`).
pub const PARAM_ERR: i16 = -50;

/// The registers of an OS trap: `d0`/`a0` are in and out, `d1`/`a1` are inputs only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regs {
    /// The trap word, flag bits included (`NewHandle` vs `NewHandleClear` differ only in bit 9).
    pub opcode: u16,
    pub d0: u32,
    pub a0: u32,
    pub d1: u32,
    pub a1: u32,
}

impl Regs {
    pub fn read(cpu: &Cpu, opcode: u16) -> Regs { Regs { opcode, d0: cpu.dreg(0), a0: cpu.areg(0), d1: cpu.dreg(1), a1: cpu.areg(1) } }
    /// Return an OSErr in D0.
    pub fn err(&mut self, e: i16) { self.d0 = e as i32 as u32; }
    /// The clear flag, bit 9, which turns NewPtr/NewHandle into NewPtrClear/NewHandleClear.
    pub fn clear_flag(&self) -> bool { self.opcode & 0x0200 != 0 }
}

/// `tst.w d0`: N from bit 15, Z from the low word, V and C cleared, X untouched.
pub fn set_flags(cpu: &mut Cpu, d0: u32) {
    let w = d0 as u16;
    let ccr = (if w & 0x8000 != 0 { 8 } else { 0 }) | (if w == 0 { 4 } else { 0 });
    cpu.set_sr((cpu.sr() & !0xF) | ccr);
}
