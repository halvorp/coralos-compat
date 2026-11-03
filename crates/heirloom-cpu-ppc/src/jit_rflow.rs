//! Tier 2's region dataflow (the region tier's increment 2): which compares can leave their result in the host FLAGS for
//! the conditional branch right after them, without the CR field ever being written - because no path from that branch
//! reads the field before it is written again.
//!
//! A CR field is LIVE at a point when some path from it reads the field before writing it, or reaches a place where the
//! field can be seen from outside the region's code: every exit (a chained or indirect exit, the IBTC probe, and every back
//! edge - it may leave on its budget check), and every
//! instruction that may call tier 1's handler (a load or store whose guard can fail, an FP form, a Generic: the handler and
//! a trap read the whole Cpu) - those READ every field here. The analysis is backward, per region node, iterated over the
//! region's internal edges (loops) to a fixpoint; fields are bits of a u8 (bit f = CR field f).
//!
//! A compare (cmpw/cmplw/cmpwi/cmplwi into field f) is FUSED with the conditional branch right after it when that branch
//! tests a LT / GT / EQ bit of f without touching CTR, and f is dead after the branch on both of its edges: the compare
//! then only sets the flags and the branch is a `jcc` on them (no CR materialisation: ~10 host instructions, and the `bt`).
//! Whether that holds is decided here; jit_emit does what it says.
use crate::t1op::{Op, K};
use alloc::vec;
use alloc::vec::Vec;

const ALL: u8 = 0xFF;

/// (fields read, fields written) by one instruction as tier 2 emits it (a possible helper call or exit reads them all).
fn effect(op: &Op) -> (u8, u8) {
    let field = |bi: u8| 1u8 << ((bi & 31) >> 2);
    let cond = |op: &Op| if op.d & 0x10 == 0 { field(op.a) } else { 0 };
    match op.k {
        K::Cmpi | K::Cmpli | K::Cmp | K::Cmpl => (0, 1 << (op.d & 7)),
        K::AndiRc | K::AddicRc | K::RlwinmRc | K::OrRc | K::AndRc | K::AddRc | K::SubfRc | K::SrawiRc => (0, 1),
        K::Nop | K::Li | K::Addi | K::Addic | K::Subfic | K::Mulli | K::Ori | K::Xori | K::Rlwinm | K::Or | K::And | K::Xor
        | K::Andc | K::Nor | K::Add | K::Subf | K::Neg | K::Mullw | K::Extsh | K::Extsb | K::Srawi | K::Slw | K::Srw
        | K::Mflr | K::Mtlr | K::Mfctr | K::Mtctr | K::B | K::Bl => (0, 0),
        K::Bc | K::Bcl => (cond(op), 0),
        _ => (ALL, 0),
    }
}

/// The fused compares of a region (jit_region::Plan's nodes and return points): per node, per instruction, true = the
/// compare there only sets the flags for the branch after it. `ignore_liveness` = planted mutant 27 (fuse when live too).
pub(crate) fn fuse_marks(nodes: &[(u32, Vec<Op>)], rets: &[u32], ignore_liveness: bool) -> Vec<Vec<bool>> {
    let index = |pc: u32| nodes.iter().position(|n| n.0 == pc);
    let mut live_in = vec![0u8; nodes.len()];
    let mut after: Vec<Vec<u8>> = nodes.iter().map(|n| vec![0u8; n.1.len()]).collect();
    loop {
        let mut changed = false;
        for (k, (pc, ops)) in nodes.iter().enumerate().rev() {
            // an edge to a LATER node continues there; one back to a node at or before this one (a loop) re-checks the
            // budget and may leave through the chained exit right there: an exit
            let tgt = |t: u32| index(t).filter(|&j| j > k).map_or(ALL, |j| live_in[j]);
            let n = ops.len();
            let next = |i: usize| pc.wrapping_add(4 * (i as u32 + 1));
            let last = &ops[n - 1];
            let always = last.d & 0x14 == 0x14;
            // what the node's end leads to (go(): a node of the region, or an exit)
            let mut live = match last.k {
                K::B | K::Bl => tgt(last.imm),
                K::Bc | K::Bcl => tgt(last.imm) | if always { 0 } else { tgt(next(n - 1)) },
                K::Bclr => ALL | rets.iter().fold(0, |m, &r| m | tgt(r)),
                _ if last.terminal() => ALL,
                _ => tgt(next(n - 1)),
            };
            for i in (0..n).rev() {
                let op = &ops[i];
                // a superblock's inner conditional branch: its taken side leaves the node
                if i + 1 < n && matches!(op.k, K::Bc | K::Bcl) { live |= tgt(op.imm); }
                after[k][i] = live;
                let (r, w) = effect(op);
                live = r | (live & !w);
            }
            if live != live_in[k] { live_in[k] = live; changed = true; }
        }
        if !changed { break; }
    }
    nodes.iter().enumerate().map(|(k, (_, ops))| {
        (0..ops.len()).map(|i| {
            let (op, Some(br)) = (&ops[i], ops.get(i + 1)) else { return false };
            let f = op.d & 7;
            matches!(op.k, K::Cmpi | K::Cmpli | K::Cmp | K::Cmpl) && matches!(br.k, K::Bc | K::Bcl)
                && br.d & 0x04 != 0 && br.d & 0x10 == 0 && (br.a & 31) >> 2 == f && br.a & 3 != 3
                && (after[k][i + 1] & (1 << f) == 0 || ignore_liveness)
        }).collect()
    }).collect()
}
