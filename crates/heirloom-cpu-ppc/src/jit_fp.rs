//! Tier 2: inline code for the FP forms WC3 gameplay still handed to the slow stub after increment 1 (WC3_JIT_CENSUS
//! 2026-11-02, ppc-jit2: fmadds/fmsubs/fnmadds/fnmsubs ~0.9% of gameplay instructions, fctiwz ~0.03%). Each restates tier
//! 0 (fpu.rs) exactly on its common path and leaves everything else to the slow stub BEFORE writing any state:
//! - the single fused multiply-add family (opcode 59, xo 28-31), Rc=0, FPSCR[RN] = 0, all three operands finite and
//!   single-representable: tier 0's `fma_single` - the product is exact in f64, TwoSum gives the exact sum as s + t, s is
//!   rounded to ODD when t != 0, then ONE f64 -> f32 nearest-even rounding (no double rounding: f64 has >= 2 more bits). A
//!   non-finite s or single result (overflow) goes slow. No FMA instruction is used (the Z8350 has no FMA3).
//! - fctiwz, Rc=0: CVTTSD2SI; 0x80000000 (NaN, out of range, or exactly -2^31) goes slow; an inexact result only when
//!   FPSCR[XX] is already set and XE is clear (then raising XX changes nothing but FI), else slow.
use crate::fpscr as fs;
use crate::jit_asm::*;
use crate::jit_emit::{cpu, fpr, E};
use crate::cpu::Cpu;
use core::mem::offset_of;

const FPSCR: usize = offset_of!(Cpu, fpscr);
fn f(n: usize) -> M { crate::jit_emit::cpu_m(fpr(n as u8)) }
const SIGN: u64 = 1 << 63;

impl E<'_, '_> {
    /// Slow unless rax's f64 bit pattern is finite (exponent != 0x7FF); rcx is clobbered.
    fn slow_unless_finite(&mut self, i: usize) {
        let a = &mut self.a;
        a.mov_rr64(RCX, RAX);
        a.sh_ri64(Sh::Shr, RCX, 52);
        a.alu_ri(Alu::And, RCX, 0x7FF);
        a.alu_ri(Alu::Cmp, RCX, 0x7FF);
        self.slow_if(Cc::E, i);
    }

    /// fmadds / fmsubs / fnmadds / fnmsubs (xo5 29 / 28 / 31 / 30), Rc=0: frD = [-](frA * frC [+-] frB) rounded once to single.
    pub(crate) fn fmadds(&mut self, i: usize, xo5: u32, d: usize, a_: usize, b: usize, c: usize) {
        let (sub, neg) = (xo5 & 1 == 0, xo5 >= 30);
        self.a.ld32(RCX, cpu(FPSCR));
        self.a.test_ri(RCX, 3); // FPSCR[RN] != 0: tier 0 refuses by name
        self.slow_if(Cc::Ne, i);
        // every operand finite and single-representable (else tier 0 takes the plain f64 fma: the slow path)
        for src in [a_, c, b] {
            let a = &mut self.a;
            a.sse_m(0xF2, 0x10, 1, f(src));
            a.sse_r(0xF2, 0x5A, 1, 1); // cvtsd2ss
            a.sse_r(0xF3, 0x5A, 1, 1); // cvtss2sd
            a.movq_rx(RAX, 1);
            a.alu_rm64(Alu::Cmp, RAX, f(src));
            self.slow_if(Cc::Ne, i);
            self.slow_unless_finite(i);
        }
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 0, f(a_));
        a.sse_m(0xF2, 0x59, 0, f(c)); // p = a * c (exact)
        a.ld64(RDX, f(b));
        if sub { a.mov_ri64(RCX, SIGN); a.alu_rr64(Alu::Xor, RDX, RCX); }
        a.movq_xr(1, RDX); // b (negated for the subtract forms)
        a.movapd(2, 0);
        a.sse_r(0xF2, 0x58, 2, 1); // s = p + b
        a.movq_rx(RAX, 2);
        self.slow_unless_finite(i);
        let a = &mut self.a;
        // TwoSum: t = (p - (s - bb)) + (b - bb), bb = s - p; p + b == s + t exactly
        a.movapd(3, 2);
        a.sse_r(0xF2, 0x5C, 3, 0); // bb
        a.movapd(4, 2);
        a.sse_r(0xF2, 0x5C, 4, 3); // s - bb
        a.movapd(5, 0);
        a.sse_r(0xF2, 0x5C, 5, 4); // p - (s - bb)
        a.movapd(6, 1);
        a.sse_r(0xF2, 0x5C, 6, 3); // b - bb
        a.sse_r(0xF2, 0x58, 5, 6); // t
        a.movq_rx(RCX, 5);
        a.mov_rr64(RDX, RCX);
        a.alu_rr64(Alu::Add, RDX, RDX);
        let exact = a.jcc(Cc::E); // t == +-0: s is the exact sum
        let twice = if self.env.mutant == crate::jit_emit::MUT_FMA_TWICE { Some(self.a.jmp()) } else { None };
        let a = &mut self.a;
        // round s to odd: one ulp toward zero when t points the other way (s != 0 here), then force the low bit
        a.alu_rr64(Alu::Xor, RCX, RAX);
        a.test_rr64(RCX, RCX);
        let same = a.jcc(Cc::Ns);
        a.alu_ri64(Alu::Sub, RAX, 1);
        a.here(same);
        a.alu_ri64(Alu::Or, RAX, 1);
        a.movq_xr(2, RAX);
        a.here(exact);
        if let Some(j) = twice { a.here(j); }
        a.sse_r(0xF2, 0x5A, 2, 2); // the one rounding to single
        a.sse_r(0xF3, 0x5A, 2, 2);
        a.movq_rx(RAX, 2);
        self.slow_unless_finite(i); // overflow: OX is tier 0's
        let a = &mut self.a;
        if neg { a.mov_ri64(RCX, SIGN); a.alu_rr64(Alu::Xor, RAX, RCX); }
        a.st64(f(d), RAX);
        self.fprf(true);
    }

    /// fctiwz, Rc=0: frD = 0xFFF80000_00000000 | (frB truncated to i32).
    pub(crate) fn fctiwz(&mut self, i: usize, d: usize, b: usize) {
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 0, f(b));
        a.cvttsd2si(RAX, 0);
        a.alu_ri(Alu::Cmp, RAX, 0x8000_0000);
        self.slow_if(Cc::E, i);
        let a = &mut self.a;
        a.cvtsi2sd(1, RAX);
        a.sse_r(0x66, 0x2E, 1, 0); // ucomisd: equal = exact (no NaN reaches here)
        let exact = a.jcc(Cc::E);
        // inexact: tier 0 raises XX and sets FI; only when XX is already set and XE clear is that just `FI |= 1`
        a.ld32(RCX, cpu(FPSCR));
        a.test_ri(RCX, fs::XX);
        self.slow_if(Cc::E, i);
        self.a.test_ri(RCX, fs::XE);
        self.slow_if(Cc::Ne, i);
        let a = &mut self.a;
        a.alu_ri(Alu::Or, RCX, fs::FI);
        a.st32(cpu(FPSCR), RCX);
        a.here(exact);
        a.mov_rr(RAX, RAX); // zero-extend
        a.mov_ri64(RDX, 0xFFF8_0000_0000_0000);
        a.alu_rr64(Alu::Or, RDX, RAX);
        a.st64(f(d), RDX);
    }
}
