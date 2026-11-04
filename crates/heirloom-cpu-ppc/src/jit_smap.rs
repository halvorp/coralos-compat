//! Tier 2's STORE MAP: one byte per 4 KiB guest page, 1 = an inline store STARTING there may write (that page AND the next one
//! are wholly inside guest RAM and the code watch marks neither: a store of at most 8 bytes, aligned or not, stays inside the
//! two - so JIT code tests no alignment and no page offset, h86 jitsize), 0 = the store takes the slow path (tier 1's handler: GuestMem's write, which bumps a
//! marked page's generation - the existing self-modifying-code semantics - or the trap past RAM). An inline store then needs
//! one byte compare (`cmp byte [r13 + (ea >> 12) + MAP], 0`: the map is inside the JIT's Ctx, jit.rs, so its base is no
//! load) where it tested RAM bounds and the watch bit (~14 host instructions:
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

/// Entries of the map: one per page of the whole 32-bit address space, so every masked EA indexes inside it whatever the mask.
pub(crate) const MAP_PAGES: usize = 1 << (32 - PAGE_SHIFT);

pub(crate) struct StoreMap {
    /// The `marked` words as last mirrored.
    shadow: Vec<u64>,
    /// What the map was built for: RAM base / length / mask / bitmap, and the watch's counters.
    built: Option<(usize, usize, u32, usize, u32)>,
    seen: (u32, u64),
    /// Full rebuilds and words re-read (diagnostics).
    pub(crate) rebuilds: u64,
    pub(crate) words: u64,
    /// Planted mutant MUT_SMAP_ONE_PAGE: an entry covers its own page only (the pre-jitsize map, under code that no longer
    /// tests alignment).
    one_page: bool,
}

impl StoreMap {
    pub(crate) fn new() -> StoreMap { StoreMap { shadow: Vec::new(), built: None, seen: (0, 0), rebuilds: 0, words: 0, one_page: false } }

    /// Make `map` (the Ctx's) agree with `mem`'s RAM and watch (before an entry into JIT code). False when the shadow could
    /// not be allocated (tier 2 then does not enter: the caller treats it as unusable). `stale` = the planted mutant that
    /// skips the mirroring.
    pub(crate) fn sync(&mut self, mem: &GuestMem, raw: &RawRam, stale: bool, one_page: bool, map: &mut [u8; MAP_PAGES]) -> bool {
        if one_page != self.one_page { (self.one_page, self.built) = (one_page, None); }
        let key = (raw.base as usize, raw.len, raw.mask, raw.marked as usize, raw.marked_words);
        let seen = (mem.code_marks(), mem.code_bumps());
        if self.built != Some(key) {
            let mut shadow = Vec::new();
            if shadow.try_reserve_exact(raw.marked_words as usize).is_err() { self.built = None; return false; }
            let full = raw.len >> PAGE_SHIFT; // pages wholly inside RAM
            let k = if self.one_page { 0 } else { 1 };
            for (p, e) in map.iter_mut().enumerate() { *e = (p + k < full) as u8; }
            shadow.resize(raw.marked_words as usize, 0);
            (self.shadow, self.built) = (shadow, Some(key));
            self.rebuilds += 1;
            self.diff(raw, full, map);
            self.seen = seen;
        } else if seen != self.seen && !stale {
            self.diff(raw, raw.len >> PAGE_SHIFT, map);
            self.seen = seen;
        }
        true
    }

    /// Re-read every bitmap word that differs from the shadow; each page of it, and the page before it (whose entry covers it
    /// too), is 1 when it and the next page are inside RAM and unmarked.
    fn diff(&mut self, raw: &RawRam, full: usize, map: &mut [u8; MAP_PAGES]) {
        for w in 0..self.shadow.len() {
            // SAFETY: w < marked_words, the length of the owner's `marked` array (RawRam's contract; the key above).
            let now = unsafe { *raw.marked.add(w) };
            if now == self.shadow[w] { continue; }
            self.shadow[w] = now;
            self.words += 1;
            for p in (w * 64).saturating_sub(1)..(w * 64 + 64).min(MAP_PAGES) {
                map[p] = (self.free(p, full) && (self.one_page || self.free(p + 1, full))) as u8;
            }
        }
    }
    /// Page `p` is inside RAM and not marked (by the shadow, as mirrored so far).
    fn free(&self, p: usize, full: usize) -> bool { p < full && self.shadow.get(p / 64).is_none_or(|&w| w & (1 << (p % 64)) == 0) }
}
