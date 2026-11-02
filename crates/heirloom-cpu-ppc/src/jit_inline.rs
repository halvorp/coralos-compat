//! Tier 2: inline code for the forms tier 1 has no handler of its own for, which WC3 gameplay hands to the slow stub 8.4% of
//! its instructions without this (WC3_JIT_CENSUS 2026-11-02: lfs ~3%, fcmpu/fcmpo, single FP arithmetic, stfs, sraw, lwbrx,
//! sthx, lhzx, extsh.). Each form restates tier 0 exactly ON ITS COMMON PATH and leaves anything else to the slow stub BEFORE
//! writing any state, so the slow stub re-runs the whole instruction on tier 1's handler:
//! - lfs: cvtss2sd is exact for every non-NaN single (MXCSR as Rust keeps it: no DAZ/FTZ); a NaN (whose quiet bit tier 0
//!   keeps) goes slow.
//! - stfs: the PEM truncation's first branch (exponent > 896, or +-0) is pure bit selection; a single denormal goes slow.
//! - fcmpu/fcmpo: ordered operands (ucomisd PF clear); a NaN (VXSNAN / VXVC) goes slow.
//! - fadd/fsub/fmul and the single forms, Rc=0: under FPSCR[RN] = 0 only, the f64 operation then, for the single forms,
//!   `(r as f32) as f64` - exactly tier 0's arithmetic - and only when the result is finite (a NaN result means a NaN or an
//!   invalid operand, an infinite one means OX or an infinite operand: all slow). FPRF from the result's class.
//! - sraw (and Rc), extsh. / extsb., and the X-form loads/stores tier 1 leaves Generic (RA = 0, halfword, byte-reversed).
use crate::cpu::Cpu;
use crate::fields::W;
use crate::jit_asm::*;
use crate::jit_emit::{cpu, fpr, g, Ea, E, CR, MUT_FP_SINGLE, MUT_SRAW_BIG};
use crate::t1op::{Op, K};
use core::mem::offset_of;

const FPSCR: usize = offset_of!(Cpu, fpscr);
fn f(n: usize) -> M { m(RBX, fpr(n as u8)) }

impl E<'_, '_> {
    /// Inline code for instruction `i` if this form has it; false = the caller uses the slow stub.
    pub(crate) fn inline(&mut self, i: usize, op: &Op) -> bool {
        let w = W(op.w);
        match op.k {
            K::Lfs => { self.lfs(i, Ea { ra: op.a, rb: None, imm: op.imm, update: false }, op.d as usize); true }
            K::Stfs => { self.stfs(i, Ea { ra: op.a, rb: None, imm: op.imm, update: false }, op.d as usize); true }
            K::Fp59 | K::Fp63 => self.fp(i, w, op.k == K::Fp59),
            K::Generic => self.x31(i, w),
            _ => false,
        }
    }

    fn lfs(&mut self, i: usize, e: Ea, d: usize) {
        self.ea(i, e, 4);
        let a = &mut self.a;
        a.ld32(RDX, mx(R12, RAX, 1));
        a.bswap32(RDX);
        a.mov_rr(RCX, RDX);
        a.alu_ri(Alu::And, RCX, 0x7FFF_FFFF);
        a.alu_ri(Alu::Cmp, RCX, 0x7F80_0000);
        self.slow_if(Cc::A, i);
        let a = &mut self.a;
        a.movd_xr(0, RDX);
        a.sse_r(0xF3, 0x5A, 0, 0); // cvtss2sd
        a.sse_m(0xF2, 0x11, 0, f(d)); // movsd [frD], xmm0
    }

    fn stfs(&mut self, i: usize, e: Ea, s: usize) {
        // the value first: a single denormal (exponent <= 896 and not +-0) is the slow path's
        let a = &mut self.a;
        a.ld64(RDX, f(s));
        a.mov_rr64(RCX, RDX);
        a.sh_ri64(Sh::Shr, RCX, 52);
        a.alu_ri(Alu::And, RCX, 0x7FF);
        a.alu_ri(Alu::Cmp, RCX, 896);
        let ok = a.jcc(Cc::A);
        a.mov_rr64(RCX, RDX);
        a.alu_rr64(Alu::Add, RCX, RCX);
        self.slow_if(Cc::Ne, i);
        self.a.here(ok);
        self.ea(i, e, 4);
        self.store_guard(i, e, 4);
        let a = &mut self.a;
        a.ld64(RDX, f(s));
        a.mov_rr64(RCX, RDX);
        a.sh_ri64(Sh::Shr, RCX, 32);
        a.alu_ri(Alu::And, RCX, 0xC000_0000);
        a.sh_ri64(Sh::Shr, RDX, 29);
        a.alu_ri(Alu::And, RDX, 0x3FFF_FFFF);
        a.alu_rr(Alu::Or, RDX, RCX);
        a.bswap32(RDX);
        a.st32(mx(R12, RAX, 1), RDX);
    }

    fn fp(&mut self, i: usize, w: W, single: bool) -> bool {
        let (d, a_, b, c) = (w.rt(), w.ra(), w.rb(), w.frc());
        if !single {
            match w.xo() {
                0 | 32 => { self.fcmp(i, w.bf() as u8, a_, b); return true; }
                40 | 72 | 136 | 264 if !w.rc() => {
                    let a = &mut self.a;
                    a.ld64(RDX, f(b));
                    match w.xo() {
                        40 => { a.mov_ri64(RCX, 1 << 63); a.alu_rr64(Alu::Xor, RDX, RCX); }
                        136 => { a.mov_ri64(RCX, 1 << 63); a.alu_rr64(Alu::Or, RDX, RCX); }
                        264 => { a.sh_ri64(Sh::Shl, RDX, 1); a.sh_ri64(Sh::Shr, RDX, 1); }
                        _ => {}
                    }
                    a.st64(f(d), RDX);
                    return true;
                }
                // every other opcode-63 form with an xo10 of its own (frsp, fctiw, mffs, mtfsf, ...): tier 1's handler
                12 | 14 | 15 | 40 | 72 | 136 | 264 | 583 | 711 | 134 | 38 | 70 | 64 => return false,
                _ => {}
            }
        }
        let (opc, src) = match w.xo5() { 20 => (0x5C, b), 21 => (0x58, b), 25 => (0x59, c), _ => return false };
        if w.rc() { return false; }
        let a = &mut self.a;
        a.ld32(RCX, cpu(FPSCR));
        a.test_ri(RCX, 3); // FPSCR[RN] != 0: tier 0 refuses by name
        self.slow_if(Cc::Ne, i);
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 0, f(a_));
        a.sse_m(0xF2, opc, 0, f(src));
        if single && self.env.mutant != MUT_FP_SINGLE {
            a.sse_r(0xF2, 0x5A, 0, 0); // cvtsd2ss
            a.sse_r(0xF3, 0x5A, 0, 0); // cvtss2sd
        }
        a.movq_rx(RAX, 0);
        a.mov_rr64(RCX, RAX);
        a.sh_ri64(Sh::Shr, RCX, 52);
        a.alu_ri(Alu::And, RCX, 0x7FF);
        a.alu_ri(Alu::Cmp, RCX, 0x7FF);
        self.slow_if(Cc::E, i);
        self.a.st64(f(d), RAX);
        self.fprf(single);
        true
    }

    /// FPSCR[FPRF] = the class of the finite result in rax (fpscr::classify): +-zero, +-denormal (below the single or the
    /// double normal range), +-normal.
    fn fprf(&mut self, single: bool) {
        let a = &mut self.a;
        a.mov_rr64(RDX, RAX);
        a.sh_ri64(Sh::Shr, RDX, 63);
        a.mov_rr64(RCX, RAX);
        a.alu_rr64(Alu::Add, RCX, RCX);
        let zero = a.jcc(Cc::E);
        a.sh_ri64(Sh::Shr, RCX, 53);
        a.alu_ri(Alu::Cmp, RCX, if single { 897 } else { 1 });
        let den = a.jcc(Cc::B);
        a.mov_rr(RAX, RDX);
        a.sh_ri(Sh::Shl, RAX, 2);
        a.alu_ri(Alu::Add, RAX, 0b00100);
        let d1 = a.jmp();
        a.here(den);
        a.mov_rr(RAX, RDX);
        a.sh_ri(Sh::Shl, RAX, 2);
        a.alu_ri(Alu::Add, RAX, 0b10100);
        let d2 = a.jmp();
        a.here(zero);
        a.mov_rr(RAX, RDX);
        a.sh_ri(Sh::Shl, RAX, 4);
        a.alu_ri(Alu::Add, RAX, 0b00010);
        a.here(d1);
        a.here(d2);
        a.sh_ri(Sh::Shl, RAX, 12);
        a.ld32(RCX, cpu(FPSCR));
        a.alu_ri(Alu::And, RCX, !0x1F000);
        a.alu_rr(Alu::Or, RCX, RAX);
        a.st32(cpu(FPSCR), RCX);
    }

    /// fcmpu / fcmpo of ordered operands: CR field `bf` and FPSCR[FPCC] = FL FG FE (FU 0); FPRF[C] untouched.
    fn fcmp(&mut self, i: usize, bf: u8, ra: usize, rb: usize) {
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 0, f(ra));
        a.sse_m(0x66, 0x2E, 0, f(rb)); // ucomisd
        self.slow_if(Cc::P, i);
        let a = &mut self.a;
        a.setcc(Cc::B, RAX);
        a.setcc(Cc::A, RCX);
        a.setcc(Cc::E, RDX);
        a.movzx8_rr(RAX, RAX);
        a.movzx8_rr(RCX, RCX);
        a.movzx8_rr(RDX, RDX);
        a.sh_ri(Sh::Shl, RAX, 3);
        a.sh_ri(Sh::Shl, RCX, 2);
        a.alu_rr(Alu::Add, RDX, RDX);
        a.alu_rr(Alu::Or, RAX, RCX);
        a.alu_rr(Alu::Or, RAX, RDX);
        a.mov_rr(RDX, RAX);
        a.sh_ri(Sh::Shl, RDX, 12);
        a.ld32(RCX, cpu(FPSCR));
        a.alu_ri(Alu::And, RCX, !0xF000);
        a.alu_rr(Alu::Or, RCX, RDX);
        a.st32(cpu(FPSCR), RCX);
        let sh = 28 - 4 * (bf as u32 & 7);
        if sh != 0 { a.sh_ri(Sh::Shl, RAX, sh as u8); }
        a.ld32(RCX, cpu(CR));
        a.alu_ri(Alu::And, RCX, !(0xF << sh));
        a.alu_rr(Alu::Or, RCX, RAX);
        a.st32(cpu(CR), RCX);
    }

    /// Opcode-31 forms tier 1 leaves Generic that WC3 runs hot.
    fn x31(&mut self, i: usize, w: W) -> bool {
        if w.op() != 31 { return false; }
        let (t, a_, b) = (w.rt() as u8, w.ra() as u8, w.rb() as u8);
        let x = |e: &mut Self, n: u8| { let ea = Ea { ra: a_, rb: Some(b), imm: 0, update: false }; e.ea(i, ea, n); ea };
        let at = mx(R12, RAX, 1);
        match w.xo() {
            792 => self.sraw(t, a_, b, w.rc()),
            922 | 954 if w.rc() => {
                if w.xo() == 922 { self.a.movsx16(RDX, g(t)) } else { self.a.movsx8(RDX, g(t)) }
                self.a.st32(g(a_), RDX);
                self.cr0_of_edx();
            }
            _ if w.rc() => return false,
            // loads: lwzx lbzx (RA = 0 only reach here), lhzx lhax, lwbrx lhbrx
            23 | 87 | 279 | 343 | 534 | 790 => {
                let n = match w.xo() { 23 | 534 => 4, 87 => 1, _ => 2 };
                x(self, n);
                let a = &mut self.a;
                match w.xo() {
                    23 => { a.ld32(RDX, at); a.bswap32(RDX); }
                    534 => a.ld32(RDX, at),
                    87 => a.movzx8(RDX, at),
                    790 => a.movzx16(RDX, at),
                    _ => { a.movzx16(RDX, at); a.bswap32(RDX); a.sh_ri(if w.xo() == 343 { Sh::Sar } else { Sh::Shr }, RDX, 16); }
                }
                a.st32(g(t), RDX);
            }
            // stores: stwx stbx (RA = 0), sthx, stwbrx sthbrx
            151 | 215 | 407 | 662 | 918 => {
                let n = match w.xo() { 151 | 662 => 4, 215 => 1, _ => 2 };
                let ea = x(self, n);
                self.store_guard(i, ea, n);
                let a = &mut self.a;
                a.ld32(RDX, g(t));
                match w.xo() {
                    151 => { a.bswap32(RDX); a.st32(at, RDX); }
                    662 => a.st32(at, RDX),
                    215 => a.st8(at, RDX),
                    918 => a.st16(at, RDX),
                    _ => { a.bswap32(RDX); a.sh_ri(Sh::Shr, RDX, 16); a.st16(at, RDX); }
                }
            }
            _ => return false,
        }
        true
    }

    /// sraw: n = rB & 0x3F; n > 31 fills with the sign (CA = negative), else srawi's rule with a variable count.
    fn sraw(&mut self, s: u8, d: u8, b: u8, rc: bool) {
        let a = &mut self.a;
        a.ld32(RCX, g(b));
        a.alu_ri(Alu::And, RCX, 0x3F);
        a.ld32(RDX, g(s));
        a.mov_rr(R8, RDX);
        a.alu_ri(Alu::Cmp, RCX, if self.env.mutant == MUT_SRAW_BIG { 63 } else { 31 });
        let big = a.jcc(Cc::A);
        a.sh_cl(Sh::Sar, RDX);
        a.mov_ri(RAX, 1);
        a.sh_cl(Sh::Shl, RAX);
        a.alu_ri(Alu::Sub, RAX, 1);
        a.test_rr(R8, RAX);
        a.setcc(Cc::Ne, RAX);
        let done = a.jmp();
        a.here(big);
        a.sh_ri(Sh::Sar, RDX, 31);
        a.mov_ri(RAX, 1);
        a.here(done);
        a.test_rr(R8, R8);
        a.setcc(Cc::S, RCX);
        a.and8_rr(RAX, RCX);
        a.st32(g(d), RDX);
        a.mov_rr(R8, RAX);
        self.ca_from_r8();
        if rc { self.cr0_of_edx(); }
    }
}
