//! Write hooks on guest RAM: the code-page WATCH a decoded-code cache invalidates by, and the undo JOURNAL a lockstep
//! comparison rolls a run back with.
//!
//! Every byte of guest RAM is written through [`GuestMem::write_bytes`], [`GuestMem::zero`] or the sized
//! `write_u8/u16/u32` (the guest CPU's stores, the native Toolbox, resource loads, BlockMove: there is no other path,
//! `ram` is private). Each calls [`Hooks::before_write`] once with the stripped range, so a watcher sees ALL of them.
//! With no page watched and no journal open the cost is one predictable branch on a bool.
//!
//! The watch is a per-4-KiB-page GENERATION: [`GuestMem::watch_code_page`] marks a page as holding decoded code and
//! returns its generation; the first write to a marked page (or [`GuestMem::invalidate_code`], the guest's `icbi`)
//! bumps the generation and unmarks it, so further writes to that page cost nothing until code there is decoded again.
//! A cache entry is valid exactly while the page's generation equals the one it was decoded at.
use alloc::boxed::Box;
use alloc::vec::Vec;

/// log2 of the watched page size (4 KiB, the PowerPC page).
pub const CODE_PAGE_SHIFT: u32 = 12;

#[derive(Default)]
struct Watch {
    /// One bit per page: code was decoded from it since its last write.
    marked: Vec<u64>,
    /// Per page: bumped by every first write after a mark.
    gen: Vec<u64>,
    /// How many generation bumps happened (all pages): a cheap "did any code page change" probe.
    bumps: u64,
}

impl Watch {
    fn ensure(&mut self, page: usize) {
        if page >= self.gen.len() { self.gen.resize(page + 1, 0); }
        if page / 64 >= self.marked.len() { self.marked.resize(page / 64 + 1, 0); }
    }
    fn hit(&mut self, page: usize) {
        if let Some(w) = self.marked.get_mut(page / 64) {
            let bit = 1u64 << (page % 64);
            if *w & bit != 0 {
                *w &= !bit;
                self.gen[page] = self.gen[page].wrapping_add(1);
                self.bumps = self.bumps.wrapping_add(1);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct Hooks {
    /// `watch.is_some() || journal.is_some()`: the only thing a write tests when nothing is hooked.
    active: bool,
    watch: Option<Box<Watch>>,
    /// (stripped address, byte before the write), in write order.
    journal: Option<Vec<(u32, u8)>>,
}

impl Hooks {
    #[inline(always)]
    pub(crate) fn before_write(&mut self, ram: &[u8], addr: u32, len: u32) {
        if !self.active || len == 0 { return; }
        // the common case inline: no journal, one page, and that page not marked
        if self.journal.is_none() && (addr & ((1 << CODE_PAGE_SHIFT) - 1)) + len <= 1 << CODE_PAGE_SHIFT {
            if let Some(w) = self.watch.as_deref() {
                let page = (addr >> CODE_PAGE_SHIFT) as usize;
                if w.marked.get(page / 64).is_none_or(|m| m & (1 << (page % 64)) == 0) { return; }
            }
        }
        self.slow(ram, addr, len);
    }
    #[cold]
    fn slow(&mut self, ram: &[u8], addr: u32, len: u32) {
        if let Some(w) = self.watch.as_deref_mut() {
            let (first, last) = ((addr >> CODE_PAGE_SHIFT) as usize, ((addr + (len - 1)) >> CODE_PAGE_SHIFT) as usize);
            for p in first..=last { w.hit(p); }
        }
        if let Some(j) = self.journal.as_mut() {
            j.extend((addr..addr + len).map(|a| (a, ram[a as usize])));
        }
    }
    fn refresh(&mut self) { self.active = self.watch.is_some() || self.journal.is_some(); }
}

impl crate::GuestMem {
    /// Mark the page holding `addr` as holding decoded code; returns its generation. The generation changes at the
    /// next write to any byte of the page (from anyone) or [`invalidate_code`](Self::invalidate_code) there.
    pub fn watch_code_page(&mut self, addr: u32) -> u64 {
        let page = (self.strip(addr) >> CODE_PAGE_SHIFT) as usize;
        let h = &mut self.hooks;
        let w = h.watch.get_or_insert_with(Box::default);
        w.ensure(page);
        w.marked[page / 64] |= 1 << (page % 64);
        h.active = true;
        w.gen[page]
    }
    /// The generation of the page holding `addr` (0 for a page never watched).
    #[inline]
    pub fn code_gen(&self, addr: u32) -> u64 {
        let page = (self.strip(addr) >> CODE_PAGE_SHIFT) as usize;
        self.hooks.watch.as_deref().and_then(|w| w.gen.get(page).copied()).unwrap_or(0)
    }
    /// The total number of generation bumps so far: unchanged means no watched page was written.
    #[inline]
    pub fn code_bumps(&self) -> u64 { self.hooks.watch.as_deref().map_or(0, |w| w.bumps) }
    /// The guest's `icbi`: code decoded from the page holding `addr` is stale from now on (if the page is watched).
    pub fn invalidate_code(&mut self, addr: u32) {
        let page = (self.strip(addr) >> CODE_PAGE_SHIFT) as usize;
        if let Some(w) = self.hooks.watch.as_deref_mut() { w.hit(page); }
    }

    /// Start recording the old value of every byte written (a lockstep comparison's undo log). Restarts an open one.
    pub fn journal_begin(&mut self) {
        let mut j = self.hooks.journal.take().unwrap_or_default();
        j.clear();
        self.hooks.journal = Some(j);
        self.hooks.refresh();
    }
    /// Stop recording; the (address, old byte) pairs in write order (empty if no journal was open).
    pub fn journal_end(&mut self) -> Vec<(u32, u8)> {
        let j = self.hooks.journal.take().unwrap_or_default();
        self.hooks.refresh();
        j
    }
    /// Put back the bytes a journal recorded, newest first: memory is as it was at `journal_begin`. The writes go
    /// through the normal path, so watched pages they touch change generation like any other write.
    pub fn journal_undo(&mut self, j: &[(u32, u8)]) {
        for &(a, b) in j.iter().rev() { let _ = self.write_u8(a, b); }
    }
}

#[cfg(test)]
mod tests {
    use crate::GuestMem;

    #[test]
    fn every_write_path_bumps_a_watched_page_once_and_an_unwatched_page_never() {
        let mut m = GuestMem::new(0x4000);
        let g = m.watch_code_page(0x1000);
        m.write_u32(0x2000, 1).unwrap();
        assert_eq!(m.code_gen(0x1000), g, "NEGATIVE CONTROL: a write to another page leaves the generation");
        m.write_u8(0x1FFF, 1).unwrap();
        assert_eq!(m.code_gen(0x1004), g + 1, "a byte store anywhere in the page");
        m.write_u32(0x1000, 2).unwrap();
        assert_eq!(m.code_gen(0x1000), g + 1, "unmarked after the first write: no further bump until watched again");
        for (i, f) in [
            (|m: &mut GuestMem| m.write_u16(0x1002, 7).unwrap()) as fn(&mut GuestMem),
            |m| m.write_bytes(0x0FFE, &[1, 2, 3, 4]).unwrap(),
            |m| m.zero(0x1800, 32).unwrap(),
            |m| m.invalidate_code(0x1ABC),
        ].into_iter().enumerate() {
            let g = m.watch_code_page(0x1000);
            f(&mut m);
            assert_eq!(m.code_gen(0x1000), g + 1, "write path {i}");
        }
        let bumps = m.code_bumps();
        m.write_u32(0x1000, 0).unwrap();
        assert_eq!(m.code_bumps(), bumps, "NEGATIVE CONTROL: the page is unmarked");
    }

    #[test]
    fn a_write_straddling_two_watched_pages_bumps_both() {
        let mut m = GuestMem::new(0x4000);
        let (a, b) = (m.watch_code_page(0x1000), m.watch_code_page(0x2000));
        m.write_u32(0x1FFE, 0xDEAD_BEEF).unwrap();
        assert_eq!((m.code_gen(0x1000), m.code_gen(0x2000)), (a + 1, b + 1));
    }

    #[test]
    fn the_journal_undoes_every_write_and_records_nothing_when_closed() {
        let mut m = GuestMem::new(0x100);
        m.write_bytes(0, &[9; 16]).unwrap();
        m.journal_begin();
        m.write_u32(0, 0x1122_3344).unwrap();
        m.write_u8(1, 0x55).unwrap();
        m.zero(8, 4).unwrap();
        let j = m.journal_end();
        assert_eq!(j.len(), 9);
        m.write_u8(15, 1).unwrap();
        assert_eq!(m.journal_end().len(), 0, "NEGATIVE CONTROL: no journal open, nothing recorded");
        m.journal_undo(&j);
        assert_eq!(m.bytes(m.span(0, 15).unwrap()).unwrap(), &[9; 15], "every byte back, newest undone first");
    }
}
