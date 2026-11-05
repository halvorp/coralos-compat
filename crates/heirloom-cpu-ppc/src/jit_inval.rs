//! Tier 2's EAGER INVALIDATION (h86 fnreg increment 5): code stays exact under self-modifying guests WITHOUT a page-generation test
//! at every block and region entry. The Z83's first real jitablate A/B (fnreg-4, two rotations of WC3 gameplay, each label's
//! second window against base) measured those entry tests at ~0.48 core cycles per guest instruction (nogen 17.51 / 17.56 vs
//! base 17.99; noise floor ~0.15) - one memory load and compare per entry, on the hot path of every dispatcher, IBTC and
//! cross-page chained entry.
//!
//! THE INVARIANT: no JIT code decoded from a code page at generation g is reachable once the page is past g.
//! * A write into a watched page is never inline: the store map sends it to tier 1's handler, which leaves JIT code right after
//!   it (h_one answers 3); a native that wrote one makes JIT code leave too (h_native); writes from outside a run happen while no
//!   JIT code runs. So JIT code always returns to the dispatcher between the write and the next entry.
//! * The dispatcher, before it enters anything, compares the guest RAM's code-write count with the one it saw last
//!   ([`crate::jit::Jit::invalidate_stale`]); when it moved, every page code was decoded from ([`crate::jit::Jit::page_gen`]) is
//!   compared with its generation now, and the code of each changed page goes: its blocks and the regions that include it
//!   (published or still pending), their dispatch-table and IBTC entries, and every link into them is re-pointed at its exit
//!   tail - the dispatcher then decodes and compiles again.
//! KNOB_ENTRY_CHECKS is the way before (entry tests, no eager invalidation: the A/B); KNOB_NO_ENTRY_GEN drops both (the negative
//! control).
use crate::jit::{Jit, EMPTY, IBTC_EMPTY};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use heirloom_mem_flat32::{GuestMem, CODE_PAGE_SHIFT};

/// Are entry tests emitted under `knobs`? With KNOB_ENTRY_CHECKS set and KNOB_NO_ENTRY_GEN (the negative control, which drops
/// every test) clear.
#[inline(always)]
pub(crate) fn entry_checks(knobs: u32) -> bool {
    knobs & heirloom_cpu_abi::ppc::KNOB_ENTRY_CHECKS != 0 && knobs & heirloom_cpu_abi::ppc::KNOB_NO_ENTRY_GEN == 0
}

impl Jit {
    /// Drop the code of every page written since the last look (see the module doc). Cheap when nothing was written: one
    /// compare of the code-write count.
    pub(crate) fn invalidate_stale(&mut self, mem: &GuestMem) {
        let b = mem.code_bumps();
        if b == self.seen_bumps { return; }
        self.seen_bumps = b;
        // KNOB_ENTRY_CHECKS is the pre-fnreg-5 way (the entry tests alone: the A/B); KNOB_NO_ENTRY_GEN neither (the negative control)
        if self.knobs & (heirloom_cpu_abi::ppc::KNOB_NO_ENTRY_GEN | heirloom_cpu_abi::ppc::KNOB_ENTRY_CHECKS) != 0 || self.mutant == crate::jit_emit::MUT_INVAL_STALE { return; }
        let stale: BTreeSet<usize> = self.page_gen.iter().filter(|&(&p, &g)| mem.code_gen((p as u32) << CODE_PAGE_SHIFT) != g).map(|(&p, _)| p).collect();
        if stale.is_empty() { return; }
        for p in &stale { self.page_gen.remove(p); }
        let page = |pc: u32| (mem.strip(pc) >> CODE_PAGE_SHIFT) as usize;
        let regions = &self.regions;
        let hit = |pc: u32, entry: u64| stale.contains(&page(pc)) || regions.get(&entry).is_some_and(|ps| ps.iter().any(|x| stale.contains(&x.0)));
        let (mut gone, mut entries) = (Vec::new(), BTreeSet::new());
        self.blocks.retain(|pc, s| { let h = hit(pc, s.entry); if h { gone.push(pc); entries.insert(s.entry); } !h });
        let mut unpend = Vec::new();
        self.pending.retain(|s| { let h = hit(s.pc, s.entry); if h { unpend.push(s.pc); entries.insert(s.entry); } !h });
        for pc in unpend { self.pending_pcs.remove(pc); }
        if entries.is_empty() { return; }
        for t in self.table.iter_mut() { if entries.contains(&t.entry) { *t = EMPTY; } }
        for v in self.ibtc.iter_mut() { if v[0] != IBTC_EMPTY && entries.contains(&v[1]) { *v = [IBTC_EMPTY, 0]; } }
        for pc in &gone {
            if let Some(v) = self.by_target.get(pc) { for &i in v { self.links[i as usize] = self.link_home[i as usize]; } }
        }
        self.churn.gone(gone.iter().copied(), crate::jit_churn::INVAL);
        self.regions.retain(|e, _| !entries.contains(e));
        self.stats.invalidated += gone.len() as u64;
    }
}
