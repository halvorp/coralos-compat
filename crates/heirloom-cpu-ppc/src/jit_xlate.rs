//! Tier 2's TRANSLATED FUNCTIONS AND LOOPS (h86 aot-xlate, 2026-11-04; the WC3 council's prototype step 4, chosen by the Z83's own
//! guest-PC profile): a whole guest function that the profile ranks hot and that is structurally simple is compiled as ONE
//! block that computes the function's exact architectural effect, instead of one block per guest basic block.
//!
//! WHY: on the Z83 the JIT is front-end bound (FETCH_STALL.ICACHE_FILL 25 % of core cycles, NO_ALLOC.NOT_DELIVERED ~45 %,
//! 12.7 host instructions a guest instruction); the guards cost nothing measurable (jitablate). The cost is the VOLUME of code
//! per guest instruction. A routine translated whole needs a fraction of it: its loads go straight into host registers, its
//! compares feed its branches directly, and the guest state it leaves behind is written once, at its exit.
//!
//! RECOGNISED BY ITS CODE, never by an address: a function is translated only when the guest words at its entry are EXACTLY a
//! known shape (position-independent: its branches are pc-relative) and it lies in one code page (the block's entry checks that
//! page's generation, so a rewrite of any of its words drops the translation as it drops any block). Every input the
//! translation cannot serve exactly - an operand range outside RAM, a NaN (tier 1's FP rules), too little budget for the whole
//! function - BAILS before any guest state is written, and tier 1 runs the original instructions.
//!
//! EXACT: the translated block leaves every register, CR field, FPSCR bit, the retired-instruction count and the pc exactly as the
//! original instructions would (JitLockstep compares them on every entry; tests/tier2.rs runs the shapes through it).
//!
//! THE SHAPES (Warcraft III gameplay on the Z83, 2026-11-04, symbolised against the static map):
//! * [`CMP16F`]: `r3 = (any of 16 single-precision floats at r3 != the one at r4)` - lfs/lfs/fcmpu/bne x 16, early exit on the
//!   first difference (a 4x4 matrix compare: the game's GL state cache asks it before every matrix upload). Rank 1 of the
//!   gameplay profile: 7.3 % of JIT time, 5.7 % of the guest's instructions, ~2,200 calls a frame. Only ever reached
//!   by direct calls (no TVector), so a host native - an exit from JIT code to Rust per call - would cost more than it saves.
//!   h86 fnreg: a vector fast path for the 64 bytes bit-identical (the census measured the scalar walk at ~330 host
//!   instructions a call, 5.9 % of the JIT's).
//! * [`Shape::Memcmp`] and [`Shape::BlendAdd`] (h86 fnreg): byte-string routines, in jit_xbytes.
use crate::jit_asm::*;
use crate::jit_emit::{cpu, cpu_m, fpr, ctx, Env, E, CR, LR};
use crate::jit::{Ctx, EXIT_BAIL};
use alloc::vec::Vec;
use core::mem::offset_of;

/// A shape tier 2 translates whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Cmp16F,
    /// jit_xbytes::MEMCMPB: an unsigned byte compare of r5 bytes (a whole function).
    Memcmp,
    /// jit_xbytes::BLENDADD: a saturating byte add of CTR groups of four (a loop, at its head).
    BlendAdd,
}

/// The 68 words of [`Shape::Cmp16F`]: `lfs f1,0(r3); li r0,0; lfs f0,0(r4); fcmpu cr0,f1,f0; bne done1`, then 15 x
/// `lfs f1,4k(r3); lfs f0,4k(r4); fcmpu cr0,f1,f0; bne done1` (the last a `beq done0`), `done1: li r0,1`,
/// `done0: clrlwi r3,r0,24; blr`.
pub const CMP16F: [u32; 68] = {
    let mut w = [0u32; 68];
    w[0] = 0xC023_0000; // lfs f1,0(r3)
    w[1] = 0x3800_0000; // li r0,0
    w[2] = 0xC004_0000; // lfs f0,0(r4)
    w[3] = 0xFC01_0000; // fcmpu cr0,f1,f0
    w[4] = 0x4082_00F4; // bne +0xF4 -> li r0,1
    let mut k = 1;
    while k < 16 {
        let at = 5 + 4 * (k - 1);
        w[at] = 0xC023_0000 | (4 * k) as u32;
        w[at + 1] = 0xC004_0000 | (4 * k) as u32;
        w[at + 2] = 0xFC01_0000;
        // bne to li r0,1 (word 65) from word at+3; the 16th pair: beq to clrlwi (word 66)
        let from = at + 3;
        w[from] = if k < 15 { 0x4082_0000 | ((65 - from) * 4) as u32 } else { 0x4182_0000 | ((66 - from) * 4) as u32 };
        k += 1;
    }
    w[65] = 0x3800_0001; // li r0,1
    w[66] = 0x5403_063E; // clrlwi r3,r0,24
    w[67] = 0x4E80_0020; // blr
    w
};

/// The shape whose words start at `pc`, read through `word`, if one does and it lies in one code page of `page_shift` bits.
/// The first word decides which shape is compared at all (one read for every other block).
pub(crate) fn recognise(pc: u32, page_shift: u32, word: impl Fn(u32) -> Option<u32>) -> Option<Shape> {
    use crate::jit_xbytes::{BLENDADD, MEMCMPB};
    if pc & 3 != 0 { return None; }
    let (s, words): (Shape, &[u32]) = match word(pc)? {
        w if w == CMP16F[0] => (Shape::Cmp16F, &CMP16F),
        w if w == MEMCMPB[0] => (Shape::Memcmp, &MEMCMPB),
        w if w == BLENDADD[0] => (Shape::BlendAdd, &BLENDADD),
        _ => return None,
    };
    let len = 4 * words.len() as u32;
    if pc >> page_shift != (pc.wrapping_add(len - 1)) >> page_shift { return None; }
    words.iter().enumerate().all(|(i, &w)| word(pc + 4 * i as u32) == Some(w)).then_some(s)
}

/// The fewest instructions a shape retires (its entry's budget test; a variable-length shape tests its own exact need after
/// it, before writing anything).
pub(crate) fn max_insns(s: Shape) -> u32 {
    match s { Shape::Cmp16F => 67, Shape::Memcmp => 9, Shape::BlendAdd => 36 }
}

/// The budget the dispatcher wants before entering the shape (Slot::len): its longest path, or for a variable-length shape
/// the need of a typical call - 64 bytes / 64 groups - so a lockstep entry (budget = this + up to 63) usually runs the
/// translation instead of bailing, and lockstep really compares it.
pub(crate) fn slot_len(s: Shape) -> u32 {
    match s { Shape::Cmp16F => 67, Shape::Memcmp => 6 * 64 + 12, Shape::BlendAdd => 36 * 64 }
}

/// Compile shape `s` at `pc` (code page `page` at generation `gen`): the code and the offset of its inner entry.
pub(crate) fn compile(s: Shape, pc: u32, gen: u64, page: usize, env: &mut Env) -> (Vec<u8>, usize) {
    let mut e = E::new(&[], pc, env);
    e.cat(crate::jit_census::XLATE);
    let mut bails = Vec::new();
    // the block's own entry checks (jit_emit::compile's): its code page's generation (unless KNOB_NO_ENTRY_GEN), then a budget
    // that covers the longest path through the whole function
    let check_gen = e.env.knobs & heirloom_cpu_abi::ppc::KNOB_NO_ENTRY_GEN == 0;
    let a = &mut e.a;
    if check_gen {
        a.ld64(RAX, ctx(offset_of!(Ctx, gen)));
        let at = m(RAX, (8 * page) as i32);
        if gen <= i32::MAX as u64 { a.alu_mi64(Alu::Cmp, at, gen as u32); } else { a.mov_ri64(RCX, gen); a.alu_rm64(Alu::Cmp, RCX, at); }
        bails.push(a.jcc(Cc::Ne));
    }
    let inner = a.pos();
    a.alu_mi64(Alu::Cmp, ctx(offset_of!(Ctx, left)), max_insns(s));
    bails.push(a.jcc(Cc::B));
    match s { Shape::Cmp16F => e.cmp16f(&mut bails), Shape::Memcmp => e.memcmpb(&mut bails), Shape::BlendAdd => e.blendadd(pc, &mut bails) }
    e.exit_tails();
    let a = &mut e.a;
    for b in bails { a.here(b); }
    a.st32i(cpu(crate::jit_emit::PC), pc);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    e.env.tag_runs = e.a.tags.take().unwrap_or_default();
    (e.a.b, inner)
}

impl E<'_, '_> {
    /// [`Shape::Cmp16F`]: find the first pair k (of 16) whose floats differ (k' = min(k, 15)), then leave the original's state:
    /// f1 / f0 = pair k' widened, CR0 and FPSCR[FPCC] = fcmpu of pair k', r0 = r3 = (a pair differs), the retired count
    /// 5 + 4 k' + (differs ? 3 : 2), and the pc = LR (its blr). Bails (nothing written) on a range outside RAM or a NaN.
    fn cmp16f(&mut self, bails: &mut Vec<Fix>) {
        // it writes FPSCR[FPCC]: a pending FPRF first
        crate::jit_inline::emit_fprf_settle(&mut self.a);
        let (r3, r4) = (self.gr(3), self.gr(4));
        let a = &mut self.a;
        // the two 64-byte operand ranges must lie in RAM (lim4: the last address a 4-byte access may start at)
        a.ld32o(RSI, r3);
        a.ld32o(RDI, r4);
        for r in [RSI, RDI] {
            a.lea64(RCX, m(r, 60));
            a.alu_rm64(Alu::Cmp, RCX, ctx(offset_of!(Ctx, lim4)));
            bails.push(a.jcc(Cc::A));
        }
        // h86 fnreg: the common case first - the 64 bytes bit-identical and no float with exponent 0xFF (an infinity or a NaN:
        // fcmpu's unordered answer is tier 1's) - is every pair equal, k' = 15, without the scalar walk (~2,000 of ~2,200 calls
        // a frame upload no matrix). Per 4-byte lane of the guest's big-endian float: exponent 0xFF = bits 0-6 and 15 set.
        // Anything else (a difference, +0 against -0, an infinity, a NaN) takes the walk below from pair 0.
        let fast = {
            a.mov_ri(RAX, if self.env.mutant == crate::jit_mutants::MUT_XLATE_FAST_NAN { 0 } else { 0x807F });
            a.movd_xr(7, RAX);
            a.pshufd(7, 7, 0);
            for j in 0..4u8 {
                a.movdqu_ld(j, mxd(R12, RSI, 1, 16 * j as i32));
                a.movdqu_ld(4, mxd(R12, RDI, 1, 16 * j as i32));
                a.pcmpeqd(4, j);
                if j == 0 { a.movdqa(5, 4) } else { a.pand(5, 4) }
                a.pand(j, 7);
                a.pcmpeqd(j, 7);
                if j == 0 { a.movdqa(6, 0) } else { a.por(6, j) }
            }
            a.pmovmskb(RAX, 5);
            a.pmovmskb(RCX, 6);
            if self.env.mutant == crate::jit_mutants::MUT_XLATE_FAST_NAN { a.alu_rr(Alu::Xor, RCX, RCX); }
            a.alu_ri(Alu::Cmp, RAX, 0xFFFF);
            let walk = a.jcc(Cc::Ne);
            a.test_rr(RCX, RCX);
            let walk2 = a.jcc(Cc::Ne);
            a.mov_ri(R8, 60);
            a.alu_rr(Alu::Xor, RSI, RSI);
            let f = a.jmp();
            a.here(walk);
            a.here(walk2);
            f
        };
        a.alu_rr(Alu::Xor, R8, R8); // 4 k
        let top = a.pos();
        // a = [ram + r3 + 4k], b = [ram + r4 + 4k], big-endian
        a.lea64(RAX, mx(RSI, R8, 1));
        a.ld32(RAX, mx(R12, RAX, 1));
        a.bswap32(RAX);
        a.lea64(RDX, mx(RDI, R8, 1));
        a.ld32(RDX, mx(R12, RDX, 1));
        a.bswap32(RDX);
        // a NaN is tier 1's (lfs and fcmpu take their slow paths there): |x| > +inf
        for r in [RAX, RDX] {
            a.mov_rr(RCX, r);
            a.alu_ri(Alu::And, RCX, 0x7FFF_FFFF);
            a.alu_ri(Alu::Cmp, RCX, 0x7F80_0000);
            bails.push(a.jcc(Cc::A));
        }
        // equal as floats (no NaN here): the same bits, or both zeros of either sign
        a.alu_rr(Alu::Cmp, RAX, RDX);
        let same = a.jcc(Cc::E);
        a.mov_rr(RCX, RAX);
        a.alu_rr(Alu::Or, RCX, RDX);
        a.test_ri(RCX, 0x7FFF_FFFF);
        let differ = a.jcc(Cc::Ne);
        a.here(same);
        a.alu_ri(Alu::Add, R8, 4);
        a.alu_ri(Alu::Cmp, R8, 64);
        let back = a.jcc(Cc::B);
        a.patch(back, top);
        // every pair equal: k' = 15, r0 = 0, 2 instructions after the last pair
        a.mov_ri(R8, 60);
        a.alu_rr(Alu::Xor, RSI, RSI); // RSI: the "differs" flag from here (the operand bases are read again from r3 / r4)
        let join = a.jmp();
        a.here(differ);
        a.mov_ri(RSI, 1);
        a.here(join);
        a.here(fast);
        // f1 / f0 = pair k' widened to double (lfs); RSI holds the flag now, so the bases are read again from r3 / r4
        let (r3, r4) = (self.gr(3), self.gr(4));
        let a = &mut self.a;
        a.ld32o(RDI, r3);
        a.alu_rr64(Alu::Add, RDI, R8);
        a.ld32(RAX, mx(R12, RDI, 1));
        a.bswap32(RAX);
        a.ld32o(RDI, r4);
        a.alu_rr64(Alu::Add, RDI, R8);
        a.ld32(RDX, mx(R12, RDI, 1));
        a.bswap32(RDX);
        a.movd_xr(0, RAX);
        a.sse_r(0xF3, 0x5A, 0, 0); // cvtss2sd xmm0, xmm0
        a.movd_xr(1, RDX);
        a.sse_r(0xF3, 0x5A, 1, 1); // cvtss2sd xmm1, xmm1
        a.sse_m(0xF2, 0x11, 0, cpu_m(fpr(1)));
        a.sse_m(0xF2, 0x11, 1, cpu_m(fpr(0)));
        // fcmpu cr0, f1, f0 (ordered): FL FG FE into CR0 and FPSCR[FPCC] (FU 0, FPRF[C] untouched)
        a.sse_r(0x66, 0x2E, 0, 1); // ucomisd xmm0, xmm1
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
        let fpscr = cpu(offset_of!(crate::cpu::Cpu, fpscr));
        a.ld32(RCX, fpscr);
        a.alu_ri(Alu::And, RCX, !0xF000);
        a.alu_rr(Alu::Or, RCX, RDX);
        a.st32(fpscr, RCX);
        a.sh_ri(Sh::Shl, RAX, 28);
        let c = self.rw(CR);
        self.a.alu_oi(Alu::And, c, 0x0FFF_FFFF);
        self.a.alu_or(Alu::Or, c, RAX);
        // r0 = r3 = the flag (clrlwi r3, r0, 24 of 0 or 1)
        let (r0, r3) = (self.gw(0), self.gw(3));
        self.a.st32o(r0, RSI);
        self.a.st32o(r3, RSI);
        // retired: 5 + 4 k' + 2 + flag = 7 + r8 + rsi
        let a = &mut self.a;
        a.alu_mr64(Alu::Sub, ctx(offset_of!(Ctx, left)), R8);
        a.alu_mr64(Alu::Sub, ctx(offset_of!(Ctx, left)), RSI);
        a.alu_mi64(Alu::Sub, ctx(offset_of!(Ctx, left)), 7);
        // blr: on to LR through the dispatch table
        let lr = self.rw(LR);
        self.a.ld32o(R8, lr);
        self.chain_dyn(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape's words are Warcraft III's (the Z83 profile's rank 1, 0x2fa7d0 in its code section, as dumped from the user's
    /// copy: the branches' displacements are checked here word by word).
    #[test]
    fn the_cmp16f_words_are_the_dumped_function() {
        assert_eq!(&CMP16F[..9], &[0xC023_0000, 0x3800_0000, 0xC004_0000, 0xFC01_0000, 0x4082_00F4, 0xC023_0004, 0xC004_0004, 0xFC01_0000, 0x4082_00E4]);
        assert_eq!(CMP16F[64], 0x4182_0008, "the 16th pair: beq to clrlwi");
        assert_eq!(CMP16F[60], 0x4082_0014);
        assert_eq!(&CMP16F[65..], &[0x3800_0001, 0x5403_063E, 0x4E80_0020]);
        let mem = |a: u32| Some(CMP16F[((a - 0x1000) / 4) as usize]);
        assert_eq!(recognise(0x1000, 12, mem), Some(Shape::Cmp16F));
        assert_eq!(recognise(0x1F00, 12, |a| Some(CMP16F[((a - 0x1F00) / 4) as usize % 68])), None, "across a code page: never");
        let mut w = CMP16F;
        w[30] ^= 4;
        assert_eq!(recognise(0x1000, 12, |a| Some(w[((a - 0x1000) / 4) as usize])), None, "one word off: never");
    }
}
