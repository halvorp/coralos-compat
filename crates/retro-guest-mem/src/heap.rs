//! A guest heap over a range of guest RAM, shared by every engine.
//!
//! The bookkeeping lives HOST-side (the guest cannot corrupt it): a first-fit free list and the set of live blocks.
//! A free of an address this heap never returned, or with a size other than the one allocated, is a named error,
//! not a silent corruption. Every block is 8-byte aligned and a multiple of 8 bytes long.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

/// Block granularity (AmigaOS MEM_BLOCKSIZE; classic Mac Memory Manager blocks are also at least 4-byte aligned).
pub const GRAIN: u32 = 8;

/// The guest heap: [start, end) of guest RAM, a first-fit free list (address -> length) and the live blocks.
pub struct Heap { free: BTreeMap<u32, u32>, live: BTreeMap<u32, u32> }

impl Heap {
    pub fn new(start: u32, end: u32) -> Heap {
        let s = (start + GRAIN - 1) & !(GRAIN - 1);
        Heap { free: BTreeMap::from([(s, end - s)]), live: BTreeMap::new() }
    }
    /// Add a free region `[start, start + len)` (memory that did not exist when the heap was made). It must not overlap anything the heap knows.
    pub fn add_region(&mut self, start: u32, len: u32) {
        let s = (start + GRAIN - 1) & !(GRAIN - 1);
        let len = len.saturating_sub(s - start) & !(GRAIN - 1);
        if len > 0 { self.free.insert(s, len); }
    }
    /// First fit; None = no block large enough (AllocMem's NULL, a legitimate answer).
    pub fn alloc(&mut self, size: u32) -> Option<u32> {
        let want = size.checked_add(GRAIN - 1)? & !(GRAIN - 1);
        let (&at, &len) = self.free.iter().find(|(_, &l)| l >= want)?;
        self.free.remove(&at);
        if len > want { self.free.insert(at + want, len - want); }
        self.live.insert(at, want);
        Some(at)
    }
    /// Return a block. Refused unless `at` is a live block whose rounded size is `size`'s.
    pub fn free(&mut self, at: u32, size: u32) -> Result<(), String> {
        let want = size.checked_add(GRAIN - 1).ok_or("size overflows")? & !(GRAIN - 1);
        match self.live.get(&at) {
            None => return Err(format!("{at:#x} is not a block this heap allocated")),
            Some(&l) if l != want => return Err(format!("{at:#x} is a {l}-byte block, freed as {size}")),
            _ => {}
        }
        self.live.remove(&at);
        let mut start = at; let mut len = want;
        if let Some((&p, &pl)) = self.free.range(..at).next_back() { if p + pl == at { self.free.remove(&p); start = p; len += pl; } }
        if let Some(&nl) = self.free.get(&(at + want)) { self.free.remove(&(at + want)); len += nl; }
        self.free.insert(start, len);
        Ok(())
    }
    pub fn available(&self, largest: bool) -> u32 {
        if largest { self.free.values().copied().max().unwrap_or(0) } else { self.free.values().sum() }
    }
    pub fn live_blocks(&self) -> usize { self.live.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_fit_rounds_coalesces_and_refuses_bad_frees() {
        let mut h = Heap::new(0x1000, 0x2000);
        let a = h.alloc(10).unwrap();
        let b = h.alloc(16).unwrap();
        assert_eq!((a, b), (0x1000, 0x1010), "10 rounds to 16");
        assert!(h.free(a, 20).is_err(), "wrong size refused");
        assert!(h.free(0x1004, 8).is_err(), "not a block refused");
        h.free(a, 10).unwrap();
        assert!(h.free(a, 10).is_err(), "double free refused");
        h.free(b, 16).unwrap();
        assert_eq!(h.available(true), 0x1000, "everything coalesced back into one block");
        assert!(h.alloc(0x1001).is_none(), "too large is NULL, not a panic");
        assert_eq!(h.live_blocks(), 0);
    }
}

