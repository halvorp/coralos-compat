//! Tier 2's REGION tier (council step 4, h86 2026-11-03): when a compiled block's head gets hot - its entry counter in the
//! Ctx (jit_emit: one `sub [r13 + counter], 1` per block entry) runs out after [`REGION_AFTER`] entries - the dispatcher
//! forms a region from it here and compiles it as ONE publish unit (jit_emit::compile_region): up to RegionShape::max_nodes
//! superblocks (each node is what tier 2 would compile at that pc on its own: jit::superblock) and max_insns guest
//! instructions on at most max_pages code pages, reached from the head along CONSTANT edges whose target is already a
//! hot compiled block (it ran [`edge_min`] times) - never through an indirect branch (blr / bctr: the region ends there,
//! the IBTC probe as always), never through a bound native, `sc` or a volatile page (Cache::peek refuses those).
//!
//! An edge from one node to another node of the region is a direct `jmp rel32` inside the published unit (no link slot, no
//! page-generation check, no budget check, no pc store); every other edge is the block tier's chained exit. The region's
//! entry checks the generation of EVERY page it was decoded from (the head's first: a chained jump from the head's page at
//! the head's generation lands past that one, the inner entry) and the budget for its longest FORWARD path; an edge back to
//! a node at or before its source (a loop) re-checks the budget for the longest forward path from that node, and leaves
//! through the chained exit when it does not fit. Inside the unit nothing can change a page's generation without leaving
//! first (an inline store never writes a watched page; the helper leaves when any code page was written), so the entry's
//! checks hold for the whole stay.
//!
//! Formation is a breadth-first walk (layout order = discovery order); a node's longest forward path is computed backwards
//! over the forward edges, which go to a LATER node by construction, so it is finite. The selection is by node hotness
//! only (the counters count block entries, not edges), as the council's rule says: a hot target is admitted, a cold one is a
//! side exit. HQEMU's 50 and 16 blocks are NOT taken as constants of ours: these are measured on WC3 (wc3-status.md, h86
//! ppc-jit5).
use crate::t1cache::Cache;
use crate::t1op::{Op, K};
use alloc::vec::Vec;
use heirloom_mem_flat32::{GuestMem, CODE_PAGE_SHIFT};
pub(crate) use crate::jit_stats::RegionShape;

/// Block entries after which a compiled block's head forms a region (0 = no regions). Tuned by measurement only.
pub(crate) const REGION_AFTER: u32 = 64;
/// A compiled block is admitted into a region once it ran a quarter of the region threshold (16 at 64: the council's hot-edge
/// rule), at least once.
pub(crate) fn edge_min(region_after: u32) -> u32 { (region_after / 4).max(1) }
/// A formed region: the nodes in layout order (pc, ops), each node's longest forward path in instructions (its own included),
/// and the code pages it was decoded from: (page, generation, a pc on the page), the head's page first.
pub(crate) struct Plan {
    pub(crate) nodes: Vec<(u32, Vec<Op>)>,
    pub(crate) maxfwd: Vec<u32>,
    pub(crate) pages: Vec<(usize, u64, u32)>,
    /// Return points: nodes at the instruction after a followed `bl` (a `blr` checks for them inline).
    pub(crate) rets: Vec<u32>,
}

/// The constant successors of a node at `pc` (a superblock): every mid-node conditional branch's taken target, then the
/// last instruction's targets (a fall-through where it has one); with `calls`, a `bl`'s callee and its return point. Indirect
/// targets are not successors.
pub(crate) fn successors(pc: u32, ops: &[Op], calls: bool, out: &mut Vec<u32>) {
    let n = ops.len();
    for (i, op) in ops.iter().enumerate() {
        let next = pc.wrapping_add(4 * (i as u32 + 1));
        let always = op.d & 0x14 == 0x14;
        let call = matches!(op.k, K::Bl | K::Bcl);
        match op.k {
            // a superblock's inner conditional branch: the taken side leaves the node (the not-taken one is the node)
            K::Bc | K::Bcl if i + 1 < n => { if !call || calls { out.push(op.imm) } }
            K::B => out.push(op.imm),
            K::Bl => { if calls { out.push(op.imm); out.push(next) } }
            K::Bc | K::Bcl => {
                if !call || calls { out.push(op.imm) }
                if !always { out.push(next) }
            }
            K::Bclr | K::Bclrl | K::Bcctr | K::Bcctrl => { if !always { out.push(next) } }
            K::GenericEnd | K::Sc => {}
            _ if i + 1 == n => out.push(next),
            _ => {}
        }
    }
}

/// Form the region headed by `head` (a compiled block whose counter ran out). `hot(pc)` = a compiled block at pc ran at
/// least [`edge_min`] times; `gen_pages` = the pages the code watch has a generation for. None when fewer than two nodes
/// qualify and the head does not loop to itself (one node is what tier 2 already runs).
pub(crate) fn form(cache: &Cache, mem: &GuestMem, head: u32, gen_pages: usize, shape: RegionShape, knobs: u32, hot: impl Fn(u32) -> bool) -> Option<Plan> {
    let mut nodes: Vec<(u32, Vec<Op>)> = Vec::new();
    let mut pages: Vec<(usize, u64, u32)> = Vec::new();
    let mut queue: Vec<u32> = alloc::vec![head];
    let (mut q, mut total) = (0, 0);
    let mut succ = Vec::new();
    while q < queue.len() && nodes.len() < shape.max_nodes {
        let pc = queue[q];
        q += 1;
        let page = (mem.strip(pc) >> CODE_PAGE_SHIFT) as usize;
        if page >= gen_pages || nodes.iter().any(|n| n.0 == pc) { continue; }
        let gen = mem.code_gen(pc);
        let new_page = !pages.iter().any(|p| p.0 == page);
        if new_page && pages.len() == shape.max_pages { continue; }
        if let Some(p) = pages.iter().find(|p| p.0 == page) { if p.1 != gen { continue; } }
        let mut first = Vec::new();
        let room = shape.max_insns - total;
        let got = match cache.hit(mem, pc) {
            Some((start, len, g)) if g == gen && len <= room => { first.extend_from_slice(&cache.ops[start..start + len]); true }
            _ => cache.peek(mem, pc, gen, room, &mut first),
        };
        if !got { if nodes.is_empty() { return None; } continue; }
        let mut ops = crate::jit::superblock(cache, mem, &first, pc, gen, page, knobs);
        if ops.len() > room { ops.truncate(first.len()); }
        total += ops.len();
        if new_page { pages.push((page, gen, pc)); }
        succ.clear();
        successors(pc, &ops, shape.follow_calls, &mut succ);
        // a translated shape (jit_xlate) is never a node: its own block is faster than any region code for it (a side exit)
        let shape = |s: u32| crate::jit_xlate::recognise(s, CODE_PAGE_SHIFT, knobs, |a| mem.read_u32(a).ok()).is_some();
        for &s in &succ { if s == head || (hot(s) && !queue.contains(&s) && !shape(s)) { queue.push(s); } }
        nodes.push((pc, ops));
    }
    // one node is what tier 2 already runs - unless it loops to itself (its back edge becomes an internal jump)
    if nodes.len() < 2 {
        succ.clear();
        successors(head, &nodes.first()?.1, shape.follow_calls, &mut succ);
        if !succ.contains(&head) { return None; }
    }
    // return points: nodes right after a node's closing `bl` (a `blr` may go there through its inline return check)
    let rets: Vec<u32> = if shape.follow_calls {
        nodes.iter().filter(|n| n.1.last().is_some_and(|o| o.k == K::Bl)).map(|n| n.0.wrapping_add(4 * n.1.len() as u32)).filter(|r| nodes.iter().any(|n| n.0 == *r)).collect()
    } else { Vec::new() };
    // the longest forward path from each node (forward edges go to a later node: a back edge re-checks the budget)
    let mut maxfwd = alloc::vec![0u32; nodes.len()];
    for i in (0..nodes.len()).rev() {
        succ.clear();
        successors(nodes[i].0, &nodes[i].1, shape.follow_calls, &mut succ);
        if nodes[i].1.last().is_some_and(|o| o.k == K::Bclr) { succ.extend_from_slice(&rets); }
        let tail = succ.iter().filter_map(|s| nodes.iter().position(|n| n.0 == *s)).filter(|&j| j > i).map(|j| maxfwd[j]).max().unwrap_or(0);
        maxfwd[i] = nodes[i].1.len() as u32 + tail;
    }
    Some(Plan { nodes, maxfwd, pages, rets })
}
