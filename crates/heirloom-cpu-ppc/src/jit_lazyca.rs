//! Tier 2's LAZY XER[CA] (h86 fnreg): srawi and addic write XER[CA] as a side effect the code after them almost never reads -
//! WC3's terrain and softfloat loops run a dozen srawi a trip, each ~9 host instructions of carry logic (the census's "CA"
//! cells), all but the last one overwritten unread. A lazy writer stores its SOURCE in the Ctx (one `mov`) and leaves the
//! carry PENDING; the bit is SETTLED - computed from the stored source and the site's constant into XER - only where it can be
//! observed:
//! * before any instruction that may read or write XER other than the lazy writers (a Generic form: adde / addze / subfe /
//!   mfxer / tier 1's handler ...; `sc`) - subfic overwrites CA outright, so a pending carry is simply dropped there;
//! * before every way out of the block or node: a chained exit (`chain`), an internal region edge (`go`), a dynamic exit
//!   (`chain_dyn`, r8 kept), a hot-path helper call;
//! * on each out-of-line path, with the pending state AT THAT INSTRUCTION ([`E::ca_at`]): a superblock side exit, a slow path
//!   (tier 1's handler may trap or leave: the Cpu must be exact there).
//! A carry overwritten by the next lazy writer before any of these is never computed at all. XER[SO] (bit 31, read by
//! SO-aware compares) is never pending: only the CA bit is.
use crate::jit::Ctx;
use crate::jit_asm::*;
use crate::jit_emit::{ctx, E, XER};
use crate::t1op::{Op, K};
use core::mem::offset_of;

/// A pending XER[CA]: how to compute it from the source stored in Ctx::ca_src.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lazy {
    /// srawi: CA = the source negative and a one bit shifted out = (src & (0x80000000 | mask)) > 0x80000000, unsigned.
    Sra(u32),
    /// addic / addic.: CA = the carry out of src + imm.
    AddImm(u32),
}

impl E<'_, '_> {
    /// Lazy carries on: unless the A/B knob KNOB_NO_LAZY_CA is set (lockstep included: its entries end at exits, which
    /// settle the carry).
    /// KNOB_NO_SUPERBLOCK turns it off too (h86 native, 2026-11-05: WC3 JitLockstep with no superblocks and lazy carries
    /// diverged in XER[CA] at a region of single-block nodes - Miles' AIL_API_file_fss_attrib_write -> AIL_set_error, block
    /// 0xd5c980 - and agreed with lazy carries off or with no regions; the Z83 priced lazy carries at ~0 (nolazyca 0), so the
    /// no-superblock A/B runs without them until that interaction is found).
    /// OFF BY DEFAULT since 2026-11-05 (the coordinator's ruling: a known-wrong XER[CA] path must not run by default): only
    /// KNOB_LAZY_CA turns it on.
    pub(crate) fn ca_lazy_on(&self) -> bool {
        use heirloom_cpu_abi::ppc::{KNOB_LAZY_CA, KNOB_NO_LAZY_CA, KNOB_NO_SUPERBLOCK};
        self.env.knobs & KNOB_LAZY_CA != 0 && self.env.knobs & (KNOB_NO_LAZY_CA | KNOB_NO_SUPERBLOCK) == 0
    }

    /// Before instruction `op` (the body loop): record the pending state its out-of-line paths settle with, and settle (or
    /// drop) the pending carry if `op` may touch XER.
    pub(crate) fn ca_before(&mut self, op: &Op) {
        self.ca_at.push(self.ca_lazy);
        match op.k {
            K::Generic | K::GenericEnd | K::Sc => self.settle_ca(),
            K::Subfic => self.ca_lazy = None,
            _ => {}
        }
    }

    /// edx = the source (already loaded): store it and leave the carry `l` pending.
    pub(crate) fn ca_defer(&mut self, l: Lazy) {
        let t = self.cat(crate::jit_census::CA);
        self.a.st32(ctx(offset_of!(Ctx, ca_src)), RDX);
        self.a.set_tag(t);
        self.ca_lazy = Some(l);
    }

    /// Compute a pending carry into XER in straight-line code (it is settled from here on: rcx, r8 and the flags clobbered);
    /// nothing when none is pending.
    pub(crate) fn settle_ca(&mut self) {
        if let Some(l) = self.ca_lazy.take() { self.emit_settle(l) }
    }
    /// The same on a path that LEAVES (an exit, an internal edge, a helper call): the code emitted after it lies on other
    /// paths, where the carry is still pending - so the pending state is kept. (Keeping it after a returning helper call only
    /// computes the same bit again later.)
    pub(crate) fn settle_ca_exit(&mut self) {
        if let Some(l) = self.ca_lazy { self.emit_settle(l) }
    }
    fn emit_settle(&mut self, l: Lazy) {
        let t = self.cat(crate::jit_census::CA);
        let a = &mut self.a;
        a.ld32(RCX, ctx(offset_of!(Ctx, ca_src)));
        match l {
            Lazy::Sra(mask) => {
                a.alu_ri(Alu::And, RCX, 0x8000_0000 | mask);
                a.mov_ri(R8, 0x8000_0000);
                a.alu_rr(Alu::Cmp, R8, RCX);
            }
            Lazy::AddImm(imm) => a.alu_ri(Alu::Add, RCX, imm),
        }
        a.alu_rr(Alu::Sbb, R8, R8);
        let x = self.rw(XER);
        self.a.alu_ro(Alu::Xor, R8, x);
        self.a.alu_ri(Alu::And, R8, crate::cpu::XER_CA);
        self.a.alu_or(Alu::Xor, x, R8);
        self.a.set_tag(t);
    }

    /// settle_ca_exit with r8 (a dynamic exit's target) kept.
    pub(crate) fn settle_ca_exit_keep_r8(&mut self) {
        if self.ca_lazy.is_none() { return; }
        self.a.mov_rr(RAX, R8);
        self.settle_ca_exit();
        self.a.mov_rr(R8, RAX);
    }

    /// An out-of-line path of instruction `i`: settle what was pending before it (planted mutant 42 skips this for side exits
    /// only: `side`).
    pub(crate) fn settle_ca_at(&mut self, i: usize, side: bool) {
        self.ca_lazy = self.ca_at.get(i).copied().flatten();
        if side && self.env.mutant == crate::jit_mutants::MUT_CA_LAZY_SIDE { self.ca_lazy = None; }
        self.settle_ca();
    }
}
