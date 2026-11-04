//! Tier 2's translated BYTE-STRING shapes (h86 fnreg, 2026-11-04; jit_xlate's family): two routines that walk guest bytes one
//! at a time, recognised by their exact words, run as native loops over guest RAM - 8 or 16 bytes a host instruction where the
//! original retires 6 or 9 guest instructions a byte - and leave EXACTLY the original's architectural state (every register
//! the routine writes, CR0, XER[CA], CTR, the retired count, the pc). Chosen by measured host instructions on the WC3 gameplay
//! census (callgrind over the JIT arena, per guest function: /build/patches/h86/fnreg), not by a static guess:
//! * [`MEMCMPB`]: `r3 = memcmp-like(r3, r4, r5)` over UNSIGNED bytes - 1 / -1 at the first difference, 0 when the r5 bytes are
//!   equal (a byte loop: lbzu / lbzu / cmplw / beq, addic. / bne). 3.35 % of the JIT's host instructions, ~360 calls a frame,
//!   ~27 bytes a call (~6.9 host instructions each of its 165 guest instructions a call).
//! * [`BLENDADD`]: a LOOP (the shape starts at its head, entered by its back edge or the fall-through into it): CTR groups of
//!   four bytes, `dst[i] = min(a[i] + b[i], 255)` (r7 = a, r6 = b, r5 = dst; bdnz). 2.8 % of the JIT's host instructions, ~3,900
//!   groups a frame. One paddusb per 16 bytes.
//!
//! WHAT THEY REFUSE (a bail before any guest state is written; tier 1 then runs the original words): a length that does not fit
//! the run's budget or is absurd (> 2^20 elements), any byte range that is not wholly inside RAM, and for BLENDADD a destination
//! that partly overlaps a source (an exact alias of a source is fine: each output byte reads only its own index) or that lies
//! on a page the store map does not clear for inline stores (a watched code page: its generation must move, tier 1's write
//! does that). In lockstep the destination range is journaled before it is written (h_note), as every inline store is.
use crate::jit::Ctx;
use crate::jit_asm::*;
use crate::jit_emit::{ctx, E, CTR, LR, XER};
use crate::cpu::XER_CA;
use alloc::vec::Vec;
use core::mem::offset_of;

/// The 20 words of [`crate::jit_xlate::Shape::Memcmp`] (PowerPC, position independent).
pub const MEMCMPB: [u32; 20] = [
    0x38C3_FFFF, // addi r6,r3,-1
    0x38E4_FFFF, // addi r7,r4,-1
    0x3885_0001, // addi r4,r5,1
    0x4800_0030, // b +0x30 (the nop before the count test)
    0x8C66_0001, // loop: lbzu r3,1(r6)
    0x8C07_0001, // lbzu r0,1(r7)
    0x7C03_0040, // cmplw r3,r0
    0x4182_0024, // beq +0x24 (the count test)
    0x8886_0000, // lbz r4,0(r6)
    0x3860_0001, // li r3,1
    0x8807_0000, // lbz r0,0(r7)
    0x7C04_0040, // cmplw r4,r0
    0x4C80_0020, // bgelr
    0x3860_FFFF, // li r3,-1
    0x4E80_0020, // blr
    0x6000_0000, // nop
    0x3484_FFFF, // addic. r4,r4,-1
    0x4082_FFCC, // bne loop
    0x3860_0000, // li r3,0
    0x4E80_0020, // blr
];

/// The 36 words of [`crate::jit_xlate::Shape::BlendAdd`]: four times `lbz r8,j(r7); lbz r0,j(r6); add r8,r8,r0; rlwinm r27,r8,0,23,23;
/// rlwinm r0,r8,24,31,31; subf r0,r0,r27; or r0,r8,r0; stb r0,j(r5)` (the fourth with `addi r7,r7,4` / `addi r6,r6,4` between its
/// loads), then `addi r5,r5,4; bdnz` back to the first word.
pub const BLENDADD: [u32; 36] = {
    let mut w = [0u32; 36];
    let mut j = 0;
    let mut at = 0;
    while j < 4 {
        w[at] = 0x8907_0000 | j; // lbz r8,j(r7)
        at += 1;
        if j == 3 { w[at] = 0x38E7_0004; at += 1; } // addi r7,r7,4
        w[at] = 0x8806_0000 | j; // lbz r0,j(r6)
        at += 1;
        if j == 3 { w[at] = 0x38C6_0004; at += 1; } // addi r6,r6,4
        w[at] = 0x7D08_0214; // add r8,r8,r0
        w[at + 1] = 0x551B_05EE; // rlwinm r27,r8,0,23,23
        w[at + 2] = 0x5500_C7FE; // rlwinm r0,r8,24,31,31
        w[at + 3] = 0x7C00_D850; // subf r0,r0,r27
        w[at + 4] = 0x7D00_0378; // or r0,r8,r0
        w[at + 5] = 0x9805_0000 | j; // stb r0,j(r5)
        at += 6;
        j += 1;
    }
    w[34] = 0x38A5_0004; // addi r5,r5,4
    w[35] = 0x4200_FF74; // bdnz -0x8C (the first word)
    w
};

/// Elements (bytes, groups) above which a shape bails (the budget test needs a bounded product; no real call is near it).
pub(crate) const MAX_ELEMS: u32 = 1 << 20;

impl E<'_, '_> {
    /// [`MEMCMPB`]: n = r5. Equal (k = n): r6 = r3 - 1 + n, r7 = r4 - 1 + n, r0 = b[n - 1] (n > 0; else untouched), r3 = r4 = 0,
    /// CR0 = EQ (addic.'s zero), 9 + 6n retired. First difference at k: r6 = r3 + k, r7 = r4 + k, r4 = a[k], r0 = b[k], CR0 =
    /// cmplw a[k], b[k], r3 = 1 (a[k] > b[k], 16 + 6k retired) or -1 (18 + 6k). XER[CA] = 1 either way (every addic. ran on a
    /// count >= 1). Then the blr.
    pub(crate) fn memcmpb(&mut self, bails: &mut Vec<Fix>) {
        let (o3, o4, o5) = (self.gr(3), self.gr(4), self.gr(5));
        let a = &mut self.a;
        a.ld32o(RCX, o5);
        a.alu_ri(Alu::Cmp, RCX, MAX_ELEMS);
        bails.push(a.jcc(Cc::A));
        // the longest path for this n: 6n + 12 (a difference at the last byte, a[k] < b[k])
        a.lea64(RAX, mx(RCX, RCX, 2));
        a.alu_rr64(Alu::Add, RAX, RAX);
        a.alu_ri64(Alu::Add, RAX, 12);
        a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, left)));
        bails.push(a.jcc(Cc::A));
        a.ld32o(RSI, o3);
        a.ld32o(RDI, o4);
        // [a, a + n) and [b, b + n) inside RAM (n = 0 reads nothing; its a - 1 test is merely conservative)
        for r in [RSI, RDI] {
            a.lea64(RDX, mxd(r, RCX, 1, -1));
            a.alu_rm64(Alu::Cmp, RDX, ctx(offset_of!(Ctx, lim1)));
            bails.push(a.jcc(Cc::A));
        }
        // host pointers; r8 = k
        a.lea64(RSI, mx(R12, RSI, 1));
        a.lea64(RDI, mx(R12, RDI, 1));
        a.alu_rr(Alu::Xor, R8, R8);
        // eight bytes at a time while eight remain and they are equal
        let q = a.pos();
        a.lea64(RAX, m(R8, 8));
        a.alu_rr64(Alu::Cmp, RAX, RCX);
        let to_bytes = a.jcc(Cc::A);
        a.ld64(RAX, mx(RSI, R8, 1));
        a.alu_rm64(Alu::Cmp, RAX, mx(RDI, R8, 1));
        let to_bytes2 = a.jcc(Cc::Ne);
        a.alu_ri64(Alu::Add, R8, 8);
        let back = a.jmp();
        a.patch(back, q);
        a.here(to_bytes);
        a.here(to_bytes2);
        // then one at a time: to the first difference, or to n
        let bl = a.pos();
        a.alu_rr64(Alu::Cmp, R8, RCX);
        let equal = a.jcc(Cc::Ae);
        a.movzx8(RAX, mx(RSI, R8, 1));
        a.movzx8(RDX, mx(RDI, R8, 1));
        a.alu_rr(Alu::Cmp, RAX, RDX);
        let differ = a.jcc(Cc::Ne);
        a.alu_ri64(Alu::Add, R8, 1);
        let back = a.jmp();
        a.patch(back, bl);

        // ---- equal: k = n (rcx) ----
        a.here(equal);
        let (o3, o4) = (self.gr(3), self.gr(4));
        let (w6, w7) = (self.gw(6), self.gw(7));
        let a = &mut self.a;
        a.ld32o(RAX, o3);
        a.lea32(RAX, mxd(RAX, RCX, 1, -1));
        a.st32o(w6, RAX);
        a.ld32o(RAX, o4);
        a.lea32(RAX, mxd(RAX, RCX, 1, -1));
        a.st32o(w7, RAX);
        // r0 = the last byte of b (n > 0)
        a.test_rr(RCX, RCX);
        let none = a.jcc(Cc::E);
        a.movzx8(RAX, mxd(RDI, RCX, 1, -1));
        let w0 = self.gw(0);
        self.a.st32o(w0, RAX);
        self.a.here(none);
        let (w3, w4) = (self.gw(3), self.gw(4));
        self.a.st32io(w3, 0);
        self.a.st32io(w4, 0);
        // retired 9 + 6n
        let a = &mut self.a;
        a.lea64(RAX, mx(RCX, RCX, 2));
        a.alu_rr64(Alu::Add, RAX, RAX);
        a.alu_ri64(Alu::Add, RAX, 9);
        a.alu_mr64(Alu::Sub, ctx(offset_of!(Ctx, left)), RAX);
        // CR0 = EQ (+ SO): the zero of addic.
        a.alu_rr(Alu::Xor, RAX, RAX);
        self.cr_set(0, Cc::L, Cc::G);
        let done = self.a.jmp();

        // ---- differ at k = r8: eax = a[k], edx = b[k] ----
        self.a.here(differ);
        let (o3, o4) = (self.gr(3), self.gr(4));
        let (w6, w7) = (self.gw(6), self.gw(7));
        let a = &mut self.a;
        a.alu_rr(Alu::Cmp, RAX, RDX); // the flags cr_set reads (lea / mov / cmov below keep them)
        a.ld32o(RCX, o3);
        a.lea32(RCX, mx(RCX, R8, 1));
        a.st32o(w6, RCX);
        a.ld32o(RCX, o4);
        a.lea32(RCX, mx(RCX, R8, 1));
        a.st32o(w7, RCX);
        let (w0, w3, w4) = (self.gw(0), self.gw(3), self.gw(4));
        let a = &mut self.a;
        a.st32o(w4, RAX);
        a.st32o(w0, RDX);
        a.mov_ri(RCX, 1);
        a.mov_ri(RSI, 0xFFFF_FFFF);
        a.cmov(Cc::B, RCX, RSI);
        a.st32o(w3, RCX);
        self.cr_set(0, Cc::B, Cc::A);
        // retired 17 + 6k - r3 (16 + 6k for 1, 18 + 6k for -1): 32-bit (k < 2^20), zero-extended
        let r3 = self.gr(3);
        let a = &mut self.a;
        a.lea64(RCX, mx(R8, R8, 2));
        a.alu_rr(Alu::Add, RCX, RCX);
        a.alu_ri(Alu::Add, RCX, 17);
        if self.env.mutant == crate::jit_mutants::MUT_XLATE_COUNT { a.mov_ri(RAX, 1) } else { a.ld32o(RAX, r3) }
        a.alu_rr(Alu::Sub, RCX, RAX);
        a.alu_mr64(Alu::Sub, ctx(offset_of!(Ctx, left)), RCX);

        self.a.here(done);
        // XER[CA] = 1; then the blr
        let x = self.rw(XER);
        self.a.alu_oi(Alu::Or, x, XER_CA);
        let lr = self.rw(LR);
        self.a.ld32o(R8, lr);
        self.chain_dyn(0);
    }

    /// [`BLENDADD`] at its head `pc`: c = CTR groups (c >= 1: the loop runs at least once; 0 would be 2^32 trips: a bail). The
    /// bytes, then r8 = a[4c - 1] + b[4c - 1] (read BEFORE any store: an aliased source still holds it), r27 = r8 & 0x100,
    /// r0 = r8 | (r27 - bit 8 of r8), r7 / r6 / r5 += 4c, CTR = 0, 36c retired; on to the word after the bdnz.
    pub(crate) fn blendadd(&mut self, pc: u32, bails: &mut Vec<Fix>) {
        let (oc, o7, o6, o5) = (self.rw(CTR), self.gr(7), self.gr(6), self.gr(5));
        let a = &mut self.a;
        a.ld32o(RCX, oc);
        a.lea32(RAX, m(RCX, -1));
        a.alu_ri(Alu::Cmp, RAX, MAX_ELEMS - 1);
        bails.push(a.jcc(Cc::A));
        a.imul_roi(RAX, O::R(RCX), 36);
        a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, left)));
        bails.push(a.jcc(Cc::A));
        // rdx = n = 4c bytes; rsi / rdi / r8 = a / b / dst (guest)
        a.mov_rr64(RDX, RCX);
        a.sh_ri64(Sh::Shl, RDX, 2);
        a.ld32o(RSI, o7);
        a.ld32o(RDI, o6);
        a.ld32o(R8, o5);
        for r in [RSI, RDI, R8] {
            a.lea64(RAX, mxd(r, RDX, 1, -1));
            a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, lim1)));
            bails.push(a.jcc(Cc::A));
        }
        // the destination may BE a source, never partly overlap one: dst != s && dst < s + n && s < dst + n bails
        if self.env.mutant != crate::jit_mutants::MUT_XLATE_OVERLAP {
            for s in [RSI, RDI] {
                let a = &mut self.a;
                a.alu_rr64(Alu::Cmp, R8, s);
                let same = a.jcc(Cc::E);
                a.lea64(RAX, mx(s, RDX, 1));
                a.alu_rr64(Alu::Cmp, R8, RAX);
                let clear1 = a.jcc(Cc::Ae);
                a.lea64(RAX, mx(R8, RDX, 1));
                a.alu_rr64(Alu::Cmp, s, RAX);
                let clear2 = a.jcc(Cc::Ae);
                bails.push(a.jmp());
                a.here(same);
                a.here(clear1);
                a.here(clear2);
            }
        }
        // every page of the destination cleared for inline stores by the store map (inside RAM, not watched code)
        let a = &mut self.a;
        a.mov_rr(RAX, R8);
        a.sh_ri(Sh::Shr, RAX, 12);
        a.lea64(RCX, mxd(R8, RDX, 1, -1));
        a.sh_ri(Sh::Shr, RCX, 12);
        let top = a.pos();
        a.cmp8_mi(mxd(R13, RAX, 1, offset_of!(Ctx, smap) as i32), 0);
        bails.push(a.jcc(Cc::E));
        a.alu_ri(Alu::Add, RAX, 1);
        a.alu_rr(Alu::Cmp, RAX, RCX);
        let again = a.jcc(Cc::Be);
        a.patch(again, top);
        // nothing can bail from here on. Lockstep: journal the destination's old bytes (h_note: a C call, so the caller-saved
        // pins are kept and the operands re-read after it)
        if self.env.logged {
            let note = self.env.note;
            let a = &mut self.a;
            // 7 pushes + 8: rsp stays 16-byte aligned for the call (store_note's 5 + 8 likewise)
            for r in crate::jit_regs::CALLER_SAVED.iter().chain(&crate::jit_regs::LOCAL_REGS) { a.push(*r); }
            a.push(RDX);
            a.push(R8);
            a.alu_ri64(Alu::Sub, RSP, 8);
            a.mov_rr64(RDI, R13);
            a.mov_rr(RSI, R8);
            a.mov_ri64(RAX, note);
            a.call_r(RAX);
            a.alu_ri64(Alu::Add, RSP, 8);
            a.pop(R8);
            a.pop(RDX);
            for r in crate::jit_regs::CALLER_SAVED.iter().chain(&crate::jit_regs::LOCAL_REGS).rev() { a.pop(*r); }
            let (o7, o6) = (self.gr(7), self.gr(6));
            self.a.ld32o(RSI, o7);
            self.a.ld32o(RDI, o6);
        }
        // host pointers
        let a = &mut self.a;
        a.lea64(RSI, mx(R12, RSI, 1));
        a.lea64(RDI, mx(R12, RDI, 1));
        a.lea64(R8, mx(R12, R8, 1));
        // the scalar registers of the last trip, from bytes read BEFORE any store (an aliased source still holds them): r8 =
        // a[n-1] + b[n-1], r27 = r8 & 0x100, r0 = r8 | (r27 - bit 8 of r8) = r8 | (r27 ? 0xFF : 0)
        a.movzx8(RCX, mxd(RSI, RDX, 1, -1));
        a.movzx8(RAX, mxd(RDI, RDX, 1, -1));
        a.alu_rr(Alu::Add, RCX, RAX);
        let (w8, w27, w0) = (self.gw(8), self.gw(27), self.gw(0));
        let a = &mut self.a;
        a.st32o(w8, RCX);
        a.mov_rr(RAX, RCX);
        a.alu_ri(Alu::And, RAX, 0x100);
        a.st32o(w27, RAX);
        a.sh_ri(Sh::Shr, RAX, 8);
        a.imul_roi(RAX, O::R(RAX), 0xFF);
        a.alu_rr(Alu::Or, RAX, RCX);
        a.st32o(w0, RAX);
        // 16 bytes at a time, then 4 (n is a multiple of 4); rax = offset
        a.alu_rr(Alu::Xor, RAX, RAX);
        let v = a.pos();
        a.lea64(RCX, m(RAX, 16));
        a.alu_rr64(Alu::Cmp, RCX, RDX);
        let fours = a.jcc(Cc::A);
        a.movdqu_ld(0, mx(RSI, RAX, 1));
        a.movdqu_ld(1, mx(RDI, RAX, 1));
        a.paddusb(0, 1);
        a.movdqu_st(mx(R8, RAX, 1), 0);
        a.alu_ri64(Alu::Add, RAX, 16);
        let back = a.jmp();
        a.patch(back, v);
        a.here(fours);
        let f = a.pos();
        a.alu_rr64(Alu::Cmp, RAX, RDX);
        let tail_done = a.jcc(Cc::Ae);
        a.sse_m(0x66, 0x6E, 0, mx(RSI, RAX, 1)); // movd xmm0,[a + i]
        a.sse_m(0x66, 0x6E, 1, mx(RDI, RAX, 1)); // movd xmm1,[b + i]
        a.paddusb(0, 1);
        a.sse_m(0x66, 0x7E, 0, mx(R8, RAX, 1)); // movd [dst + i],xmm0
        a.alu_ri64(Alu::Add, RAX, 4);
        let back = a.jmp();
        a.patch(back, f);
        a.here(tail_done);
        // the pointers move by n, CTR = 0, 36c (= 9n) retired
        let (w7, w6, w5, wc) = (self.gw(7), self.gw(6), self.gw(5), self.rw(CTR));
        for w in [w7, w6, w5] {
            self.a.ld32o(RAX, w);
            self.a.alu_rr(Alu::Add, RAX, RDX);
            self.a.st32o(w, RAX);
        }
        self.a.st32io(wc, 0);
        let a = &mut self.a;
        a.lea64(RAX, mxd(RDX, RDX, 8, 0));
        a.alu_mr64(Alu::Sub, ctx(offset_of!(Ctx, left)), RAX);
        // on past the bdnz
        let after = pc.wrapping_add(4 * BLENDADD.len() as u32);
        self.chain(after, 0);
    }
}

