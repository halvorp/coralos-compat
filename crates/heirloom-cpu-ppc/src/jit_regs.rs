//! Tier 2's register cache: the Cpu fields a block uses most (GPRs, CR, XER, LR, CTR - every one a 32-bit field at
//! `rbx + offset`) live in host registers for the block's whole body.
//!
//! A block is compiled twice: the COUNTING pass runs the same emitter with every guest-register access answered by memory
//! and tallied per field (its code is thrown away, and it makes no link slots); then up to [`HOST`].len() fields are
//! PINNED, each to one host register, those whose memory accesses a register saves the most: a pinned field costs one load
//! at entry if its first access reads it and one store per exit taken if it is written, against one memory access per use
//! (a field is pinned only when that saves at least one). Then the real pass emits:
//! - after the entry checks, every pinned field whose first access reads it is loaded (nothing is pinned on the bail paths;
//!   a field written first needs no load: until it is written it is not dirty, so no exit stores it);
//! - an access to a pinned field is a register operand; a write marks it DIRTY (a compile-time set: code inside a block is
//!   straight-line, its only joins are the slow tails, which carry the dirty set of their jump);
//! - before every exit (a chained or dynamic jump, the dispatcher) and every helper call, the dirty fields are stored; after
//!   a helper call - the handler may have written any field - every pinned field is reloaded and nothing is dirty.
//! So at every point where code outside the block can see the Cpu (an exit, a helper, a trap the slow stub raises), memory
//! holds exactly what the memory-operand compiler would have left there: tier 1's exactness contract is unchanged.
//! CR and XER are fields like any other, so a compare's CR field and a carry's XER[CA] are register operations,
//! materialised in the Cpu only at exits and helpers ("lazy" CR / XER, as far as exactness allows: every exit and helper
//! sees them exact).
//!
//! Host registers: r9 r10 r11 (caller-saved: the lockstep journal call `h_note` keeps them on the stack) and rbp, r14
//! (callee-saved; the trampoline saves them; nothing else in tier 2 uses them - the budget is read from the Ctx).
use crate::jit_asm::*;
use crate::jit_emit::{gpr, E, LR, MUT_NO_SPILL};
use alloc::vec::Vec;

/// The host registers a block may pin guest fields to, in the order fields are given them.
pub(crate) const HOST: [u8; 5] = [R9, R10, R11, RBP, R14];
/// The caller-saved ones (pushed around the lockstep journal call).
pub(crate) const CALLER_SAVED: [u8; 3] = [R9, R10, R11];

/// How the counting pass saw one field.
#[derive(Clone, Copy)]
struct Use { off: usize, uses: u32, read_first: bool, written: bool }

/// An access: a read, a write that reads nothing (the whole field replaced), or both.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Acc { R, W, Rw }

pub(crate) struct Regs {
    /// Counting pass: every field used, in order of first use.
    count: Option<Vec<Use>>,
    /// (Cpu field offset, host register, loaded at entry), by pin index.
    pins: Vec<(usize, u8, bool)>,
    /// Pin indices whose host register holds a value memory does not have yet.
    pub(crate) dirty: u8,
}

impl Regs {
    pub(crate) fn counting() -> Regs { Regs { count: Some(Vec::new()), pins: Vec::new(), dirty: 0 } }
    pub(crate) fn pinned(pins: Vec<(usize, u8, bool)>) -> Regs { Regs { count: None, pins, dirty: 0 } }
    pub(crate) fn counting_pass(&self) -> bool { self.count.is_some() }

    /// After the counting pass: the fields a register saves the most memory accesses for (at least one), each with its host
    /// register and whether the entry loads it.
    pub(crate) fn choose(self) -> Vec<(usize, u8, bool)> {
        let saved = |u: &Use| u.uses as i32 - u.read_first as i32 - u.written as i32;
        let mut c = self.count.unwrap_or_default();
        c.retain(|u| saved(u) >= 1);
        // most saved first; ties by offset, so a block always compiles to the same code
        c.sort_by(|x, y| saved(y).cmp(&saved(x)).then(x.off.cmp(&y.off)));
        c.iter().zip(HOST).map(|(u, h)| (u.off, h, u.read_first)).collect()
    }

    fn at(&mut self, off: usize, acc: Acc) -> O {
        if let Some(c) = self.count.as_mut() {
            match c.iter_mut().find(|u| u.off == off) {
                Some(u) => { u.uses += 1; u.written |= acc != Acc::R; }
                None => c.push(Use { off, uses: 1, read_first: acc != Acc::W, written: acc != Acc::R }),
            }
            return O::M(m(RBX, off as i32));
        }
        match self.pins.iter().position(|p| p.0 == off) {
            Some(k) => {
                if acc != Acc::R { self.dirty |= 1 << k; }
                O::R(self.pins[k].1)
            }
            None => O::M(m(RBX, off as i32)),
        }
    }
}

impl E<'_, '_> {
    pub(crate) fn counting(&self) -> bool { self.regs.counting_pass() }
    /// The Cpu field at `off` (a 32-bit one), to read / to write / to read and write.
    pub(crate) fn r(&mut self, off: usize) -> O { self.regs.at(off, Acc::R) }
    pub(crate) fn w(&mut self, off: usize) -> O { self.regs.at(off, Acc::W) }
    pub(crate) fn rw(&mut self, off: usize) -> O { self.regs.at(off, Acc::Rw) }
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

    /// Store every dirty pinned field (before an exit when `exit`, else before a helper call). The dirty set is left as
    /// it is: the code after a conditional exit still has those values only in registers.
    pub(crate) fn spill(&mut self, exit: bool) {
        let mut d = self.regs.dirty;
        if exit && self.env.mutant == MUT_NO_SPILL && d != 0 { d &= !(1 << (7 - d.leading_zeros())); }
        for (k, &(off, h, _)) in self.regs.pins.iter().enumerate() {
            if d & 1 << k != 0 { self.a.st32(m(RBX, off as i32), h); }
        }
    }
    /// Load every pinned field (after a helper call; at the block entry only those read before written): nothing is dirty
    /// after it.
    pub(crate) fn reload(&mut self, entry: bool) {
        for &(off, h, read_first) in &self.regs.pins { if read_first || !entry { self.a.ld32(h, m(RBX, off as i32)); } }
        self.regs.dirty = 0;
    }
}
