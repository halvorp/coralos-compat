//! Tier 2's block compiler: one tier-1 block (t1cache boundaries, t1op decode) to x86-64.
//!
//! Register model: rbx = &Cpu (a guest register the block does not cache is a memory operand at its `offset_of!`), r12 =
//! guest RAM base, r13 = &Ctx (the remaining budget included), r15 = the instructions retired so far (across chained
//! blocks); r9 r10 r11 rbp r14 are the register cache's (jit_regs: the GPRs / CR / XER / LR / CTR a block uses most, stored
//! at every exit and helper call, reloaded after a helper). rax, rcx, rdx, rsi, rdi, r8 are scratch. Code size matters
//! (the WC3 working set is tens of thousands of blocks): every slow path is one
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
/// An exit stores every dirty cached register but the last one (jit_regs).
pub(crate) const MUT_NO_SPILL: u8 = 8;
/// The cached registers are not reloaded after a helper call (jit_regs).
pub(crate) const MUT_NO_RELOAD: u8 = 9;
/// CR0 of `add.` read from the add's own flags (its OF is not the result's sign test) instead of `test`.
pub(crate) const MUT_LAZY_FLAGS: u8 = 10;

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
    /// Inline fast paths that fell back: (jcc to patch, instruction index, the cached registers dirty at the jump); each
    /// gets a slow stub after the body.
    slow: Vec<(Fix, usize, u8)>,
    resume: Vec<usize>,
    words: Vec<u32>,
    /// The register cache (jit_regs): the Cpu fields held in host registers for this block, which of them hold a value
    /// not yet stored, and - in the counting pass - how often each field is used.
    pub(crate) regs: crate::jit_regs::Regs,
}

/// Compile `ops` (a whole tier-1 block at `pc`, decoded from code page `page` at generation `gen`): a counting pass picks
/// the Cpu fields the register cache holds, then the real pass emits. Answers the code and how many fields were pinned.
pub(crate) fn compile(ops: &[Op], pc: u32, gen: u64, page: usize, env: &mut Env) -> (Vec<u8>, usize) {
    let pins = {
        let mut e = E::new(ops, pc, env, crate::jit_regs::Regs::counting());
        e.body(ops);
        core::mem::replace(&mut e.regs, crate::jit_regs::Regs::pinned(Vec::new())).choose()
    };
    let pinned = pins.len();
    let mut e = E::new(ops, pc, env, crate::jit_regs::Regs::pinned(pins));
    let a = &mut e.a;
    // entry checks: budget, then the page generation
    a.lea64(RAX, m(R15, ops.len() as i32));
    a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, budget)));
    let b1 = a.jcc(Cc::A);
    a.ld64(RAX, ctx(offset_of!(Ctx, gen)));
    let at = m(RAX, (8 * page) as i32);
    if gen <= i32::MAX as u64 { a.alu_mi64(Alu::Cmp, at, gen as u32); } else { a.mov_ri64(RCX, gen); a.alu_rm64(Alu::Cmp, RCX, at); }
    let b2 = a.jcc(Cc::Ne);
    e.reload(true);
    e.body(ops);
    e.tails();
    let a = &mut e.a;
    a.here(b1);
    a.here(b2);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    (e.a.b, pinned)
}

impl<'a, 'b> E<'a, 'b> {
    fn new(ops: &[Op], pc: u32, env: &'a mut Env<'b>, regs: crate::jit_regs::Regs) -> Self {
        E { a: Asm::default(), env, pc, slow: Vec::new(), resume: Vec::new(), words: ops.iter().map(|o| o.w).collect(), regs }
    }

    /// The instructions, and the fall-through exit when the last one does not end the block.
    fn body(&mut self, ops: &[Op]) {
        let mut ended = false;
        for (i, op) in ops.iter().enumerate() {
            ended = self.op(i, op);
            self.resume.push(self.a.pos());
            if ended { break; }
        }
        if !ended { self.chain(self.pc.wrapping_add(4 * ops.len() as u32), ops.len()); }
    }
}

impl E<'_, '_> {
    pub(crate) fn ipc(&self, i: usize) -> u32 { self.pc.wrapping_add(4 * i as u32) }

    /// Leave for `target` (a constant), `n` instructions of this block retired.
    fn chain(&mut self, target: u32, n: usize) {
        self.spill(true);
        self.a.alu_ri64(Alu::Add, R15, n as i32);
        self.a.st32i(cpu(PC), target);
        if self.counting() {
            self.exit_normal();
        } else if self.env.chain {
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
        self.spill(true);
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
    /// leaves with the exact retired count). The register cache is stored first (the handler reads, and the stub may leave
    /// with, the Cpu) and reloaded after (the handler may have written any of it).
    pub(crate) fn helper(&mut self, i: usize) {
        self.spill(false);
        let (ipc, w) = (self.ipc(i), self.words[i]);
        let a = &mut self.a;
        a.mov_ri(RSI, w);
        a.mov_ri(RDX, ipc);
        a.mov_ri(RCX, i as u32);
        a.call_m(ctx(offset_of!(Ctx, slow)));
        if self.env.mutant == MUT_NO_RELOAD { self.regs.dirty = 0; } else { self.reload(false); }
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
        self.slow.push((f, i, self.regs.dirty));
    }

    /// The slow paths of the inline forms: store what the cache held dirty AT THE JUMP (every fast path leaves before
    /// writing guest state), the helper, reload, then back after the instruction.
    fn tails(&mut self) {
        for (f, i, dirty) in core::mem::take(&mut self.slow) {
            self.a.here(f);
            self.regs.dirty = dirty;
            self.helper(i);
            let back = self.a.jmp();
            let r = self.resume[i];
            self.a.patch(back, r);
        }
    }

    /// CR field `bf` from the flags of a compare: LT / GT by `lt` / `gt`, EQ otherwise, SO from XER. Flag-free moves and
    /// CMOVs first (the compare's flags are read once, here), then BT / ADC for SO into bit 0.
    pub(crate) fn cr_set(&mut self, bf: u8, lt: Cc, gt: Cc) {
        let no_so = self.env.mutant == MUT_NO_SO;
        let a = &mut self.a;
        a.mov_ri(RAX, 0b0010);
        a.mov_ri(RCX, 0b1000);
        a.cmov(lt, RAX, RCX);
        a.mov_ri(RCX, 0b0100);
        a.cmov(gt, RAX, RCX);
        if !no_so {
            let x = self.r(XER);
            self.a.bt_oi(x, 31);
            self.a.adc_ri(RAX, 0);
        }
        let sh = 28 - 4 * (bf as u32 & 7);
        if sh != 0 { self.a.sh_ri(Sh::Shl, RAX, sh as u8); }
        let c = self.rw(CR);
        self.a.alu_oi(Alu::And, c, !(0xF << sh));
        self.a.alu_or(Alu::Or, c, RAX);
    }
    /// CR0 from the signed value in edx.
    pub(crate) fn cr0_of_edx(&mut self) {
        self.a.test_rr(RDX, RDX);
        self.cr_set(0, Cc::L, Cc::G);
    }
    /// CR0 from the flags the instruction that produced edx left (AND / OR / XOR: SF and ZF of the result, OF = 0, so
    /// they ARE `test edx, edx`'s); only moves may come between. Anything that sets OF (add, sub) must use cr0_of_edx.
    pub(crate) fn cr0_of_logic_flags(&mut self) {
        self.cr_set(0, Cc::L, Cc::G);
    }
    /// XER[CA] = the byte in r8b (0 or 1).
    pub(crate) fn ca_from_r8(&mut self) {
        let a = &mut self.a;
        a.movzx8_rr(R8, R8);
        a.sh_ri(Sh::Shl, R8, 29);
        let x = self.rw(XER);
        self.a.alu_oi(Alu::And, x, !XER_CA);
        self.a.alu_or(Alu::Or, x, R8);
    }

    /// eax = EA & mask (esi = EA unmasked too, for the update forms); a slow path unless `n` bytes from it are in RAM.
    pub(crate) fn ea(&mut self, i: usize, e: Ea, n: u8) {
        self.ea_regs(e);
        let lim = match n { 1 => offset_of!(Ctx, lim1), 2 => offset_of!(Ctx, lim2), 4 => offset_of!(Ctx, lim4), _ => offset_of!(Ctx, lim8) };
        self.a.alu_rm64(Alu::Cmp, RAX, ctx(lim));
        self.slow_if(Cc::A, i);
    }
    fn ea_regs(&mut self, e: Ea) {
        let r = if e.update { RSI } else { RAX };
        match (e.ra, e.rb) {
            (0, Some(rb)) => { let o = self.gr(rb); self.a.ld32o(r, o) }
            (ra, Some(rb)) => {
                let (oa, ob) = (self.gr(ra), self.gr(rb));
                self.a.ld32o(r, oa);
                self.a.alu_ro(Alu::Add, r, ob);
            }
            (ra, None) => { let o = self.gr(ra); self.a.ld32o(r, o); if e.imm != 0 { self.a.alu_ri(Alu::Add, r, e.imm); } }
        }
        let a = &mut self.a;
        if e.update { a.mov_rr(RAX, RSI); }
        if self.env.mask != u32::MAX { a.alu_rm(Alu::And, RAX, ctx(offset_of!(Ctx, mask))); }
    }
    /// The store guards after `ea`: no page crossing, the page not watched (it holds decoded code), and in lockstep the
    /// old bytes journaled first.
    pub(crate) fn store_guard(&mut self, i: usize, e: Ea, n: u8) {
        if n > 1 {
            let a = &mut self.a;
            a.mov_rr(RCX, RAX);
            a.alu_ri(Alu::And, RCX, 0xFFF);
            a.alu_ri(Alu::Cmp, RCX, 0x1000 - n as u32);
            self.slow_if(Cc::A, i);
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
            self.slow_if(Cc::B, i);
        }
        if self.env.logged {
            // h_note is a C call: the cache's caller-saved registers are kept across it (rsp stays 16-byte aligned)
            let a = &mut self.a;
            for r in crate::jit_regs::CALLER_SAVED { a.push(r); }
            a.alu_ri64(Alu::Sub, RSP, 8);
            a.mov_rr64(RDI, R13);
            a.mov_rr(RSI, RAX);
            a.mov_ri(RDX, n as u32);
            a.mov_ri64(RAX, self.env.note);
            a.call_r(RAX);
            a.alu_ri64(Alu::Add, RSP, 8);
            for r in crate::jit_regs::CALLER_SAVED.iter().rev() { a.pop(*r); }
            self.ea_regs(e);
        }
    }

    /// One instruction; true when it ended the block.
    fn op(&mut self, i: usize, op: &Op) -> bool {
        let (d, s, b) = (op.d, op.a, op.b);
        match op.k {
            K::Nop => {}
            K::Li => { let o = self.gw(d); self.a.st32io(o, op.imm) }
            K::Addi | K::Ori | K::Xori => {
                let alu = match op.k { K::Addi => Alu::Add, K::Ori => Alu::Or, _ => Alu::Xor };
                self.ldg(RDX, s);
                self.a.alu_ri(alu, RDX, op.imm);
                self.stg(d, RDX);
            }
            K::AndiRc => {
                self.ldg(RDX, s);
                self.a.alu_ri(Alu::And, RDX, op.imm);
                self.stg(d, RDX);
                self.cr0_of_logic_flags();
            }
            K::Addic | K::AddicRc => {
                self.ldg(RDX, s);
                self.a.alu_ri(Alu::Add, RDX, op.imm);
                self.a.setcc(Cc::B, R8);
                self.stg(d, RDX);
                self.ca_from_r8();
                if op.k == K::AddicRc { self.cr0_of_edx(); }
            }
            K::Subfic => {
                // CA = carry of (!rA + imm + 1) = no borrow in imm - rA
                self.a.mov_ri(RDX, op.imm);
                let o = self.gr(s);
                self.a.alu_ro(Alu::Sub, RDX, o);
                self.a.setcc(if self.env.mutant == MUT_SUBFIC_CA { Cc::B } else { Cc::Ae }, R8);
                self.stg(d, RDX);
                self.ca_from_r8();
            }
            K::Mulli => { let o = self.gr(s); self.a.imul_roi(RDX, o, op.imm); self.stg(d, RDX); }
            K::Rlwinm | K::RlwinmRc => {
                self.ldg(RDX, s);
                if b & 31 != 0 { self.a.sh_ri(Sh::Rol, RDX, b & 31); }
                self.a.alu_ri(Alu::And, RDX, op.imm);
                self.stg(d, RDX);
                if op.k == K::RlwinmRc { self.cr0_of_logic_flags(); }
            }
            K::Cmpi | K::Cmpli => {
                let o = self.gr(s);
                self.a.alu_oi(Alu::Cmp, o, op.imm);
                if op.k == K::Cmpi { self.cr_set(d, Cc::L, Cc::G) } else { self.cr_set(d, Cc::B, Cc::A) }
            }
            K::Cmp | K::Cmpl => {
                self.ldg(RDX, s);
                self.alug(Alu::Cmp, RDX, b);
                if op.k == K::Cmp { self.cr_set(d, Cc::L, Cc::G) } else { self.cr_set(d, Cc::B, Cc::A) }
            }
            K::Or | K::OrRc | K::And | K::AndRc | K::Xor | K::Add | K::AddRc => {
                let alu = match op.k { K::Or | K::OrRc => Alu::Or, K::And | K::AndRc => Alu::And, K::Xor => Alu::Xor, _ => Alu::Add };
                self.ldg(RDX, s);
                self.alug(alu, RDX, b);
                self.stg(d, RDX);
                match op.k {
                    K::OrRc | K::AndRc => self.cr0_of_logic_flags(),
                    // an add's OF is not the result's sign test: only the planted mutant reads CR0 from it
                    K::AddRc if self.env.mutant == MUT_LAZY_FLAGS => self.cr0_of_logic_flags(),
                    K::AddRc => self.cr0_of_edx(),
                    _ => {}
                }
            }
            K::Andc => { self.ldg(RDX, b); self.a.not(RDX); self.alug(Alu::And, RDX, s); self.stg(d, RDX); }
            K::Nor => { self.ldg(RDX, s); self.alug(Alu::Or, RDX, b); self.a.not(RDX); self.stg(d, RDX); }
            K::Subf | K::SubfRc => {
                self.ldg(RDX, b);
                self.alug(Alu::Sub, RDX, s);
                self.stg(d, RDX);
                if op.k == K::SubfRc { self.cr0_of_edx(); }
            }
            K::Neg => { self.ldg(RDX, s); self.a.neg(RDX); self.stg(d, RDX); }
            K::Mullw => { self.ldg(RDX, s); let o = self.gr(b); self.a.imul_ro(RDX, o); self.stg(d, RDX); }
            K::Extsh => { let o = self.gr(s); self.a.movsx16o(RDX, o); self.stg(d, RDX); }
            K::Extsb => { let o = self.gr(s); self.a.movsx8o(RDX, o); self.stg(d, RDX); }
            K::Srawi | K::SrawiRc => {
                let sh = b & 31;
                self.ldg(RDX, s);
                let a = &mut self.a;
                a.mov_rr(R8, RDX);
                if sh != 0 { a.sh_ri(Sh::Sar, RDX, sh); }
                self.stg(d, RDX);
                // CA = negative and a one bit shifted out
                let a = &mut self.a;
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
                self.ldg(RCX, b);
                self.ldg(RDX, s);
                let a = &mut self.a;
                a.sh_cl(if op.k == K::Slw { Sh::Shl } else { Sh::Shr }, RDX);
                a.alu_rr(Alu::Xor, RAX, RAX);
                a.test_ri(RCX, 0x20);
                a.cmov(Cc::Ne, RDX, RAX);
                self.stg(d, RDX);
            }
            K::Mflr | K::Mfctr => { let o = self.r(if op.k == K::Mflr { LR } else { CTR }); self.a.ld32o(RDX, o); self.stg(d, RDX); }
            K::Mtlr | K::Mtctr => { self.ldg(RDX, d); let o = self.w(if op.k == K::Mtlr { LR } else { CTR }); self.a.st32o(o, RDX); }
            K::Lwz | K::Lbz | K::Lhz | K::Lha | K::Lwzu | K::Lbzu | K::Lwzx | K::Lbzx => {
                let n = match op.k { K::Lwz | K::Lwzu | K::Lwzx => 4, K::Lhz | K::Lha => 2, _ => 1 };
                self.ea(i, Ea::of(op, matches!(op.k, K::Lwzx | K::Lbzx)), n);
                self.ld_be(n, op.k == K::Lha);
                self.stg(d, RDX);
                if matches!(op.k, K::Lwzu | K::Lbzu) { self.stg(s, RSI); }
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
                if n == 8 { self.a.ld64(RDX, m(RBX, fpr(d))) } else { self.ldg(RDX, d) }
                self.st_be(n);
                if matches!(op.k, K::Stwu | K::Stbu) { self.stg(s, RSI); }
            }
            K::B => { self.chain(op.imm, i + 1); return true; }
            K::Bl => { self.set_lr(i); self.chain(op.imm, i + 1); return true; }
            K::Bc | K::Bcl => {
                if op.k == K::Bcl { self.set_lr(i); }
                let not = self.cond(d, s);
                self.chain(op.imm, i + 1);
                for f in not { self.a.here(f); }
                self.chain(self.ipc(i).wrapping_add(4), i + 1);
                return true;
            }
            K::Bclr | K::Bclrl | K::Bcctr | K::Bcctrl => {
                let from = if matches!(op.k, K::Bcctr | K::Bcctrl) { CTR } else { LR };
                let o = self.r(from);
                self.a.ld32o(R8, o);
                self.a.alu_ri(Alu::And, R8, !3);
                if matches!(op.k, K::Bclrl | K::Bcctrl) { self.set_lr(i); }
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
        if bo & 0x04 == 0 {
            let o = self.rw(CTR);
            self.a.alu_oi(Alu::Sub, o, 1);
            not.push(self.a.jcc(if bo & 0x02 != 0 { Cc::Ne } else { Cc::E }));
        }
        if bo & 0x10 == 0 {
            let o = self.r(CR);
            self.a.bt_oi(o, 31 - (bi & 31));
            not.push(self.a.jcc(if bo & 0x08 != 0 { Cc::Ae } else { Cc::B }));
        }
        not
    }
}
