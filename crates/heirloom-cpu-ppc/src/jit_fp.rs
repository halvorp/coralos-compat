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
    /// Slow unless rax's f64 bit pattern is finite (exponent != 0x7FF): rcx = the bits less the sign, shifted up one (kept
    /// for a caller's further tests), at or above Ctx::k_inf2 = exponent 0x7FF.
    pub(crate) fn slow_unless_finite(&mut self, i: usize) {
        let t = self.cat(crate::jit_census::FP_GUARD);
        let a = &mut self.a;
        a.lea64(RCX, mx(RAX, RAX, 1));
        a.alu_rm64(Alu::Cmp, RCX, crate::jit_emit::ctx(offset_of!(crate::jit::Ctx, k_inf2)));
        self.slow_if(Cc::Ae, i);
        self.a.set_tag(t);
    }

    /// fmadds / fmsubs / fnmadds / fnmsubs (xo5 29 / 28 / 31 / 30), Rc=0: frD = [-](frA * frC [+-] frB) rounded once to single.
    pub(crate) fn fmadds(&mut self, i: usize, xo5: u32, d: usize, a_: usize, b: usize, c: usize) {
        let (sub, neg) = (xo5 & 1 == 0, xo5 >= 30);
        self.a.ld32(RCX, cpu(FPSCR));
        self.a.test_ri(RCX, 3); // FPSCR[RN] != 0: tier 0 refuses by name
        self.slow_if(Cc::Ne, i);
        // every operand single-representable (else tier 0 takes the plain f64 fma: the slow path). Not tested here: that each
        // is FINITE - a non-finite operand (single-representable or not) makes the sum s below non-finite (a, c finite
        // singles: their product cannot overflow f64), and a non-finite s is the slow path's before anything is written
        // a and c together (one packed round trip to single and back, compared lane by lane: a NaN compares unequal and
        // goes slow, as it would below), b alone (its bits; xmm3 keeps it)
        let t = self.cat(crate::jit_census::FP_GUARD);
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 1, f(a_));
        a.sse_m(0x66, 0x16, 1, f(c)); // movhpd xmm1, [c]
        a.sse_r(0x66, 0x5A, 2, 1); // cvtpd2ps
        a.cvtps2pd(2, 2);
        a.cmpneqpd(2, 1);
        a.movmskpd(RCX, 2);
        a.test_rr(RCX, RCX);
        self.slow_if(Cc::Ne, i);
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 3, f(b));
        a.sse_r(0xF2, 0x5A, 4, 3); // cvtsd2ss
        a.sse_r(0xF3, 0x5A, 4, 4); // cvtss2sd
        a.movq_rx(RAX, 4);
        a.alu_rm64(Alu::Cmp, RAX, f(b));
        self.slow_if(Cc::Ne, i);
        self.a.set_tag(t);
        let a = &mut self.a;
        a.sse_m(0xF2, 0x10, 0, f(a_));
        a.sse_m(0xF2, 0x59, 0, f(c)); // p = a * c (exact)
        if sub {
            a.ld64(RDX, f(b));
            a.mov_ri64(RCX, SIGN);
            a.alu_rr64(Alu::Xor, RDX, RCX);
            a.movq_xr(1, RDX); // b negated for the subtract forms
        } else {
            a.movapd(1, 3); // b
        }
        a.movapd(2, 0);
        a.sse_r(0xF2, 0x58, 2, 1); // s = p + b
        a.movq_rx(RAX, 2);
        self.slow_unless_finite(i);
        // s rounds to the same single as the exact p + b unless s is a MIDPOINT of the single grid (a 25-bit significand:
        // mantissa bit 28 set, bits 0-27 clear) - the exact sum and s are on the same side of every single midpoint (each is
        // a double, and s is the double nearest the sum) - as long as that grid is uniform at s (|s| >= 2^-126, single
        // normal range; rcx = s's bits shifted up one, from slow_unless_finite). A midpoint, a tiny s or +-0 takes the
        // exact path out of line (the shared stub: TwoSum, s rounded to odd when inexact, jit_fp emit_fma_odd).
        let t = self.cat(crate::jit_census::FP_GUARD);
        let a = &mut self.a;
        a.alu_rm64(Alu::Cmp, RCX, crate::jit_emit::ctx(offset_of!(crate::jit::Ctx, k_tiny2)));
        let tiny = a.jcc(Cc::B);
        a.mov_rr(RCX, RAX);
        a.alu_ri(Alu::And, RCX, 0x1FFF_FFFF);
        a.alu_ri(Alu::Cmp, RCX, 0x1000_0000);
        let mid = a.jcc(Cc::E);
        self.a.set_tag(t);
        let back = self.a.pos();
        if self.env.mutant == crate::jit_emit::MUT_FMA_NO_MIDPOINT { self.a.patch(mid, back); self.fma_outs.push((alloc::vec![tiny], back)); }
        else { self.fma_outs.push((alloc::vec![tiny, mid], back)); }
        let a = &mut self.a;
        a.sse_r(0xF2, 0x5A, 2, 2); // the one rounding to single
        a.sse_r(0xF3, 0x5A, 2, 2);
        a.movq_rx(RAX, 2);
        self.slow_unless_finite(i); // overflow: OX is tier 0's
        let a = &mut self.a;
        if neg { a.mov_ri64(RCX, SIGN); a.alu_rr64(Alu::Xor, RAX, RCX); }
        a.st64(f(d), RAX);
        self.fprf(true);
    }

}

/// The shared stub of fmadds's exact path (jit.rs `stubs`; called from the out-of-line code of an fmadds whose s was a
/// single midpoint, tiny or +-0): xmm0 = p (exact), xmm1 = b (negated for the subtract forms), xmm2 = s = p + b, rax = s's
/// bits. TwoSum gives t with p + b == s + t exactly; t != 0: s is rounded to ODD (one ulp toward zero when t points the
/// other way, then the low bit forced) into xmm2 and rax - the single rounding after it is then correct (f64 has >= 2 more
/// bits than f32). rcx, rdx, xmm3-6 clobbered. `twice` = planted mutant MUT_FMA_TWICE (s left as it is: two roundings).
pub(crate) fn emit_fma_odd(a: &mut Asm, twice: bool) {
    a.movapd(3, 2);
    a.sse_r(0xF2, 0x5C, 3, 0); // bb = s - p
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
    if !twice {
        a.alu_rr64(Alu::Xor, RCX, RAX);
        a.test_rr64(RCX, RCX);
        let same = a.jcc(Cc::Ns);
        a.alu_ri64(Alu::Sub, RAX, 1);
        a.here(same);
        a.alu_ri64(Alu::Or, RAX, 1);
        a.movq_xr(2, RAX);
    }
    a.here(exact);
    a.ret();
}

impl E<'_, '_> {
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
