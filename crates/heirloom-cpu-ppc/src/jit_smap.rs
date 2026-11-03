//! Tier 2's STORE MAP: one byte per 4 KiB guest page, 1 = an inline store may write there (the whole page is inside guest RAM
//! and the code watch does not mark it), 0 = the store takes the slow path (tier 1's handler: GuestMem's write, which bumps a
//! marked page's generation - the existing self-modifying-code semantics - or the trap past RAM). An inline store then needs
//! one byte compare (`cmp byte [map + ea >> 12], 0`) where it tested RAM bounds and the watch bit (~14 host instructions:
//! bounds, a 64-bit bitmap word load, BT); never `bt [mem], reg` (microcoded on the Atom).
//!
//! The map MIRRORS the watch's `marked` bitmap, and the watch stays the authority: it is checked before every entry into JIT
//! code (`sync`). A mark is set only by `watch_code_page` (counted in `CodeWatch::marks`) and cleared only by a write or `icbi`
//! (each one a generation bump, `bumps`), so (marks, bumps) unchanged = the bitmap unchanged; when either moved, the bitmap
//! words that differ from the map's shadow copy are re-read (64 pages each). A page marked while JIT code runs cannot happen:
//! marking is tier 1's decode, outside JIT code. A page UNMARKED while JIT code runs (a store through the helper) keeps its 0
//! until the next entry: the slow path, never a stale fast one.
use alloc::vec::Vec;
use heirloom_mem_flat32::{GuestMem, RawRam};

const PAGE_SHIFT: u32 = 12;

pub(crate) struct StoreMap {
    /// One byte per page of the masked address space ((mask >> 12) + 1 entries): every masked EA indexes inside it.
    pub(crate) map: Vec<u8>,
    /// The `marked` words as last mirrored.
    shadow: Vec<u64>,
    /// What the map was built for: RAM base / length / mask / bitmap, and the watch's counters.
    built: Option<(usize, usize, u32, usize, u32)>,
    seen: (u32, u64),
    /// Full rebuilds and words re-read (diagnostics).
    pub(crate) rebuilds: u64,
    pub(crate) words: u64,
}

impl StoreMap {
    pub(crate) fn new() -> StoreMap { StoreMap { map: Vec::new(), shadow: Vec::new(), built: None, seen: (0, 0), rebuilds: 0, words: 0 } }

    /// Make the map agree with `mem`'s RAM and watch (before an entry into JIT code). False when it could not be allocated
    /// (tier 2 then does not enter: the caller treats it as unusable). `stale` = the planted mutant that skips the mirroring.
    pub(crate) fn sync(&mut self, mem: &GuestMem, raw: &RawRam, stale: bool) -> bool {
        let key = (raw.base as usize, raw.len, raw.mask, raw.marked as usize, raw.marked_words);
        let seen = (mem.code_marks(), mem.code_bumps());
        if self.built != Some(key) {
            let n = (raw.mask >> PAGE_SHIFT) as usize + 1;
            let (mut map, mut shadow) = (Vec::new(), Vec::new());
            if map.try_reserve_exact(n).is_err() || shadow.try_reserve_exact(raw.marked_words as usize).is_err() { self.built = None; return false; }
            let full = raw.len >> PAGE_SHIFT; // pages wholly inside RAM
            map.extend((0..n).map(|p| (p < full) as u8));
            shadow.resize(raw.marked_words as usize, 0);
            (self.map, self.shadow, self.built) = (map, shadow, Some(key));
            self.rebuilds += 1;
            self.diff(raw, full);
            self.seen = seen;
        } else if seen != self.seen && !stale {
            self.diff(raw, raw.len >> PAGE_SHIFT);
            self.seen = seen;
        }
        true
    }

    /// Re-read every bitmap word that differs from the shadow: a marked page 0, an unmarked page inside RAM 1.
    fn diff(&mut self, raw: &RawRam, full: usize) {
        for w in 0..self.shadow.len() {
            // SAFETY: w < marked_words, the length of the owner's `marked` array (RawRam's contract; the key above).
            let now = unsafe { *raw.marked.add(w) };
            if now == self.shadow[w] { continue; }
            self.shadow[w] = now;
            self.words += 1;
            for b in 0..64 {
                let p = w * 64 + b;
                if p >= self.map.len() { break; }
                self.map[p] = (p < full && now & (1 << b) == 0) as u8;
            }
        }
    }
}
