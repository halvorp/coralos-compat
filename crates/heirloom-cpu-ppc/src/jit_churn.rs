//! Tier 2's COMPILE-CAUSE CENSUS (h86 native, 2026-11-05; diagnostics, off unless Exec::set_jit_churn): why a block is compiled
//! in steady play. WC3 gameplay on the host compiles ~2.7 blocks a frame long after the game's code is warm (88 % of frames
//! compile something); each compiled block is classified by what became of its pc before:
//! * NEW - this pc was never compiled (a path reached for the first time);
//! * INVAL - its block was dropped by the eager invalidation (jit_inval: its code page was written);
//! * EVICT - its block went with an arena segment the FIFO reused;
//! * DROP - every block went (drop_code / flush) since it was last compiled;
//! * STALE - a block is still mapped at pc but its page generation, or a page of its region, moved (jit.rs `hot`);
//! * OTHER - compiled before, none of the above (e.g. refused or never published).
//! It counts per cause and keeps the (pc, cause) of each compile while on, for the harness to symbolise.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

pub const NEW: u8 = 0;
pub const INVAL: u8 = 1;
pub const EVICT: u8 = 2;
pub const DROP: u8 = 3;
pub const STALE: u8 = 4;
pub const OTHER: u8 = 5;
/// The causes' names, by value.
pub const NAMES: [&str; 6] = ["new", "inval", "evict", "drop", "stale", "other"];

#[derive(Default)]
pub(crate) struct Churn {
    pub(crate) on: bool,
    /// pc -> the drop epoch when it was last compiled (a later epoch means a drop_code came between).
    ever: BTreeMap<u32, u32>,
    /// pc -> why its block went (INVAL / EVICT), until it is compiled again.
    why: BTreeMap<u32, u8>,
    epoch: u32,
    pub(crate) counts: [u64; 6],
    pub(crate) log: Vec<(u32, u8)>,
    /// Why the next drop_code happens (set by its caller; DROP_* below), and every whole-code drop seen: (why, guest clock).
    pub(crate) drop_why: u8,
    /// Regions formed while on: (head pc, whether a region was formed at that head before).
    pub(crate) regions: Vec<(u32, bool)>,
    region_heads: alloc::collections::BTreeSet<u32>,
    pub(crate) drops: Vec<(u8, u64)>,
}

/// Why every block went (drop_code's callers).
pub const DROP_RECONFIGURE: u8 = 0;
pub const DROP_T1_FLUSH: u8 = 1;
pub const DROP_RAM: u8 = 2;
pub const DROP_LINKS: u8 = 3;
pub const DROP_ARENA: u8 = 4;
pub const DROP_SO: u8 = 5;
pub const DROP_MAP: u8 = 6;
pub const DROP_NAMES: [&str; 7] = ["reconfigure", "tier1-cache-flush", "ram-or-helper-or-mask", "links-full", "arena", "xer-so", "block-map"];

impl Churn {
    /// Start counting afresh (the history of pcs stays: turn the census on early, reset it where the window starts).
    pub(crate) fn reset(&mut self) { self.counts = [0; 6]; self.log.clear(); self.regions.clear(); }
    /// A region headed by `pc` was compiled.
    pub(crate) fn region(&mut self, pc: u32) {
        if self.on { let again = !self.region_heads.insert(pc); self.regions.push((pc, again)); }
    }
    /// Blocks at these pcs went for `cause` (INVAL or EVICT).
    pub(crate) fn gone(&mut self, pcs: impl Iterator<Item = u32>, cause: u8) {
        if self.on { for pc in pcs { self.why.insert(pc, cause); } }
    }
    /// Every block went (drop_code).
    pub(crate) fn dropped_all(&mut self, clock: u64) {
        if self.on { self.epoch += 1; self.why.clear(); self.drops.push((self.drop_why, clock)); }
        self.drop_why = DROP_RECONFIGURE;
    }
    /// A block is being compiled at `pc`; `mapped` = a block is still mapped there (stale).
    pub(crate) fn compiled(&mut self, pc: u32, mapped: bool) {
        if !self.on { return; }
        let cause = if mapped { STALE } else if let Some(c) = self.why.remove(&pc) { c } else {
            match self.ever.get(&pc) { None => NEW, Some(&e) if e < self.epoch => DROP, Some(_) => OTHER }
        };
        self.ever.insert(pc, self.epoch);
        self.counts[cause as usize] += 1;
        self.log.push((pc, cause));
    }
}
