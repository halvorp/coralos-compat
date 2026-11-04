//! Tier 2's LOAD GROUPS (h86 jitsize increment 2): D-form loads in one block (or region node) off the same base register,
//! with no write to that register between them, share ONE RAM bounds test. The group's head tests the whole span its
//! members read - `rA + lo` through `rA + lo + span`, every offset an immediate - and its members test nothing: when the head's
//! test passes, every member's EA is inside RAM (no 32-bit wrap: lo + span stays under the RAM length). When it fails (some
//! member would be outside RAM, or the span wraps), the head runs on tier 1's handler and the block LEAVES right after it,
//! so the dispatcher - tier 1 - runs the rest one instruction at a time, each with its own test, exactly.
//!
//! What ends a group: a write to the base register (an instruction whose destination it is, an update form, a Generic or
//! `sc` - tier 1's handler may write any register), the end of the block. Stores do not join a load group and do not end one
//! unless they update the base.
//!
//! STORE GROUPS, the same way: D-form stores off one unwritten base whose bytes span at most a page share ONE store-map test
//! (jit_smap: the entry of the page holding the group's lowest EA covers that page and the next, which hold the whole span).
//! The map cannot turn against a member after the head's test: a page is marked (its entry cleared) only by tier 1's decode,
//! outside JIT code - and a member that WRITES a watched page is not possible then. When the head's test fails, the head
//! runs on tier 1's handler and the block leaves after it, as for loads. Lockstep still journals every member's old bytes.
use crate::t1op::{Op, K};
use alloc::vec;
use alloc::vec::Vec;

/// An instruction's part in a load group.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Grp {
    None,
    /// The group's first load: test `[rA + lo, rA + lo + span)` once.
    Head { lo: i32, span: u32 },
    /// A later load of the group: no test of its own.
    Member,
}

/// A D-form store: (base register, offset, bytes, no GPR written, update form).
fn store(op: &Op) -> Option<(u8, i32, u32, Option<u8>, bool)> {
    let n = match op.k { K::Stw | K::Stwu | K::Stfs => 4, K::Stb | K::Stbu => 1, K::Sth => 2, K::Stfd => 8, _ => return None };
    Some((op.a, op.imm as i32, n, None, matches!(op.k, K::Stwu | K::Stbu)))
}

/// A D-form load: (base register, offset, bytes, the GPR it writes, update form).
fn load(op: &Op) -> Option<(u8, i32, u32, Option<u8>, bool)> {
    let n = match op.k { K::Lwz | K::Lwzu | K::Lfs => 4, K::Lbz | K::Lbzu => 1, K::Lhz | K::Lha => 2, K::Lfd => 8, _ => return None };
    let gpr = !matches!(op.k, K::Lfs | K::Lfd);
    Some((op.a, op.imm as i32, n, gpr.then_some(op.d), matches!(op.k, K::Lwzu | K::Lbzu)))
}

/// May `op` write GPR `r` (Some(true)), certainly not (Some(false)), or is it unknown (None: tier 1's handler runs it)?
fn writes(op: &Op, r: u8) -> Option<bool> {
    Some(match op.k {
        K::Generic | K::GenericEnd | K::Sc => return None,
        // compares write a CR field, mtlr / mtctr an SPR, branches CTR / LR, plain stores memory, FP forms FPRs / FPSCR / CR1
        K::Nop | K::Cmpi | K::Cmpli | K::Cmp | K::Cmpl | K::Mtlr | K::Mtctr | K::B | K::Bl | K::Bc | K::Bcl | K::Bclr | K::Bclrl
        | K::Bcctr | K::Bcctrl | K::Stw | K::Stb | K::Sth | K::Stwx | K::Stbx | K::Stfs | K::Stfd | K::Fp59 | K::Fp63 | K::Lfs | K::Lfd => false,
        K::Stwu | K::Stbu => op.a == r,
        K::Lwzu | K::Lbzu => op.a == r || op.d == r,
        // every other form tier 2 decodes writes exactly its op.d (t1op: the destination, whatever the encoding's field)
        _ => op.d == r,
    })
}

/// The load groups of `ops` (one block or region node), per instruction. `short` = planted mutant MUT_LGROUP_SHORT (the span
/// leaves out the last member's bytes).
pub(crate) fn plan(ops: &[Op], short: bool) -> Vec<Grp> { plan_by(ops, load, u32::MAX, short) }

/// The store groups of `ops`, per instruction (span at most a page; `wide` = planted mutant MUT_SGROUP_WIDE: any span).
pub(crate) fn plan_stores(ops: &[Op], wide: bool) -> Vec<Grp> { plan_by(ops, store, if wide { u32::MAX } else { 4096 }, false) }

fn plan_by(ops: &[Op], access: fn(&Op) -> Option<(u8, i32, u32, Option<u8>, bool)>, max_span: u32, short: bool) -> Vec<Grp> {
    let mut g = vec![Grp::None; ops.len()];
    let mut i = 0;
    while i < ops.len() {
        let Some((ra, off, n, dst, upd)) = access(&ops[i]) else { i += 1; continue };
        if ra == 0 || upd || dst == Some(ra) { i += 1; continue; }
        let (mut lo, mut hi, mut last_n, mut members) = (off as i64, off as i64 + n as i64, n, vec![i]);
        let mut j = i + 1;
        while j < ops.len() {
            let op = &ops[j];
            if let Some((rb, o, m, d, u)) = access(op) {
                let (lo2, hi2) = (lo.min(o as i64), hi.max(o as i64 + m as i64));
                if rb == ra && hi2 - lo2 <= max_span as i64 {
                    members.push(j);
                    lo = lo.min(o as i64);
                    if o as i64 + m as i64 >= hi { hi = o as i64 + m as i64; last_n = m; }
                    if u || d == Some(ra) { break; }
                    j += 1;
                    continue;
                }
            }
            if writes(op, ra) != Some(false) || op.terminal() { break; }
            j += 1;
        }
        if members.len() >= 2 {
            let span = (hi - lo) as u32 - if short { last_n } else { 0 };
            g[i] = Grp::Head { lo: lo as i32, span };
            for &m in &members[1..] { g[m] = Grp::Member; }
        }
        i = members[members.len() - 1] + 1;
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    fn op(k: K, d: u8, a: u8, imm: i32) -> Op { Op { k, d, a, b: 0, imm: imm as u32, w: 0 } }

    #[test]
    fn loads_off_one_unwritten_base_share_a_test_and_a_write_to_it_ends_the_group() {
        let ops = [op(K::Lwz, 5, 3, 8), op(K::Addi, 6, 3, 4), op(K::Lwz, 7, 3, -4), op(K::Lbz, 8, 3, 20), op(K::Addi, 3, 3, 1), op(K::Lwz, 9, 3, 0)];
        let g = plan(&ops, false);
        assert_eq!(g, [Grp::Head { lo: -4, span: 25 }, Grp::None, Grp::Member, Grp::Member, Grp::None, Grp::None]);
    }

    #[test]
    fn stores_off_one_base_within_a_page_share_a_test_and_a_wider_span_or_a_base_write_ends_the_group() {
        let ops = [op(K::Stw, 0, 1, 8), op(K::Stw, 31, 1, -4), op(K::Stfd, 1, 1, 16), op(K::Stwu, 1, 1, -64), op(K::Stw, 5, 1, 0)];
        assert_eq!(plan_stores(&ops, false), [Grp::Head { lo: -64, span: 88 }, Grp::Member, Grp::Member, Grp::Member, Grp::None]);
        let ops = [op(K::Stw, 5, 3, 0), op(K::Stw, 6, 3, 0x1000), op(K::Stw, 7, 3, 4)];
        assert_eq!(plan_stores(&ops, false), [Grp::Head { lo: 0, span: 8 }, Grp::None, Grp::Member]);
        let ops = [op(K::Stw, 5, 3, 0), op(K::Lwz, 3, 4, 0), op(K::Stw, 7, 3, 4)];
        assert_eq!(plan_stores(&ops, false), [Grp::None, Grp::None, Grp::None]);
    }

    #[test]
    fn a_load_into_its_own_base_or_an_update_form_is_the_last_member_and_a_generic_ends_a_group() {
        let ops = [op(K::Lwz, 5, 3, 0), op(K::Lwz, 3, 3, 4), op(K::Lwz, 6, 3, 8)];
        assert_eq!(plan(&ops, false), [Grp::Head { lo: 0, span: 8 }, Grp::Member, Grp::None]);
        let ops = [op(K::Lwz, 5, 3, 0), op(K::Lwzu, 6, 3, 4), op(K::Lwz, 7, 3, 8)];
        assert_eq!(plan(&ops, false), [Grp::Head { lo: 0, span: 8 }, Grp::Member, Grp::None]);
        let ops = [op(K::Lwz, 5, 3, 0), op(K::Generic, 0, 0, 0), op(K::Lwz, 7, 3, 8)];
        assert_eq!(plan(&ops, false), [Grp::None, Grp::None, Grp::None]);
        let ops = [op(K::Lfs, 1, 4, 0), op(K::Stw, 5, 6, 0), op(K::Lfd, 2, 4, 8), op(K::Stwu, 1, 4, -16), op(K::Lfs, 3, 4, 0)];
        assert_eq!(plan(&ops, false), [Grp::Head { lo: 0, span: 16 }, Grp::None, Grp::Member, Grp::None, Grp::None]);
    }
}
