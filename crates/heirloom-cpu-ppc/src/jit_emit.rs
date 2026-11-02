//! Tier 2's block compiler: one tier-1 block (t1cache boundaries, t1op decode) to x86-64.
//!
//! Register model (increment 1): rbx = &Cpu (every guest register is a memory operand at its `offset_of!`), r12 = guest RAM
//! base, r13 = &Ctx, r14 = the instructions this entry may retire, r15 = the instructions retired so far (across chained
//! blocks), rbp = the block-relative index of the instruction the shared slow stub is running. rax, rcx, rdx, rsi, rdi, r8
//! are scratch; nothing guest-visible lives in a host register across an instruction, so a helper call or an exit needs
//! no spill. Code size matters (the WC3 working set is tens of thousands of blocks): every slow path is one
//! `call [r13 + slow]` into the shared stub (jit.rs), which calls the helper and either returns or leaves.
//!
//! Exactness (tier 1's contract): every exit leaves pc and the retired count (the dispatcher sets the time base from it); a
//! form without inline code, a load or store that would fault or touch a watched code page, and every FP arithmetic form
//! runs tier 1's own handler for that ONE instruction through `jit::h_one` with the exact pc and time base, which also
//! produces any trap - JIT code never builds a trap. A block starts with its own checks (the remaining budget covers the
//! whole block; its code page is still at the generation it was decoded at) and BAILS to the dispatcher otherwise, so a
//! chained jump to a stale or over-budget block is safe without unlinking anything.
use crate::cpu::{Cpu, XER_CA};
use crate::jit::{Ctx, Slot, EXIT_BAIL, EXIT_NORMAL};
use crate::jit_asm::*;
use crate::t1op::{Op, K};
use alloc::vec::Vec;
use core::mem::offset_of;

/// Planted mutants for the negative controls (each must make JitLockstep RED). 0 = none.
pub(crate) const MUT_SUBFIC_CA: u8 = 1;
pub(crate) const MUT_NO_SO: u8 = 2;
pub(crate) const MUT_NO_WATCH: u8 = 3;
pub(crate) const MUT_EXIT_COUNT: u8 = 4;
/// Single-precision FP arithmetic not rounded to single (jit_inline).
pub(crate) const MUT_FP_SINGLE: u8 = 5;
/// sraw's count > 31 path dropped (x86 masks the count to 5 bits: the wrong result and CA) (jit_inline).
pub(crate) const MUT_SRAW_BIG: u8 = 6;
/// adde / addze / subfe / subfze ignore XER[CA] as their carry in (jit_inline).
pub(crate) const MUT_CA_IN: u8 = 7;

/// What a block's code may refer to outside itself.
pub(crate) struct Env<'a> {
    /// `jit::h_note::<H>` (lockstep only).
    pub(crate) note: u64,
    /// The guest RAM address mask the code is compiled for (all ones: no masking instruction at all).
    pub(crate) mask: u32,
    /// The link-table slot a constant exit to `pc` jumps through.
    pub(crate) link: &'a mut dyn FnMut(u32) -> *const u64,
    /// Chain to the next block (false in lockstep: every block returns to the dispatcher).
    pub(crate) chain: bool,
    /// Lockstep: record every inline store's old bytes in the guest RAM journal first (`h_note`).
    pub(crate) logged: bool,
    pub(crate) mutant: u8,
    pub(crate) table_mask: u32,
    /// Use MOVBE for big-endian loads and stores (CPUID-dispatched: jit_asm::cpu_has_movbe).
    pub(crate) movbe: bool,
}

pub(crate) const fn gpr(n: u8) -> i32 { (offset_of!(Cpu, gpr) + 4 * (n as usize & 31)) as i32 }
pub(crate) const fn fpr(n: u8) -> i32 { (offset_of!(Cpu, fpr) + 8 * (n as usize & 31)) as i32 }
pub(crate) fn g(n: u8) -> M { m(RBX, gpr(n)) }
pub(crate) fn cpu(off: usize) -> M { m(RBX, off as i32) }
pub(crate) fn ctx(off: usize) -> M { m(R13, off as i32) }
pub(crate) const CR: usize = offset_of!(Cpu, cr);
pub(crate) const XER: usize = offset_of!(Cpu, xer);
pub(crate) const LR: usize = offset_of!(Cpu, lr);
pub(crate) const CTR: usize = offset_of!(Cpu, ctr);
pub(crate) const PC: usize = offset_of!(Cpu, pc);

/// How an access forms its EA: (rA|0) + rB, or rA + imm (rA != 0); `update` keeps the unmasked EA in esi for rA.
#[derive(Clone, Copy)]
pub(crate) struct Ea { pub(crate) ra: u8, pub(crate) rb: Option<u8>, pub(crate) imm: u32, pub(crate) update: bool }
impl Ea {
    fn of(op: &Op, indexed: bool) -> Ea {
        Ea { ra: op.a, rb: indexed.then_some(op.b), imm: op.imm, update: matches!(op.k, K::Lwzu | K::Lbzu | K::Stwu | K::Stbu) }
    }
}

pub(crate) struct E<'a, 'b> {
    pub(crate) a: Asm,
    pub(crate) env: &'a mut Env<'b>,
    pc: u32,
    /// Inline fast paths that fell back: (jcc to patch, instruction index); each gets a slow stub after the body.
    slow: Vec<(Fix, usize)>,
    resume: Vec<usize>,
    words: Vec<u32>,
}

/// Compile `ops` (a whole tier-1 block at `pc`, decoded from code page `page` at generation `gen`).
pub(crate) fn compile(ops: &[Op], pc: u32, gen: u64, page: usize, env: &mut Env) -> Vec<u8> {
    let mut e = E { a: Asm::default(), env, pc, slow: Vec::new(), resume: Vec::new(), words: ops.iter().map(|o| o.w).collect() };
    let a = &mut e.a;
    // entry checks: budget, then the page generation
    a.lea64(RAX, m(R15, ops.len() as i32));
    a.alu_rr64(Alu::Cmp, RAX, R14);
    let b1 = a.jcc(Cc::A);
    a.ld64(RAX, ctx(offset_of!(Ctx, gen)));
    let at = m(RAX, (8 * page) as i32);
    if gen <= i32::MAX as u64 { a.alu_mi64(Alu::Cmp, at, gen as u32); } else { a.mov_ri64(RCX, gen); a.alu_rm64(Alu::Cmp, RCX, at); }
    let b2 = a.jcc(Cc::Ne);
    let mut ended = false;
    for (i, op) in ops.iter().enumerate() {
        ended = e.op(i, op);
        e.resume.push(e.a.pos());
        if ended { break; }
    }
    if !ended { e.chain(pc.wrapping_add(4 * ops.len() as u32), ops.len()); }
    e.tails();
    let a = &mut e.a;
    a.here(b1);
    a.here(b2);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    e.a.b
}

impl E<'_, '_> {
    fn ipc(&self, i: usize) -> u32 { self.pc.wrapping_add(4 * i as u32) }

    /// Leave for `target` (a constant), `n` instructions of this block retired.
    fn chain(&mut self, target: u32, n: usize) {
        self.a.alu_ri64(Alu::Add, R15, n as i32);
        self.a.st32i(cpu(PC), target);
        if self.env.chain {
            let slot = (self.env.link)(target);
            self.a.mov_ri64(RAX, slot as u64);
            self.a.jmp_m(m(RAX, 0));
        } else {
            self.exit_normal();
        }
    }
    fn exit_normal(&mut self) {
        self.a.mov_ri(RAX, EXIT_NORMAL);
        self.a.jmp_m(ctx(offset_of!(Ctx, exit)));
    }
    /// Leave for the target in r8d, `n` retired: through the dispatch table when chaining.
    fn chain_dyn(&mut self, n: usize) {
        let a = &mut self.a;
        a.alu_ri64(Alu::Add, R15, n as i32);
        a.st32(cpu(PC), R8);
        if self.env.chain {
            a.mov_rr(RAX, R8);
            a.sh_ri(Sh::Shr, RAX, 2);
            a.alu_ri(Alu::And, RAX, self.env.table_mask);
            a.sh_ri(Sh::Shl, RAX, core::mem::size_of::<Slot>().trailing_zeros() as u8);
            a.alu_rm64(Alu::Add, RAX, ctx(offset_of!(Ctx, table)));
            a.alu_rm(Alu::Cmp, R8, m(RAX, offset_of!(Slot, pc) as i32));
            let miss = a.jcc(Cc::Ne);
            a.jmp_m(m(RAX, offset_of!(Slot, entry) as i32));
            a.here(miss);
        }
        self.exit_normal();
    }

    /// Run instruction `i` through tier 1's handler (the shared slow stub: it returns here when the block goes on, else
    /// leaves with the exact retired count).
    pub(crate) fn helper(&mut self, i: usize) {
        let (ipc, w) = (self.ipc(i), self.words[i]);
        let a = &mut self.a;
        a.mov_ri(RSI, w);
        a.mov_ri(RDX, ipc);
        a.mov_ri(RCX, i as u32);
        a.call_m(ctx(offset_of!(Ctx, slow)));
    }

    /// edx (rdx for 8) = the big-endian `n`-byte value at [r12 + rax], zero- or sign-extended (16 bits).
    pub(crate) fn ld_be(&mut self, n: u8, sign: bool) {
        let (a, at) = (&mut self.a, mx(R12, RAX, 1));
        match (n, self.env.movbe) {
            (8, true) => a.movbe_ld(64, RDX, at),
            (8, false) => { a.ld64(RDX, at); a.bswap64(RDX); }
            (4, true) => a.movbe_ld(32, RDX, at),
            (4, false) => { a.ld32(RDX, at); a.bswap32(RDX); }
            (2, true) => { a.movbe_ld(16, RDX, at); if sign { a.movsx16_rr(RDX, RDX) } else { a.movzx16_rr(RDX, RDX) } }
            (2, false) => { a.movzx16(RDX, at); a.bswap32(RDX); a.sh_ri(if sign { Sh::Sar } else { Sh::Shr }, RDX, 16); }
            _ => a.movzx8(RDX, at),
        }
    }
    /// Store edx (rdx for 8) big-endian as `n` bytes at [r12 + rax] (edx is clobbered).
    pub(crate) fn st_be(&mut self, n: u8) {
        let (a, at) = (&mut self.a, mx(R12, RAX, 1));
        match (n, self.env.movbe) {
            (8, true) => a.movbe_st(64, at, RDX),
            (8, false) => { a.bswap64(RDX); a.st64(at, RDX); }
            (4, true) => a.movbe_st(32, at, RDX),
            (4, false) => { a.bswap32(RDX); a.st32(at, RDX); }
            (2, true) => a.movbe_st(16, at, RDX),
            (2, false) => { a.bswap32(RDX); a.sh_ri(Sh::Shr, RDX, 16); a.st16(at, RDX); }
            _ => a.st8(at, RDX),
        }
    }

    /// A conditional jump to instruction `i`'s slow path (tier 1's handler for it, then back after it).
    pub(crate) fn slow_if(&mut self, cc: Cc, i: usize) {
        let f = self.a.jcc(cc);
        self.slow.push((f, i));
    }

    /// The slow paths of the inline loads and stores: the helper, then back to the instruction after.
    fn tails(&mut self) {
        for (f, i) in core::mem::take(&mut self.slow) {
            self.a.here(f);
            self.helper(i);
            let back = self.a.jmp();
            let r = self.resume[i];
            self.a.patch(back, r);
        }
    }

    /// CR field `bf` from the flags of a compare: LT / GT by `lt` / `gt`, EQ by ZF, SO from XER.
    pub(crate) fn cr_set(&mut self, bf: u8, lt: Cc, gt: Cc) {
        let no_so = self.env.mutant == MUT_NO_SO;
        let a = &mut self.a;
        a.setcc(lt, RAX);
        a.setcc(gt, RCX);
        a.setcc(Cc::E, RDX);
        a.movzx8_rr(RAX, RAX);
        a.movzx8_rr(RCX, RCX);
        a.movzx8_rr(RDX, RDX);
        a.sh_ri(Sh::Shl, RAX, 3);
        a.sh_ri(Sh::Shl, RCX, 2);
        a.alu_rr(Alu::Add, RDX, RDX);
        a.alu_rr(Alu::Or, RAX, RCX);
        a.alu_rr(Alu::Or, RAX, RDX);
        if !no_so {
            a.ld32(RCX, cpu(XER));
            a.sh_ri(Sh::Shr, RCX, 31);
            a.alu_rr(Alu::Or, RAX, RCX);
        }
        let sh = 28 - 4 * (bf as u32 & 7);
        if sh != 0 { a.sh_ri(Sh::Shl, RAX, sh as u8); }
        a.ld32(RCX, cpu(CR));
        a.alu_ri(Alu::And, RCX, !(0xF << sh));
        a.alu_rr(Alu::Or, RCX, RAX);
        a.st32(cpu(CR), RCX);
    }
    /// CR0 from the signed value in edx.
    pub(crate) fn cr0_of_edx(&mut self) {
        self.a.test_rr(RDX, RDX);
        self.cr_set(0, Cc::L, Cc::G);
    }
    /// XER[CA] = the byte in r8b (0 or 1).
    pub(crate) fn ca_from_r8(&mut self) {
        let a = &mut self.a;
        a.movzx8_rr(R8, R8);
        a.sh_ri(Sh::Shl, R8, 29);
        a.ld32(RCX, cpu(XER));
        a.alu_ri(Alu::And, RCX, !XER_CA);
        a.alu_rr(Alu::Or, RCX, R8);
        a.st32(cpu(XER), RCX);
    }

    /// eax = EA & mask (esi = EA unmasked too, for the update forms); a slow path unless `n` bytes from it are in RAM.
    pub(crate) fn ea(&mut self, i: usize, e: Ea, n: u8) {
        self.ea_regs(e);
        let lim = match n { 1 => offset_of!(Ctx, lim1), 2 => offset_of!(Ctx, lim2), 4 => offset_of!(Ctx, lim4), _ => offset_of!(Ctx, lim8) };
        self.a.alu_rm64(Alu::Cmp, RAX, ctx(lim));
        let f = self.a.jcc(Cc::A);
        self.slow.push((f, i));
    }
    fn ea_regs(&mut self, e: Ea) {
        let r = if e.update { RSI } else { RAX };
        let a = &mut self.a;
        match (e.ra, e.rb) {
            (0, Some(rb)) => a.ld32(r, g(rb)),
            (ra, Some(rb)) => { a.ld32(r, g(ra)); a.alu_rm(Alu::Add, r, g(rb)); }
            (ra, None) => { a.ld32(r, g(ra)); if e.imm != 0 { a.alu_ri(Alu::Add, r, e.imm); } }
        }
        if e.update { a.mov_rr(RAX, RSI); }
        if self.env.mask != u32::MAX { a.alu_rm(Alu::And, RAX, ctx(offset_of!(Ctx, mask))); }
    }
    /// The store guards after `ea`: no page crossing, the page not watched (it holds decoded code), and in lockstep the
    /// old bytes journaled first.
    pub(crate) fn store_guard(&mut self, i: usize, e: Ea, n: u8) {
        let a = &mut self.a;
        if n > 1 {
            a.mov_rr(RCX, RAX);
            a.alu_ri(Alu::And, RCX, 0xFFF);
            a.alu_ri(Alu::Cmp, RCX, 0x1000 - n as u32);
            let f = a.jcc(Cc::A);
            self.slow.push((f, i));
        }
        if self.env.mutant != MUT_NO_WATCH {
            let a = &mut self.a;
            a.mov_rr(RCX, RAX);
            a.sh_ri(Sh::Shr, RCX, 12);
            a.mov_rr(RDX, RCX);
            a.sh_ri(Sh::Shr, RDX, 6);
            a.ld64(RDI, ctx(offset_of!(Ctx, marked)));
            a.ld64(RDX, mx(RDI, RDX, 8));
            a.bt_rr64(RDX, RCX);
            let f = a.jcc(Cc::B);
            self.slow.push((f, i));
        }
        if self.env.logged {
            let a = &mut self.a;
            a.mov_rr64(RDI, R13);
            a.mov_rr(RSI, RAX);
            a.mov_ri(RDX, n as u32);
            a.mov_ri64(RAX, self.env.note);
            a.call_r(RAX);
            self.ea_regs(e);
        }
    }

    /// One instruction; true when it ended the block.
    fn op(&mut self, i: usize, op: &Op) -> bool {
        let (d, s, b) = (op.d, op.a, op.b);
        match op.k {
            K::Nop => {}
            K::Li => self.a.st32i(g(d), op.imm),
            K::Addi | K::Ori | K::Xori => {
                let alu = match op.k { K::Addi => Alu::Add, K::Ori => Alu::Or, _ => Alu::Xor };
                self.a.ld32(RDX, g(s));
                self.a.alu_ri(alu, RDX, op.imm);
                self.a.st32(g(d), RDX);
            }
            K::AndiRc => {
                self.a.ld32(RDX, g(s));
                self.a.alu_ri(Alu::And, RDX, op.imm);
                self.a.st32(g(d), RDX);
                self.cr0_of_edx();
            }
            K::Addic | K::AddicRc => {
                self.a.ld32(RDX, g(s));
                self.a.alu_ri(Alu::Add, RDX, op.imm);
                self.a.setcc(Cc::B, R8);
                self.a.st32(g(d), RDX);
                self.ca_from_r8();
                if op.k == K::AddicRc { self.cr0_of_edx(); }
            }
            K::Subfic => {
                // CA = carry of (!rA + imm + 1) = no borrow in imm - rA
                self.a.mov_ri(RDX, op.imm);
                self.a.alu_rm(Alu::Sub, RDX, g(s));
                self.a.setcc(if self.env.mutant == MUT_SUBFIC_CA { Cc::B } else { Cc::Ae }, R8);
                self.a.st32(g(d), RDX);
                self.ca_from_r8();
            }
            K::Mulli => { self.a.imul_rmi(RDX, g(s), op.imm); self.a.st32(g(d), RDX); }
            K::Rlwinm | K::RlwinmRc => {
                self.a.ld32(RDX, g(s));
                if b & 31 != 0 { self.a.sh_ri(Sh::Rol, RDX, b & 31); }
                self.a.alu_ri(Alu::And, RDX, op.imm);
                self.a.st32(g(d), RDX);
                if op.k == K::RlwinmRc { self.cr0_of_edx(); }
            }
            K::Cmpi => { self.a.alu_mi(Alu::Cmp, g(s), op.imm); self.cr_set(d, Cc::L, Cc::G); }
            K::Cmpli => { self.a.alu_mi(Alu::Cmp, g(s), op.imm); self.cr_set(d, Cc::B, Cc::A); }
            K::Cmp | K::Cmpl => {
                self.a.ld32(RDX, g(s));
                self.a.alu_rm(Alu::Cmp, RDX, g(b));
                if op.k == K::Cmp { self.cr_set(d, Cc::L, Cc::G) } else { self.cr_set(d, Cc::B, Cc::A) }
            }
            K::Or | K::OrRc | K::And | K::AndRc | K::Xor | K::Add | K::AddRc => {
                let alu = match op.k { K::Or | K::OrRc => Alu::Or, K::And | K::AndRc => Alu::And, K::Xor => Alu::Xor, _ => Alu::Add };
                self.a.ld32(RDX, g(s));
                self.a.alu_rm(alu, RDX, g(b));
                self.a.st32(g(d), RDX);
                if matches!(op.k, K::OrRc | K::AndRc | K::AddRc) { self.cr0_of_edx(); }
            }
            K::Andc => { self.a.ld32(RDX, g(b)); self.a.not(RDX); self.a.alu_rm(Alu::And, RDX, g(s)); self.a.st32(g(d), RDX); }
            K::Nor => { self.a.ld32(RDX, g(s)); self.a.alu_rm(Alu::Or, RDX, g(b)); self.a.not(RDX); self.a.st32(g(d), RDX); }
            K::Subf | K::SubfRc => {
                self.a.ld32(RDX, g(b));
                self.a.alu_rm(Alu::Sub, RDX, g(s));
                self.a.st32(g(d), RDX);
                if op.k == K::SubfRc { self.cr0_of_edx(); }
            }
            K::Neg => { self.a.ld32(RDX, g(s)); self.a.neg(RDX); self.a.st32(g(d), RDX); }
            K::Mullw => { self.a.ld32(RDX, g(s)); self.a.imul_rm(RDX, g(b)); self.a.st32(g(d), RDX); }
            K::Extsh => { self.a.movsx16(RDX, g(s)); self.a.st32(g(d), RDX); }
            K::Extsb => { self.a.movsx8(RDX, g(s)); self.a.st32(g(d), RDX); }
            K::Srawi | K::SrawiRc => {
                let sh = b & 31;
                let a = &mut self.a;
                a.ld32(RDX, g(s));
                a.mov_rr(R8, RDX);
                if sh != 0 { a.sh_ri(Sh::Sar, RDX, sh); }
                a.st32(g(d), RDX);
                // CA = negative and a one bit shifted out
                a.test_ri(R8, if sh == 0 { 0 } else { (1u32 << sh) - 1 });
                a.setcc(Cc::Ne, RAX);
                a.test_rr(R8, R8);
                a.setcc(Cc::S, RCX);
                a.and8_rr(RAX, RCX);
                a.mov_rr(R8, RAX);
                self.ca_from_r8();
                if op.k == K::SrawiRc { self.cr0_of_edx(); }
            }
            K::Slw | K::Srw => {
                let a = &mut self.a;
                a.ld32(RCX, g(b));
                a.ld32(RDX, g(s));
                a.sh_cl(if op.k == K::Slw { Sh::Shl } else { Sh::Shr }, RDX);
                a.alu_rr(Alu::Xor, RAX, RAX);
                a.test_ri(RCX, 0x20);
                a.cmov(Cc::Ne, RDX, RAX);
                a.st32(g(d), RDX);
            }
            K::Mflr => { self.a.ld32(RDX, cpu(LR)); self.a.st32(g(d), RDX); }
            K::Mfctr => { self.a.ld32(RDX, cpu(CTR)); self.a.st32(g(d), RDX); }
            K::Mtlr => { self.a.ld32(RDX, g(d)); self.a.st32(cpu(LR), RDX); }
            K::Mtctr => { self.a.ld32(RDX, g(d)); self.a.st32(cpu(CTR), RDX); }
            K::Lwz | K::Lbz | K::Lhz | K::Lha | K::Lwzu | K::Lbzu | K::Lwzx | K::Lbzx => {
                let n = match op.k { K::Lwz | K::Lwzu | K::Lwzx => 4, K::Lhz | K::Lha => 2, _ => 1 };
                self.ea(i, Ea::of(op, matches!(op.k, K::Lwzx | K::Lbzx)), n);
                self.ld_be(n, op.k == K::Lha);
                let a = &mut self.a;
                a.st32(g(d), RDX);
                if matches!(op.k, K::Lwzu | K::Lbzu) { a.st32(g(s), RSI); }
            }
            K::Lfd => {
                self.ea(i, Ea::of(op, false), 8);
                self.ld_be(8, false);
                self.a.st64(m(RBX, fpr(d)), RDX);
            }
            K::Stw | K::Stb | K::Sth | K::Stwu | K::Stbu | K::Stwx | K::Stbx | K::Stfd => {
                let n = match op.k { K::Stw | K::Stwu | K::Stwx => 4, K::Sth => 2, K::Stfd => 8, _ => 1 };
                let indexed = matches!(op.k, K::Stwx | K::Stbx);
                self.ea(i, Ea::of(op, indexed), n);
                self.store_guard(i, Ea::of(op, indexed), n);
                if n == 8 { self.a.ld64(RDX, m(RBX, fpr(d))) } else { self.a.ld32(RDX, g(d)) }
                self.st_be(n);
                let a = &mut self.a;
                if matches!(op.k, K::Stwu | K::Stbu) { a.st32(g(s), RSI); }
            }
            K::B => { self.chain(op.imm, i + 1); return true; }
            K::Bl => { self.a.st32i(cpu(LR), self.ipc(i).wrapping_add(4)); self.chain(op.imm, i + 1); return true; }
            K::Bc | K::Bcl => {
                if op.k == K::Bcl { self.a.st32i(cpu(LR), self.ipc(i).wrapping_add(4)); }
                let not = self.cond(d, s);
                self.chain(op.imm, i + 1);
                for f in not { self.a.here(f); }
                self.chain(self.ipc(i).wrapping_add(4), i + 1);
                return true;
            }
            K::Bclr | K::Bclrl | K::Bcctr | K::Bcctrl => {
                let from = if matches!(op.k, K::Bcctr | K::Bcctrl) { CTR } else { LR };
                self.a.ld32(R8, cpu(from));
                self.a.alu_ri(Alu::And, R8, !3);
                if matches!(op.k, K::Bclrl | K::Bcctrl) { self.a.st32i(cpu(LR), self.ipc(i).wrapping_add(4)); }
                let not = self.cond(d, s);
                self.chain_dyn(i + 1);
                for f in not { self.a.here(f); }
                self.chain(self.ipc(i).wrapping_add(4), i + 1);
                return true;
            }
            K::Generic | K::GenericEnd | K::Sc | K::Fp59 | K::Fp63 | K::Lfs | K::Stfs => {
                if self.inline(i, op) { return false; }
                self.helper(i);
                if op.terminal() {
                    // a block-ending Generic always leaves through its helper (it is a branch: answer 1)
                    self.chain_dyn_after_helper(i);
                    return true;
                }
            }
        }
        false
    }

    /// After a terminal Generic answered 0 (it never does: a branch answers 1 with pc set), leave with pc as it is.
    fn chain_dyn_after_helper(&mut self, i: usize) {
        self.a.alu_ri64(Alu::Add, R15, i as i32 + 1);
        self.exit_normal();
    }

    /// The BO / BI tests of a conditional branch (CTR decremented first unless BO says not); the returned jumps go to
    /// the NOT-taken path.
    fn cond(&mut self, bo: u8, bi: u8) -> Vec<Fix> {
        let mut not = Vec::new();
        let a = &mut self.a;
        if bo & 0x04 == 0 {
            a.alu_mi(Alu::Sub, cpu(CTR), 1);
            not.push(a.jcc(if bo & 0x02 != 0 { Cc::Ne } else { Cc::E }));
        }
        if bo & 0x10 == 0 {
            a.bt_mi(cpu(CR), 31 - (bi & 31));
            not.push(a.jcc(if bo & 0x08 != 0 { Cc::Ae } else { Cc::B }));
        }
        not
    }
}
