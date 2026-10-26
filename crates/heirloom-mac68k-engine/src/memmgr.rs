//! Memory Manager basics: Ptr and Handle blocks over the shared guest [`Heap`].
//!
//! A Handle is a pointer to a *master pointer*, which points at the block. Master pointers live in 64-entry blocks
//! taken from the heap (MoreMasters); a Handle is the guest-visible address of its master pointer, so the classic
//! `**h` double dereference works, and the engine can move a block (SetHandleSize) and fix only the master. Every
//! flag the real Memory Manager keeps in the master pointer's high byte (locked, purgeable, resource) is kept
//! HOST-side in [`HBlock`], which is the authority (the guest cannot corrupt it); on a 24-bit machine
//! ([`MemMgr::sync_master`]) the same bits are MIRRORED into the master pointer's high byte, as Inside Macintosh:
//! Memory describes, because applications read `**h` and rely on the bus ignoring that byte. `HGetState` reads the host copy.
//! Nothing is ever compacted or purged: a purgeable block is merely flagged.
//!
//! Results are Mac OS error codes (the second half of each `Result`); the trap glue in [`crate::memtraps`] puts them
//! in D0 and in the MemErr low-memory global. A handle or pointer the manager does not own is `memWZErr`.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use heirloom_mem_flat32::heap::Heap;
use heirloom_mem_flat32::{GuestMem, MASK_32};

pub const MEM_FULL_ERR: i16 = -108;
pub const NIL_HANDLE_ERR: i16 = -109;
pub const MEM_WZ_ERR: i16 = -111;
pub const MEM_LOCKED_ERR: i16 = -117;
/// Master pointers per MoreMasters block.
pub const MASTERS_PER_BLOCK: u32 = 64;

/// HGetState/HSetState byte: the bits a real master pointer's high byte holds.
pub const LOCKED_BIT: u8 = 0x80;
pub const PURGEABLE_BIT: u8 = 0x40;
pub const RESOURCE_BIT: u8 = 0x20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HBlock { pub ptr: u32, pub size: u32, pub locked: bool, pub purgeable: bool, pub resource: bool }

impl HBlock {
    pub fn state(&self) -> u8 {
        (if self.locked { LOCKED_BIT } else { 0 }) | (if self.purgeable { PURGEABLE_BIT } else { 0 }) | (if self.resource { RESOURCE_BIT } else { 0 })
    }
}

#[derive(Default)]
pub struct MemMgr {
    free_masters: Vec<u32>,
    handles: BTreeMap<u32, HBlock>,
    ptrs: BTreeMap<u32, u32>,
}

/// The heap takes at least one byte so a zero-length block is still a distinct, freeable block.
fn heap_len(size: u32) -> u32 { size.max(1) }

impl MemMgr {
    pub fn live_handles(&self) -> usize { self.handles.len() }
    pub fn live_ptrs(&self) -> usize { self.ptrs.len() }
    pub fn block(&self, h: u32) -> Option<&HBlock> { self.handles.get(&h) }
    pub fn block_mut(&mut self, h: u32) -> Option<&mut HBlock> { self.handles.get_mut(&h) }

    /// Rewrite handle `h`'s master pointer: the block address, plus (24-bit addressing only) the lock/purge/resource
    /// bits in the high byte. A 32-bit machine's master pointer is the bare address (the Memory Manager there keeps
    /// the flags elsewhere), so the guest never sees stale flag bits after a mode change.
    pub fn sync_master(&self, mem: &mut GuestMem, h: u32) -> Result<(), i16> {
        let b = self.handles.get(&h).ok_or(MEM_WZ_ERR)?;
        let flags = if mem.addr_mask() == MASK_32 { 0 } else { u32::from(b.state()) << 24 };
        mem.write_u32(h, b.ptr | flags).map_err(|_| MEM_WZ_ERR)
    }

    /// Resynchronise every master pointer (the addressing mode just changed).
    pub fn sync_all_masters(&self, mem: &mut GuestMem) {
        for &h in self.handles.keys() { let _ = self.sync_master(mem, h); }
    }

    /// MoreMasters: add one block of master pointers.
    pub fn more_masters(&mut self, mem: &mut GuestMem, heap: &mut Heap) -> Result<(), i16> {
        let at = heap.alloc(MASTERS_PER_BLOCK * 4).ok_or(MEM_FULL_ERR)?;
        mem.zero(at, MASTERS_PER_BLOCK * 4).map_err(|_| MEM_WZ_ERR)?;
        // pushed in reverse so masters are handed out in ascending address order
        for i in (0..MASTERS_PER_BLOCK).rev() { self.free_masters.push(at + 4 * i); }
        Ok(())
    }

    fn alloc_block(heap: &mut Heap, mem: &mut GuestMem, size: u32, clear: bool) -> Result<u32, i16> {
        let p = heap.alloc(heap_len(size)).ok_or(MEM_FULL_ERR)?;
        if clear { mem.zero(p, heap_len(size)).map_err(|_| MEM_WZ_ERR)?; }
        Ok(p)
    }

    pub fn new_handle(&mut self, mem: &mut GuestMem, heap: &mut Heap, size: u32, clear: bool) -> Result<u32, i16> {
        if self.free_masters.is_empty() { self.more_masters(mem, heap)?; }
        let ptr = Self::alloc_block(heap, mem, size, clear)?;
        let h = self.free_masters.pop().expect("more_masters just filled the list");
        mem.write_u32(h, ptr).map_err(|_| MEM_WZ_ERR)?;
        self.handles.insert(h, HBlock { ptr, size, locked: false, purgeable: false, resource: false });
        Ok(h)
    }

    pub fn dispose_handle(&mut self, mem: &mut GuestMem, heap: &mut Heap, h: u32) -> Result<(), i16> {
        if h == 0 { return Err(NIL_HANDLE_ERR); }
        let b = self.handles.remove(&h).ok_or(MEM_WZ_ERR)?;
        heap.free(b.ptr, heap_len(b.size)).map_err(|_| MEM_WZ_ERR)?;
        let _ = mem.write_u32(h, 0);
        self.free_masters.push(h);
        Ok(())
    }

    pub fn handle_size(&self, h: u32) -> Result<u32, i16> {
        if h == 0 { return Err(NIL_HANDLE_ERR); }
        self.handles.get(&h).map(|b| b.size).ok_or(MEM_WZ_ERR)
    }

    /// SetHandleSize: the block moves (new block, copy, free old) so every Handle stays valid. A locked block must not
    /// move, so a locked handle can only be "resized" to its present size (`memLockedErr` otherwise).
    pub fn set_handle_size(&mut self, mem: &mut GuestMem, heap: &mut Heap, h: u32, size: u32) -> Result<(), i16> {
        if h == 0 { return Err(NIL_HANDLE_ERR); }
        let b = self.handles.get(&h).ok_or(MEM_WZ_ERR)?.clone();
        if size == b.size { return Ok(()); }
        if b.locked { return Err(MEM_LOCKED_ERR); }
        let new = heap.alloc(heap_len(size)).ok_or(MEM_FULL_ERR)?;
        let keep = b.size.min(size);
        let old = mem.bytes(mem.span(b.ptr, keep).map_err(|_| MEM_WZ_ERR)?).map_err(|_| MEM_WZ_ERR)?.to_vec();
        mem.zero(new, heap_len(size)).map_err(|_| MEM_WZ_ERR)?; // bytes past the old size are zero, deterministically
        mem.write_bytes(new, &old).map_err(|_| MEM_WZ_ERR)?;
        heap.free(b.ptr, heap_len(b.size)).map_err(|_| MEM_WZ_ERR)?;
        let e = self.handles.get_mut(&h).expect("checked above");
        e.ptr = new;
        e.size = size;
        self.sync_master(mem, h)
    }

    pub fn set_lock(&mut self, mem: &mut GuestMem, h: u32, locked: bool) -> Result<(), i16> {
        if h == 0 { return Err(NIL_HANDLE_ERR); }
        self.handles.get_mut(&h).ok_or(MEM_WZ_ERR)?.locked = locked;
        self.sync_master(mem, h)
    }

    pub fn state(&self, h: u32) -> Result<u8, i16> {
        if h == 0 { return Err(NIL_HANDLE_ERR); }
        self.handles.get(&h).map(HBlock::state).ok_or(MEM_WZ_ERR)
    }

    /// HSetState: restore the lock/purge/resource bits `state` carries.
    pub fn set_state(&mut self, mem: &mut GuestMem, h: u32, state: u8) -> Result<(), i16> {
        if h == 0 { return Err(NIL_HANDLE_ERR); }
        let b = self.handles.get_mut(&h).ok_or(MEM_WZ_ERR)?;
        b.locked = state & LOCKED_BIT != 0;
        b.purgeable = state & PURGEABLE_BIT != 0;
        b.resource = state & RESOURCE_BIT != 0;
        self.sync_master(mem, h)
    }

    pub fn new_ptr(&mut self, mem: &mut GuestMem, heap: &mut Heap, size: u32, clear: bool) -> Result<u32, i16> {
        let p = Self::alloc_block(heap, mem, size, clear)?;
        self.ptrs.insert(p, size);
        Ok(p)
    }

    pub fn dispose_ptr(&mut self, heap: &mut Heap, p: u32) -> Result<(), i16> {
        if p == 0 { return Ok(()); } // DisposePtr(NIL) is a no-op on a real Mac
        let size = self.ptrs.remove(&p).ok_or(MEM_WZ_ERR)?;
        heap.free(p, heap_len(size)).map_err(|_| MEM_WZ_ERR)
    }

    /// RecoverHandle: the handle whose block starts at `p`.
    pub fn recover_handle(&self, p: u32) -> Option<u32> { self.handles.iter().find(|(_, b)| b.ptr == p).map(|(&h, _)| h) }

    pub fn ptr_size(&self, p: u32) -> Result<u32, i16> { self.ptrs.get(&p).copied().ok_or(MEM_WZ_ERR) }
}

/// BlockMove: copy `n` bytes, correct for overlapping ranges (a real BlockMove is memmove).
pub fn block_move(mem: &mut GuestMem, src: u32, dst: u32, n: u32) -> Result<(), i16> {
    let tmp = mem.bytes(mem.span(src, n).map_err(|_| MEM_WZ_ERR)?).map_err(|_| MEM_WZ_ERR)?.to_vec();
    mem.write_bytes(dst, &tmp).map_err(|_| MEM_WZ_ERR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> (MemMgr, GuestMem, Heap) { (MemMgr::default(), GuestMem::new(0x10000), Heap::new(0x1000, 0x10000)) }

    #[test]
    fn a_handle_is_a_pointer_to_a_master_pointer_to_the_block() {
        let (mut mm, mut mem, mut heap) = world();
        let h = mm.new_handle(&mut mem, &mut heap, 10, true).unwrap();
        let p = mem.read_u32(h).unwrap();
        assert_eq!(p, mm.block(h).unwrap().ptr);
        mem.write_bytes(p, b"0123456789").unwrap();
        assert_eq!(mm.handle_size(h), Ok(10));
        // grow: the block moves, the handle does not, the contents follow
        mm.set_handle_size(&mut mem, &mut heap, h, 5000).unwrap();
        let p2 = mem.read_u32(h).unwrap();
        assert_ne!(p2, p, "the block moved");
        assert_eq!(mem.bytes(mem.span(p2, 10).unwrap()).unwrap(), b"0123456789");
        assert_eq!(mem.read_u8(p2 + 4999), Ok(0));
        mm.dispose_handle(&mut mem, &mut heap, h).unwrap();
        assert_eq!((mm.live_handles(), heap.live_blocks()), (0, 1), "only the master block is left");
        // NEGATIVE controls
        assert_eq!(mm.handle_size(h), Err(MEM_WZ_ERR), "a disposed handle is not a handle");
        assert_eq!(mm.dispose_handle(&mut mem, &mut heap, h), Err(MEM_WZ_ERR), "double dispose");
        assert_eq!(mm.handle_size(0), Err(NIL_HANDLE_ERR));
    }

    #[test]
    fn locked_blocks_do_not_move_and_state_round_trips() {
        let (mut mm, mut mem, mut heap) = world();
        let h = mm.new_handle(&mut mem, &mut heap, 16, false).unwrap();
        mm.set_lock(&mut mem, h, true).unwrap();
        assert_eq!(mm.state(h), Ok(LOCKED_BIT));
        assert_eq!(mm.set_handle_size(&mut mem, &mut heap, h, 64), Err(MEM_LOCKED_ERR));
        assert_eq!(mm.set_handle_size(&mut mem, &mut heap, h, 16), Ok(()), "same size does not move");
        let s = mm.state(h).unwrap();
        mm.set_lock(&mut mem, h, false).unwrap();
        mm.set_handle_size(&mut mem, &mut heap, h, 64).unwrap();
        mm.set_state(&mut mem, h, s).unwrap();
        assert!(mm.block(h).unwrap().locked, "HSetState restored the lock");
    }

    #[test]
    fn clear_zeroes_and_plain_does_not() {
        let (mut mm, mut mem, mut heap) = world();
        let p = mm.new_ptr(&mut mem, &mut heap, 32, false).unwrap();
        mem.write_bytes(p, &[0xEE; 32]).unwrap();
        mm.dispose_ptr(&mut heap, p).unwrap();
        let again = mm.new_ptr(&mut mem, &mut heap, 32, false).unwrap();
        assert_eq!((again, mem.read_u8(again).unwrap()), (p, 0xEE), "NewPtr leaves the old contents");
        mm.dispose_ptr(&mut heap, again).unwrap();
        let c = mm.new_ptr(&mut mem, &mut heap, 32, true).unwrap();
        assert_eq!((c, mem.read_u8(c).unwrap()), (p, 0), "NewPtrClear zeroes the same block");
        assert_eq!(mm.ptr_size(c), Ok(32));
        assert_eq!(mm.dispose_ptr(&mut heap, c + 8), Err(MEM_WZ_ERR), "an interior address is not a pointer");
        assert_eq!(mm.dispose_ptr(&mut heap, 0), Ok(()));
    }

    #[test]
    fn exhaustion_is_memfull_and_zero_length_blocks_are_distinct() {
        let (mut mm, mut mem, mut heap) = world();
        assert_eq!(mm.new_handle(&mut mem, &mut heap, 0x20000, false), Err(MEM_FULL_ERR));
        let a = mm.new_handle(&mut mem, &mut heap, 0, false).unwrap();
        let b = mm.new_handle(&mut mem, &mut heap, 0, false).unwrap();
        assert_ne!(mem.read_u32(a), mem.read_u32(b));
        assert_eq!(mm.handle_size(a), Ok(0));
        // 64 masters per block: the 65th handle triggers a second MoreMasters block
        let hs: Vec<u32> = (0..70).map(|_| mm.new_handle(&mut mem, &mut heap, 4, false).unwrap()).collect();
        assert_eq!(hs.len(), 70);
        assert_eq!(heap.live_blocks(), 2 + 2 + 70, "two master blocks, two empty blocks, seventy small ones");
    }

    #[test]
    fn block_move_is_correct_for_overlap_and_refuses_wild_ranges() {
        let mut mem = GuestMem::new(0x100);
        mem.write_bytes(0x10, b"abcdefgh").unwrap();
        block_move(&mut mem, 0x10, 0x12, 8).unwrap();
        assert_eq!(mem.bytes(mem.span(0x10, 10).unwrap()).unwrap(), b"ababcdefgh");
        assert_eq!(block_move(&mut mem, 0x10, 0xFFFF_0000, 4), Err(MEM_WZ_ERR));
        assert_eq!(block_move(&mut mem, 0xFFFF_0000, 0x10, 4), Err(MEM_WZ_ERR));
    }
}
