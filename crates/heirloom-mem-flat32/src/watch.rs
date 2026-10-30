//! Write hooks on guest RAM: the code-page WATCH a decoded-code cache invalidates by, and the undo JOURNAL a lockstep
//! comparison rolls a run back with.
//!
//! Every byte of guest RAM is written through [`GuestMem::write_bytes`], [`GuestMem::zero`] or the sized
//! `write_u8/u16/u32` (the guest CPU's stores, the native Toolbox, resource loads, BlockMove: there is no other path,
//! the bytes are private). Each calls [`Hooks::before_write`] once with the stripped range, so a watcher sees ALL of them.
//! With no page watched and no journal open the cost is one predictable branch.
//!
//! The watch is a per-4-KiB-page GENERATION: [`GuestMem::watch_code_page`] marks a page as holding decoded code and
//! returns its generation; the first write to a marked page (or [`GuestMem::invalidate_code`], the guest's `icbi`)
//! bumps the generation and unmarks it, so further writes to that page cost nothing until code there is decoded again.
//! A cache entry is valid exactly while the page's generation equals the one it was decoded at.
//!
//! The watch's state is a `#[repr(C)]` [`CodeWatch`] (heirloom-mem-abi; CPU ABI v3 borrows it): the RAM's OWNER allocates its arrays,
//! sized to cover every page of RAM, and a BORROWED view (cpu-ppc.library, [`GuestMem::view`]) reads and writes the same
//! elements in place - so a write from either side invalidates code decoded on either side. Nobody but the owner resizes them.
use alloc::vec::Vec;
use heirloom_mem_abi::CodeWatch;

/// log2 of the watched page size (4 KiB, the PowerPC page).
pub const CODE_PAGE_SHIFT: u32 = heirloom_mem_abi::CODE_PAGE_SHIFT;

pub(crate) struct Hooks {
    /// The shared watch record (the owner's boxed one, or the one a view borrowed). Never null.
    pub(crate) watch: *mut CodeWatch,
    /// (stripped address, byte before the write), in write order.
    journal: Option<Vec<(u32, u8)>>,
}

impl Hooks {
    pub(crate) fn new(watch: *mut CodeWatch) -> Hooks { Hooks { watch, journal: None } }

    fn w(&self) -> &CodeWatch {
        // SAFETY: `watch` points at the live CodeWatch of this RAM (GuestMem keeps the owner's Box alive as long as itself; a
        // view's creator guarantees the descriptor outlives the view - GuestMem::view's contract).
        unsafe { &*self.watch }
    }
    fn w_mut(&mut self) -> &mut CodeWatch {
        // SAFETY: as in `w`; `&mut self` is the only path to the record through this GuestMem.
        unsafe { &mut *self.watch }
    }

    #[inline(always)]
    fn marked(&self, page: usize) -> bool {
        let w = self.w();
        if page / 64 >= w.marked_words as usize { return false; }
        // SAFETY: page / 64 < marked_words, the length of the owner-allocated `marked` array.
        unsafe { *w.marked.add(page / 64) & (1u64 << (page % 64)) != 0 }
    }

    /// Unmark `page` and bump its generation, if it is marked.
    fn hit(&mut self, page: usize) {
        if !self.marked(page) { return; }
        let w = self.w_mut();
        // SAFETY: `marked` checked page / 64 < marked_words; the owner sizes gen_pages >= marked pages (64 per word), checked here too.
        unsafe {
            *w.marked.add(page / 64) &= !(1u64 << (page % 64));
            if page < w.gen_pages as usize { *w.gen.add(page) = (*w.gen.add(page)).wrapping_add(1); }
        }
        w.bumps = w.bumps.wrapping_add(1);
    }

    #[inline(always)]
    pub(crate) fn before_write(&mut self, ram: &[u8], addr: u32, len: u32) {
        if len == 0 || (self.w().active == 0 && self.journal.is_none()) { return; }
        // the common case inline: no journal, one page, and that page not marked
        if self.journal.is_none() && (addr & ((1 << CODE_PAGE_SHIFT) - 1)) + len <= 1 << CODE_PAGE_SHIFT
            && !self.marked((addr >> CODE_PAGE_SHIFT) as usize) {
            return;
        }
        self.slow(ram, addr, len);
    }
    #[cold]
    fn slow(&mut self, ram: &[u8], addr: u32, len: u32) {
        let (first, last) = ((addr >> CODE_PAGE_SHIFT) as usize, ((addr + (len - 1)) >> CODE_PAGE_SHIFT) as usize);
        for p in first..=last { self.hit(p); }
        if let Some(j) = self.journal.as_mut() {
            j.extend((addr..addr + len).map(|a| (a, ram[a as usize])));
        }
    }
}

impl crate::GuestMem {
    /// Mark the page holding `addr` as holding decoded code; returns its generation. The generation changes at the
    /// next write to any byte of the page (from anyone) or [`invalidate_code`](Self::invalidate_code) there.
    /// A page outside the watch's coverage (only possible past the end of RAM, where nothing can be decoded) is not marked.
    pub fn watch_code_page(&mut self, addr: u32) -> u64 {
        let page = (self.strip(addr) >> CODE_PAGE_SHIFT) as usize;
        let w = self.hooks.w_mut();
        if page >= w.gen_pages as usize || page / 64 >= w.marked_words as usize { return 0; }
        // SAFETY: both indices are inside the owner-allocated arrays (checked above).
        unsafe {
            *w.marked.add(page / 64) |= 1 << (page % 64);
            w.active = 1;
            *w.gen.add(page)
        }
    }
    /// The generation of the page holding `addr` (0 for a page outside the watch).
    #[inline]
    pub fn code_gen(&self, addr: u32) -> u64 {
        let page = (self.strip(addr) >> CODE_PAGE_SHIFT) as usize;
        let w = self.hooks.w();
        // SAFETY: index checked against gen_pages, the length of the owner-allocated `gen` array.
        if page < w.gen_pages as usize { unsafe { *w.gen.add(page) } } else { 0 }
    }
    /// The total number of generation bumps so far: unchanged means no watched page was written.
    #[inline]
    pub fn code_bumps(&self) -> u64 { self.hooks.w().bumps }
    /// The guest's `icbi`: code decoded from the page holding `addr` is stale from now on (if the page is watched).
    pub fn invalidate_code(&mut self, addr: u32) {
        let page = (self.strip(addr) >> CODE_PAGE_SHIFT) as usize;
        self.hooks.hit(page);
    }

    /// Start recording the old value of every byte written (a lockstep comparison's undo log). Restarts an open one.
    pub fn journal_begin(&mut self) {
        let mut j = self.hooks.journal.take().unwrap_or_default();
        j.clear();
        self.hooks.journal = Some(j);
    }
    /// Stop recording; the (address, old byte) pairs in write order (empty if no journal was open).
    pub fn journal_end(&mut self) -> Vec<(u32, u8)> { self.hooks.journal.take().unwrap_or_default() }
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
