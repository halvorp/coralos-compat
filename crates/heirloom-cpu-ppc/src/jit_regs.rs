//! Tier 2's register model: the GLOBAL pins. Five guest fields (32-bit Cpu fields at `rbx + offset`) live in host registers
//! across ALL JIT code - every block and every chained jump between blocks - so no block loads them at its entry or stores
//! them at its exits; every other guest field is a memory operand at `rbx + offset`.
//!
//! WHICH FIVE, by measurement (WC3 gameplay, h86 2026-11-03, the block-boundary loads + stores of each field a per-block cache
//! paid): CR 13.2%, r3 8.7%, XER 7.6%, r0 6.8%, r4 6.8% of the boundary traffic (then LR 5.2%, r1 4.5%; r2 2.0%, r12 1.8%).
//! Pinning them globally beat the per-block cache this module was before (a counting pass per block choosing up to five fields,
//! loaded at entry, stored at exits), and three globals + two per-block registers, and four + one: JIT host instructions per
//! native call 4,024 (per-block) -> 3,946 (3 + 2) -> 3,918 (4 + 1) -> 3,874 (5 + 0); and with no per-block registers left the
//! counting pass (half of every compile) is gone.
//!
//! h86 jitsize: XER gave its register to r1. Block-boundary traffic stopped mattering once the pins were global; what a pin
//! saves now is the instructions of each USE (a memory operand loaded into scratch, `mov` + `add` for an EA where `lea` does).
//! Executed GPR references in JIT code (WC3 gameplay, callgrind window, per guest instruction): r0 ~0.33 (overcounted: RA=0),
//! r3 0.16, r1 0.13 (0.10 as a load / store base: the stack), r4 0.13, r6 0.09, r5 0.09, r8 0.08, r7 0.07, r31 0.05; XER's
//! uses (CA in / out, SO under SO-aware code: jit.rs `so_aware`) are read-modify-write forms that take a memory operand at
//! the same instruction count.
//!
//! EXACTNESS: the Cpu holds a global's value wherever Rust can see it. The trampoline loads them; the common exit stores them
//! (every way out of JIT code passes there: chained-exit misses, bails, traps, the slow stub's leaving paths); a call into
//! Rust - the slow stub's helper (tier 1's handler for one instruction), a native block's h_native - is preceded by a store
//! and followed by a load (the handler may write any field). The lockstep journal call `h_note` reads no Cpu field: the
//! caller-saved globals are kept on the stack around it. In JitLockstep every block is entered through the trampoline, so the
//! globals are flushed at every block boundary there.
//!
//! Host registers: r9 r10 r11 (caller-saved) and rbp, r14, r15 (callee-saved; the trampoline saves them). h86 jitsize increment
//! 5: r15 held the budget's count; that count is in the Ctx now (`left`, read-modify-write at the same instruction count) and
//! r15 holds r6 (0.09 executed references per guest insn, the next GPR after r0 r3 r1 r4). Nothing else in tier 2
//! uses them (scratch is rax rcx rdx r8; rsi and rdi hold a region's locals, below, and call arguments).
//!
//! REGION LOCALS (increment 2 of the region tier): inside a region two more GPRs - the two the region's code accesses most,
//! counted by a dry run of the same emitter - live in rsi and rdi. They are loaded after the region's entry checks, kept
//! across its internal edges, stored at every exit (each one the region writes anywhere: an exit cannot know the path), and
//! around every helper call (stored before, reloaded after: the handler may write any field).
use crate::jit_asm::*;
use crate::jit_emit::{gpr, CR, E, LR};

/// The global pins: (Cpu field offset, host register).
pub(crate) const GLOBALS: [(usize, u8); 6] = [(CR, R14), (gpr(3) as usize, RBP), (gpr(1) as usize, R11), (gpr(0) as usize, R10), (gpr(4) as usize, R9), (gpr(6) as usize, R15)];
/// The caller-saved ones (pushed around the lockstep journal call).
pub(crate) const CALLER_SAVED: [u8; 3] = [R9, R10, R11];

/// The global pin of field `off`, if it is one.
#[inline(always)]
fn global(off: usize) -> Option<u8> { GLOBALS.iter().find(|g| g.0 == off).map(|g| g.1) }
/// The host registers a region's locals live in.
pub(crate) const LOCAL_REGS: [u8; 2] = [RSI, RDI];
/// The host XMM registers a region's FPR locals live in (xmm8..xmm15: unused by every scalar scratch / stub path).
pub(crate) const FP_LOCAL_REGS: [u8; 8] = [8, 9, 10, 11, 12, 13, 14, 15];
/// A GPR becomes a region local when the region's code accesses it at least this often (a load at entry and a store per
/// exit must pay off).
const LOCAL_MIN_USES: u32 = 4;
/// An FPR becomes a region FP local when accessed at least this often in the region (zero GPR pressure).
const FP_LOCAL_MIN_USES: u32 = 2;

/// Pick up to 8 most-used FPRs in a region for `xmm8..xmm15`: `(fpr_index, xmm_reg, written)`.
pub(crate) fn pick_fp_locals(uses: &[u32; 32], writes: &[u32; 32]) -> alloc::vec::Vec<(usize, u8, bool)> {
    let mut v: alloc::vec::Vec<(u32, usize)> = (0..32usize)
        .filter(|&n| uses[n] >= FP_LOCAL_MIN_USES)
        .map(|n| (uses[n], n))
        .collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.into_iter()
        .zip(FP_LOCAL_REGS)
        .map(|((_, n), x)| (n, x, writes[n] != 0))
        .collect()
}

/// A GPR local must be used this many times MORE than the global pin whose host register it takes (h86 fnreg: the pin's value
/// is stored at the region's entry and loaded back at every exit; a local in a free register pays only its own load / store).
const DISPLACE_MARGIN: u32 = 4;

/// The region's REGISTER BINDING from a dry run's GPR access counts (h86 fnreg, the function-region step "the pinned host
/// registers re-bound to the region's hottest GPRs"): the most-used GPRs that are not global pins become locals, first in rsi
/// and rdi, then - with `displace` - in the host registers of the GPR pins the region uses least (r1 r0 r3 r4 r6: CR keeps
/// r14), each only while the local is used [`DISPLACE_MARGIN`] times more than the pin it displaces. Answers the locals
/// (Cpu offset, host register, written in the region) and the DISPLACED pins (Cpu offset, host register): inside the region a
/// displaced pin is a memory operand; its register is stored at the entry and loaded back at every exit.
pub(crate) fn pick_locals(uses: &[u32; 32], writes: &[u32; 32], displace: bool) -> (alloc::vec::Vec<(usize, u8, bool)>, alloc::vec::Vec<(usize, u8)>) {
    let n_of = |off: usize| (off - gpr(0) as usize) / 4;
    let mut v: alloc::vec::Vec<(u32, u8)> = (0..32u8).filter(|&n| global(gpr(n) as usize).is_none() && uses[n as usize] >= LOCAL_MIN_USES).map(|n| (uses[n as usize], n)).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    // the host registers on offer, cheapest first: the two free ones, then the GPR pins by their uses here, least first
    let mut slots: alloc::vec::Vec<(u8, Option<(usize, u32)>)> = LOCAL_REGS.iter().map(|&r| (r, None)).collect();
    if displace {
        let mut pins: alloc::vec::Vec<(u32, usize, u8)> = GLOBALS.iter().filter(|g| g.0 != CR).map(|&(off, h)| (uses[n_of(off)], off, h)).collect();
        pins.sort_unstable();
        slots.extend(pins.into_iter().map(|(u, off, h)| (h, Some((off, u)))));
    }
    let (mut locals, mut displaced) = (alloc::vec::Vec::new(), alloc::vec::Vec::new());
    for (&(u, n), &(h, pin)) in v.iter().zip(&slots) {
        if let Some((off, pu)) = pin {
            if u < pu + DISPLACE_MARGIN { break; }
            displaced.push((off, h));
        }
        locals.push((gpr(n) as usize, h, writes[n as usize] != 0));
    }
    (locals, displaced)
}
/// Store every global pin into the Cpu (before Rust can read it).
pub(crate) fn store_globals(a: &mut Asm) { for (off, h) in GLOBALS { a.st32(crate::jit_emit::cpu_m(off as i32), h); } }
/// Store every global pin but the `displaced` ones (a region's helper call: a displaced pin's Cpu field is current, its
/// register holds a local).
pub(crate) fn store_globals_except(a: &mut Asm, displaced: &[(usize, u8)]) {
    for (off, h) in GLOBALS { if !displaced.iter().any(|d| d.0 == off) { a.st32(crate::jit_emit::cpu_m(off as i32), h); } }
}
/// Load every global pin from the Cpu (after Rust may have written it).
pub(crate) fn load_globals(a: &mut Asm) { for (off, h) in GLOBALS { a.ld32(h, crate::jit_emit::cpu_m(off as i32)); } }

impl E<'_, '_> {
    /// The Cpu field at `off` (a 32-bit one): its host register when it is a global pin, else the memory operand.
    #[inline(always)]
    fn field(&self, off: usize) -> O {
        if let Some(l) = self.locals.iter().find(|l| l.0 == off) { return O::R(l.1) }
        match global(off) {
            Some(h) if !self.displaced.iter().any(|d| d.0 == off) => O::R(h),
            _ => O::M(crate::jit_emit::cpu_m(off as i32)),
        }
    }
    /// A region's entry, after its checks: the displaced pins' registers into the Cpu, then every local into its register.
    pub(crate) fn bind_entry(&mut self) {
        let t = self.cat(crate::jit_census::LOCALS_LOAD);
        for &(off, h) in &self.displaced { self.a.st32(crate::jit_emit::cpu_m(off as i32), h); }
        self.a.set_tag(t);
        self.load_locals();
    }
    /// Every way out of a region (after store_locals): the displaced pins back into their registers, so the code it goes to
    /// finds every global pin where it always is.
    pub(crate) fn restore_displaced(&mut self) {
        if self.env.mutant == crate::jit_mutants::MUT_REGION_DISPLACED_EXIT { return; }
        let t = self.cat(crate::jit_census::LOCALS_STORE);
        for &(off, h) in &self.displaced { self.a.ld32(h, crate::jit_emit::cpu_m(off as i32)); }
        self.a.set_tag(t);
    }
    /// Load every region local (GPR + FPR) from the Cpu (region entry; after a helper call).
    pub(crate) fn load_locals(&mut self) {
        let t = self.cat(crate::jit_census::LOCALS_LOAD);
        for &(off, h, _) in &self.locals { self.a.ld32(h, crate::jit_emit::cpu_m(off as i32)); }
        for &(n, x, _) in &self.fp_locals { self.a.sse_m(0xF2, 0x10, x, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))); }
        self.a.set_tag(t);
    }
    /// Store every region local (GPR + FPR) the region writes into the Cpu (every exit; before a helper call).
    pub(crate) fn store_locals(&mut self) {
        if self.env.mutant == crate::jit_emit::MUT_REGION_LOCAL_EXIT { return; }
        let t = self.cat(crate::jit_census::LOCALS_STORE);
        for &(off, h, w) in &self.locals { if w { self.a.st32(crate::jit_emit::cpu_m(off as i32), h); } }
        if self.env.mutant != crate::jit_mutants::MUT_FP_LOCAL_EXIT {
            for &(n, x, w) in &self.fp_locals { if w { self.a.sse_m(0xF2, 0x11, x, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))); } }
        }
        self.a.set_tag(t);
    }
    /// Save / reload region FPR locals only (around lockstep `h_note` which is a C call that may clobber caller-saved XMMs).
    pub(crate) fn store_fp_locals(&mut self) {
        for &(n, x, w) in &self.fp_locals { if w { self.a.sse_m(0xF2, 0x11, x, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))); } }
    }
    pub(crate) fn load_fp_locals(&mut self) {
        for &(n, x, _) in &self.fp_locals { self.a.sse_m(0xF2, 0x10, x, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))); }
    }
    /// Record an FPR access and return its resident XMM register (`xmm8..xmm15`) when cached in this region.
    pub(crate) fn fp_reg(&mut self, n: usize, write: bool) -> Option<u8> {
        let i = n & 31;
        self.fpr_uses[i] += 1;
        if write { self.fpr_writes[i] += 1; }
        self.fp_locals.iter().find(|l| l.0 == i).map(|l| l.1)
    }
    /// Peek whether FPR `n` is resident in an XMM register without changing dry-run counts.
    pub(crate) fn fp_cached(&self, n: usize) -> Option<u8> {
        self.fp_locals.iter().find(|l| l.0 == (n & 31)).map(|l| l.1)
    }
    /// Load FPR `n` into host XMM register `dst_xmm`.
    pub(crate) fn fp_load(&mut self, dst_xmm: u8, n: usize) {
        match self.fp_reg(n, false) {
            Some(x) => if x != dst_xmm { self.a.movapd(dst_xmm, x); },
            None => self.a.sse_m(0xF2, 0x10, dst_xmm, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))),
        }
    }
    /// Store host XMM register `src_xmm` into FPR `n`.
    pub(crate) fn fp_store(&mut self, n: usize, src_xmm: u8) {
        match self.fp_reg(n, true) {
            Some(x) => if x != src_xmm { self.a.movapd(x, src_xmm); },
            None => self.a.sse_m(0xF2, 0x11, src_xmm, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))),
        }
    }
    /// Emit `pre 0F opc dst_xmm, fpr(n)` directly register-to-register when cached, else from memory.
    pub(crate) fn fp_op(&mut self, pre: u8, opc: u8, dst_xmm: u8, n: usize) {
        match self.fp_reg(n, false) {
            Some(x) => self.a.sse_r(pre, opc, dst_xmm, x),
            None => self.a.sse_m(pre, opc, dst_xmm, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))),
        }
    }
    /// Load FPR `n`'s raw 64 bits into host GPR `dst_gpr`.
    pub(crate) fn fp_load_gpr(&mut self, dst_gpr: u8, n: usize) {
        match self.fp_reg(n, false) {
            Some(x) => self.a.movq_rx(dst_gpr, x),
            None => self.a.ld64(dst_gpr, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))),
        }
    }
    /// Store host GPR `src_gpr`'s raw 64 bits into FPR `n`.
    pub(crate) fn fp_store_gpr(&mut self, n: usize, src_gpr: u8) {
        match self.fp_reg(n, true) {
            Some(x) => self.a.movq_xr(x, src_gpr),
            None => self.a.st64(crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8)), src_gpr),
        }
    }
    /// Compare 64-bit `RAX` against FPR `n`'s raw 64 bits (clobbers `RCX` only when `n` is in XMM).
    pub(crate) fn fp_cmp_rax(&mut self, n: usize) {
        match self.fp_cached(n) {
            Some(x) => { self.a.movq_rx(RCX, x); self.a.alu_rr64(Alu::Cmp, RAX, RCX); }
            None => self.a.alu_rm64(Alu::Cmp, RAX, crate::jit_emit::cpu_m(crate::jit_emit::fpr(n as u8))),
        }
    }
    /// The Cpu field at `off`, to read / to write / to read and write (the same operand: named for the reader).
    pub(crate) fn r(&mut self, off: usize) -> O { self.field(off) }
    pub(crate) fn w(&mut self, off: usize) -> O { self.field(off) }
    pub(crate) fn rw(&mut self, off: usize) -> O { self.field(off) }
    /// GPR n, to read / to write.
    pub(crate) fn gr(&mut self, n: u8) -> O { self.uses[n as usize & 31] += 1; self.r(gpr(n) as usize) }
    pub(crate) fn gw(&mut self, n: u8) -> O { self.uses[n as usize & 31] += 1; self.writes[n as usize & 31] += 1; self.w(gpr(n) as usize) }
    /// host `d` = GPR n; GPR n = host `s`; host `d` op= GPR n.
    pub(crate) fn ldg(&mut self, d: u8, n: u8) {
        let o = self.gr(n);
        let t = self.cat(if matches!(o, O::M(_)) { crate::jit_census::GPR_LD } else { crate::jit_census::GPR_MOV });
        self.a.ld32o(d, o);
        self.a.set_tag(t);
    }
    pub(crate) fn stg(&mut self, n: u8, s: u8) {
        let o = self.gw(n);
        let t = self.cat(if matches!(o, O::M(_)) { crate::jit_census::GPR_ST } else { crate::jit_census::GPR_MOV });
        self.a.st32o(o, s);
        self.a.set_tag(t);
    }
    pub(crate) fn alug(&mut self, op: Alu, d: u8, n: u8) { let o = self.gr(n); self.a.alu_ro(op, d, o) }
    /// LR = the address after instruction i (bl / bcl / bclrl / bcctrl).
    pub(crate) fn set_lr(&mut self, i: usize) {
        let v = self.ipc(i).wrapping_add(4);
        let o = self.w(LR);
        let t = self.cat(crate::jit_census::LR_SET);
        self.a.st32io(o, v);
        self.a.set_tag(t);
    }
}
