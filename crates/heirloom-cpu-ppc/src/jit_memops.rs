//! Tier 2's guest memory access code (split out of jit_emit): the effective address, the RAM bounds test of a load (or a
//! load group's one test: jit_lgroup), the store map's test of a store (or a store group's), the lockstep journal call, and
//! the big-endian access itself (MOVBE where the CPU has it).
use crate::jit::Ctx;
use crate::jit_asm::*;
use crate::jit_census as tg;
use crate::jit_emit::{ctx, Ea, E, MUT_NO_WATCH};
use core::mem::offset_of;

impl E<'_, '_> {
    /// edx (rdx for 8) = the big-endian `n`-byte value at [r12 + rax], zero- or sign-extended (16 bits).
    pub(crate) fn ld_be(&mut self, n: u8, sign: bool) { self.ld_be_to(RDX, n, sign) }
    /// Host register `r` = the big-endian `n`-byte value at [r12 + rax] (a guest field's own register: no copy after).
    pub(crate) fn ld_be_to(&mut self, r: u8, n: u8, sign: bool) {
        let t = self.cat(tg::ACCESS);
        let (a, at) = (&mut self.a, mx(R12, RAX, 1));
        match (n, self.env.movbe) {
            (8, true) => a.movbe_ld(64, r, at),
            (8, false) => { a.ld64(r, at); a.bswap64(r); }
            (4, true) => a.movbe_ld(32, r, at),
            (4, false) => { a.ld32(r, at); a.bswap32(r); }
            (2, true) => { a.movbe_ld(16, r, at); if sign { a.movsx16_rr(r, r) } else { a.movzx16_rr(r, r) } }
            (2, false) => { a.movzx16(r, at); a.bswap32(r); a.sh_ri(if sign { Sh::Sar } else { Sh::Shr }, r, 16); }
            _ => a.movzx8(r, at),
        }
        a.set_tag(t);
    }
    /// Store edx (rdx for 8) big-endian as `n` bytes at [r12 + rax] (edx is clobbered).
    pub(crate) fn st_be(&mut self, n: u8) {
        let t = self.cat(tg::ACCESS);
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
        a.set_tag(t);
    }
    /// Store GPR `n` big-endian as `bytes` (1, 2 or 4) at [r12 + rax]: straight from its host register where the store needs
    /// no byte swap in a scratch register (MOVBE, or one byte), else through edx.
    pub(crate) fn st_gpr(&mut self, n: u8, bytes: u8) {
        match self.gr(n) {
            O::R(r) if bytes == 1 || self.env.movbe => {
                let t = self.cat(tg::ACCESS);
                let at = mx(R12, RAX, 1);
                if bytes == 1 { self.a.st8(at, r) } else { self.a.movbe_st(8 * bytes, at, r) }
                self.a.set_tag(t);
            }
            o => { let t = self.cat(tg::GPR_LD); self.a.ld32o(RDX, o); self.a.set_tag(t); self.st_be(bytes) }
        }
    }

    /// eax = EA & mask (esi = EA unmasked too, for the update forms); a slow path unless `n` bytes from it are in RAM.
    pub(crate) fn ea(&mut self, i: usize, e: Ea, n: u8) {
        use crate::jit_lgroup::Grp;
        match self.groups.get(i).copied() {
            // a group's later load: the head tested its bytes
            Some(Grp::Member) => return self.ea_regs(e),
            Some(Grp::Head { lo, span }) => {
                // the group's lowest EA, its whole span tested against the RAM length (64-bit: no wrap), then this load's EA
                self.ea_regs(Ea { imm: lo as u32, ..e });
                let t = self.cat(tg::GUARD_BOUNDS);
                let a = &mut self.a;
                a.lea64(RCX, m(RAX, span as i32));
                a.alu_rm64(Alu::Cmp, RCX, ctx(offset_of!(Ctx, len)));
                let f = a.jcc(Cc::A);
                self.slow_leave.push((f, i));
                self.a.set_tag(t);
                let d = e.imm.wrapping_sub(lo as u32) as i32;
                if d != 0 { let t = self.cat(tg::EA); self.a.lea32(RAX, m(RAX, d)); self.a.set_tag(t); }
                return;
            }
            _ => {}
        }
        self.ea_regs(e);
        let t = self.cat(tg::GUARD_BOUNDS);
        let lim = match n { 1 => offset_of!(Ctx, lim1), 2 => offset_of!(Ctx, lim2), 4 => offset_of!(Ctx, lim4), _ => offset_of!(Ctx, lim8) };
        self.a.alu_rm64(Alu::Cmp, RAX, ctx(lim));
        self.slow_if(Cc::A, i);
        self.a.set_tag(t);
    }
    pub(crate) fn ea_regs(&mut self, e: Ea) {
        let t = self.cat(tg::EA);
        let r = if e.update { R8 } else { RAX };
        match (e.ra, e.rb) {
            (0, Some(rb)) => { let o = self.gr(rb); self.a.ld32o(r, o) }
            (ra, Some(rb)) => match (self.gr(ra), self.gr(rb)) {
                (O::R(x), O::R(y)) => self.a.lea32(r, mx(x, y, 1)),
                (oa, ob) => { self.a.ld32o(r, oa); self.a.alu_ro(Alu::Add, r, ob); }
            },
            (ra, None) => match self.gr(ra) {
                O::R(x) => self.a.lea32(r, m(x, e.imm as i32)),
                o => { self.a.ld32o(r, o); if e.imm != 0 { self.a.alu_ri(Alu::Add, r, e.imm); } }
            },
        }
        let a = &mut self.a;
        if e.update { a.mov_rr(RAX, R8); }
        if self.env.mask != u32::MAX { a.alu_rm(Alu::And, RAX, ctx(offset_of!(Ctx, mask))); }
        a.set_tag(t);
    }
    /// The store guard after `ea_regs` (eax = the masked EA; no bounds test, no alignment or page-offset test: the store map's
    /// entry for the store's first page covers that page and the next, jit_smap), then `store_guard_n`.
    pub(crate) fn store_guard(&mut self, i: usize, e: Ea, n: u8) {
        use crate::jit_lgroup::Grp;
        let t = self.a.tag;
        match self.sgroups.get(i).copied() {
            // a store group's later store: the head's test covered its bytes (jit_lgroup)
            Some(Grp::Member) => self.store_note(n as u32),
            Some(Grp::Head { lo, .. }) => {
                // the map entry of the page holding the group's LOWEST EA covers that page and the next: the whole span
                self.cat(tg::GUARD_SMAP);
                let d = (lo as u32).wrapping_sub(e.imm) as i32;
                let a = &mut self.a;
                if d == 0 { a.mov_rr(RCX, RAX) } else { a.lea32(RCX, m(RAX, d)) }
                a.sh_ri(Sh::Shr, RCX, 12);
                a.cmp8_mi(mxd(R13, RCX, 1, offset_of!(Ctx, smap) as i32), 0);
                let f = a.jcc(Cc::E);
                self.slow_leave.push((f, i));
                self.store_note(n as u32);
            }
            _ => self.store_guard_n(i, n as u32),
        }
        if self.env.logged { self.ea_regs(e); }
        self.a.set_tag(t);
    }
    /// A store of at most 8 `bytes` at eax: the slow path unless the store map (jit_smap) says its page and the next are inside
    /// RAM and not watched (a watched page holds decoded code: tier 1's handler writes it through GuestMem, which bumps its
    /// generation); in lockstep the old bytes are journaled first (eax is then clobbered: the caller recomputes it).
    pub(crate) fn store_guard_n(&mut self, i: usize, bytes: u32) {
        let t = self.cat(tg::GUARD_SMAP);
        let a = &mut self.a;
        if self.env.mutant != MUT_NO_WATCH && self.env.knobs & heirloom_cpu_abi::ppc::KNOB_NO_SMC_STORE == 0 {
            a.mov_rr(RCX, RAX);
            a.sh_ri(Sh::Shr, RCX, 12);
            a.cmp8_mi(mxd(R13, RCX, 1, offset_of!(Ctx, smap) as i32), 0);
            self.slow_if(Cc::E, i);
        } else {
            // the planted mutant drops the watch test only: RAM bounds still hold (the map was both)
            a.lea64(RCX, m(RAX, bytes as i32));
            a.alu_rm64(Alu::Cmp, RCX, ctx(offset_of!(Ctx, len)));
            self.slow_if(Cc::A, i);
        }
        self.store_note(bytes);
        self.a.set_tag(t);
    }
    /// Lockstep only: journal the old `bytes` at eax before the store (eax is then clobbered: the caller recomputes it).
    pub(crate) fn store_note(&mut self, bytes: u32) {
        if self.env.logged {
            let t = self.a.tag;
            self.cat(tg::NOTE);
            // h_note is a C call: the cache's caller-saved registers are kept across it (rsp stays 16-byte aligned)
            let a = &mut self.a;
            for r in crate::jit_regs::CALLER_SAVED.iter().chain(&crate::jit_regs::LOCAL_REGS) { a.push(*r); }
            a.alu_ri64(Alu::Sub, RSP, 8);
            a.mov_rr64(RDI, R13);
            a.mov_rr(RSI, RAX);
            a.mov_ri(RDX, bytes);
            a.mov_ri64(RAX, self.env.note);
            a.call_r(RAX);
            a.alu_ri64(Alu::Add, RSP, 8);
            for r in crate::jit_regs::CALLER_SAVED.iter().chain(&crate::jit_regs::LOCAL_REGS).rev() { a.pop(*r); }
            self.a.set_tag(t);
        }
    }
}
