//! Instruction word field extraction (PowerPC manual numbering: bit 0 is the MSB, so field "6-10" is `>> 21 & 31`).

#[derive(Clone, Copy)]
pub struct W(pub u32);

impl W {
    pub fn op(self) -> u32 { self.0 >> 26 }
    /// RT / RS / FRT / FRS / BO / BT (bits 6-10).
    pub fn rt(self) -> usize { (self.0 >> 21 & 31) as usize }
    pub fn ra(self) -> usize { (self.0 >> 16 & 31) as usize }
    pub fn rb(self) -> usize { (self.0 >> 11 & 31) as usize }
    /// FRC (bits 21-25).
    pub fn frc(self) -> usize { (self.0 >> 6 & 31) as usize }
    pub fn rc(self) -> bool { self.0 & 1 != 0 }
    pub fn oe(self) -> bool { self.0 >> 10 & 1 != 0 }
    /// Extended opcode of X/XO/XL forms (bits 21-30).
    pub fn xo(self) -> u32 { self.0 >> 1 & 0x3FF }
    /// Extended opcode of the A-form FP instructions (bits 26-30).
    pub fn xo5(self) -> u32 { self.0 >> 1 & 0x1F }
    pub fn sh(self) -> u32 { self.0 >> 11 & 31 }
    pub fn mb(self) -> u32 { self.0 >> 6 & 31 }
    pub fn me(self) -> u32 { self.0 >> 1 & 31 }
    pub fn simm(self) -> u32 { self.0 as u16 as i16 as i32 as u32 }
    pub fn uimm(self) -> u32 { self.0 & 0xFFFF }
    /// CR field selector BF / BFA-style (bits 6-8).
    pub fn bf(self) -> u32 { self.0 >> 23 & 7 }
    pub fn bfa(self) -> u32 { self.0 >> 18 & 7 }
    pub fn lk(self) -> bool { self.0 & 1 != 0 }
    pub fn aa(self) -> bool { self.0 & 2 != 0 }
    /// The 10-bit SPR/TBR number: its two 5-bit halves are swapped in the encoding (PEM mfspr).
    pub fn spr(self) -> u32 { (self.0 >> 16 & 31) | (self.0 >> 11 & 31) << 5 }
}

/// MASK(mb, me) of rlwinm & co: ones from bit `mb` through `me` (manual numbering), wrapping when mb > me.
pub fn mask(mb: u32, me: u32) -> u32 {
    let a = 0xFFFF_FFFFu32 >> mb;
    let b = 0xFFFF_FFFFu32 << (31 - me);
    if mb <= me { a & b } else { a | b }
}
