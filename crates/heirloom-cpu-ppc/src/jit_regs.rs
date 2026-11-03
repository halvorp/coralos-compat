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
//! EXACTNESS: the Cpu holds a global's value wherever Rust can see it. The trampoline loads them; the common exit stores them
//! (every way out of JIT code passes there: chained-exit misses, bails, traps, the slow stub's leaving paths); a call into
//! Rust - the slow stub's helper (tier 1's handler for one instruction), a native block's h_native - is preceded by a store
//! and followed by a load (the handler may write any field). The lockstep journal call `h_note` reads no Cpu field: the
//! caller-saved globals are kept on the stack around it. In JitLockstep every block is entered through the trampoline, so the
//! globals are flushed at every block boundary there.
//!
//! Host registers: r9 r10 r11 (caller-saved) and rbp, r14 (callee-saved; the trampoline saves them). Nothing else in tier 2
//! uses them (scratch is rax rcx rdx rsi rdi r8).
use crate::jit_asm::*;
use crate::jit_emit::{gpr, CR, E, LR, XER};

/// The global pins: (Cpu field offset, host register).
pub(crate) const GLOBALS: [(usize, u8); 5] = [(CR, R14), (gpr(3) as usize, RBP), (XER, R11), (gpr(0) as usize, R10), (gpr(4) as usize, R9)];
/// The caller-saved ones (pushed around the lockstep journal call).
pub(crate) const CALLER_SAVED: [u8; 3] = [R9, R10, R11];

/// The global pin of field `off`, if it is one.
#[inline(always)]
fn global(off: usize) -> Option<u8> { GLOBALS.iter().find(|g| g.0 == off).map(|g| g.1) }
/// Store every global pin into the Cpu (before Rust can read it).
pub(crate) fn store_globals(a: &mut Asm) { for (off, h) in GLOBALS { a.st32(m(RBX, off as i32), h); } }
/// Load every global pin from the Cpu (after Rust may have written it).
pub(crate) fn load_globals(a: &mut Asm) { for (off, h) in GLOBALS { a.ld32(h, m(RBX, off as i32)); } }

impl E<'_, '_> {
    /// The Cpu field at `off` (a 32-bit one): its host register when it is a global pin, else the memory operand.
    #[inline(always)]
    fn field(&self, off: usize) -> O { global(off).map_or(O::M(m(RBX, off as i32)), O::R) }
    /// The Cpu field at `off`, to read / to write / to read and write (the same operand: named for the reader).
    pub(crate) fn r(&mut self, off: usize) -> O { self.field(off) }
    pub(crate) fn w(&mut self, off: usize) -> O { self.field(off) }
    pub(crate) fn rw(&mut self, off: usize) -> O { self.field(off) }
    /// GPR n, to read / to write.
    pub(crate) fn gr(&mut self, n: u8) -> O { self.r(gpr(n) as usize) }
    pub(crate) fn gw(&mut self, n: u8) -> O { self.w(gpr(n) as usize) }
    /// host `d` = GPR n; GPR n = host `s`; host `d` op= GPR n.
    pub(crate) fn ldg(&mut self, d: u8, n: u8) { let o = self.gr(n); self.a.ld32o(d, o) }
    pub(crate) fn stg(&mut self, n: u8, s: u8) { let o = self.gw(n); self.a.st32o(o, s) }
    pub(crate) fn alug(&mut self, op: Alu, d: u8, n: u8) { let o = self.gr(n); self.a.alu_ro(op, d, o) }
    /// LR = the address after instruction i (bl / bcl / bclrl / bcctrl).
    pub(crate) fn set_lr(&mut self, i: usize) {
        let v = self.ipc(i).wrapping_add(4);
        let o = self.w(LR);
        self.a.st32io(o, v);
    }
}
