#![cfg_attr(test, allow(unused))] // for unit tests
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use heirloom_cpu_arm::Memory;
use heirloom_mem_paged32::{ChunkId, VmError};
use crate::layout;
use crate::mem::GuestMem;

#[derive(Debug)]
pub enum HeapError {
    UnknownAddress(u32),
}

pub struct Heap {
    heap_id: ChunkId,
    base: u32,
    max: u32,
    committed_end: u32,
    live: BTreeMap<u32, u32>,
    free_list: Vec<(u32, u32)>, // sorted by addr
}

impl Heap {
    const ALIGN: u32 = 8;

    pub fn new(heap_id: ChunkId, base: u32, max: u32, already_committed: u32) -> Self {
        let mut free_list = Vec::new();
        if already_committed > 0 {
            free_list.push((base, already_committed));
        }
        Self {
            heap_id,
            base,
            max,
            committed_end: base + already_committed,
            live: BTreeMap::new(),
            free_list,
        }
    }

    fn align_up(n: u32) -> Option<u32> {
        Some(n.checked_add(Self::ALIGN - 1)? & !(Self::ALIGN - 1))
    }

    fn ensure_committed(&mut self, end: u32, mem: &mut GuestMem) -> Result<(), VmError> {
        if end <= self.committed_end {
            return Ok(());
        }
        let reservation = layout::pru(self.max);
        let target = self.base + layout::pru(end - self.base).min(reservation);
        if target <= self.committed_end {
            return Ok(());
        }
        let offset = self.committed_end - self.base;
        let len = target - self.committed_end;
        mem.vm.commit(self.heap_id, offset, len)?;
        self.committed_end = target;
        Ok(())
    }

    fn insert_free(&mut self, addr: u32, len: u32) {
        if len == 0 {
            return;
        }
        let mut entry = (addr, len);
        let mut i = 0;
        while i < self.free_list.len() && self.free_list[i].0 < entry.0 {
            i += 1;
        }
        // Merge with the right neighbor, then with the left one.
        if i < self.free_list.len() && self.free_list[i].0 == entry.0 + entry.1 {
            entry.1 += self.free_list[i].1;
            self.free_list.remove(i);
        }
        if i > 0 && self.free_list[i - 1].0 + self.free_list[i - 1].1 == entry.0 {
            self.free_list[i - 1].1 += entry.1;
        } else {
            self.free_list.insert(i, entry);
        }
    }

    pub fn alloc(&mut self, len: u32, mem: &mut GuestMem) -> Option<u32> {
        // Guard against guest passing huge/negative lengths.
        let len = Self::align_up(len)?;
        let len = if len == 0 { Self::ALIGN } else { len };
        if len > self.max {
            return None;
        }

        // First fit.
        let mut found = None;
        for (i, &(_, hole_len)) in self.free_list.iter().enumerate() {
            if hole_len >= len {
                found = Some(i);
                break;
            }
        }
        let idx = found?;
        let (addr, hole_len) = self.free_list[idx];
        self.ensure_committed(addr.checked_add(len)?, mem).ok()?;
        self.live.insert(addr, len);
        if hole_len > len {
            self.free_list[idx] = (addr + len, hole_len - len);
        } else {
            self.free_list.swap_remove(idx);
        }
        Some(addr)
    }

    pub fn free(&mut self, addr: u32) -> Result<(), HeapError> {
        let len = self.live.remove(&addr).ok_or(HeapError::UnknownAddress(addr))?;
        self.insert_free(addr, len);
        Ok(())
    }

    pub fn realloc(&mut self, addr: u32, new_len: u32, mem: &mut GuestMem) -> Option<u32> {
        let old_len = *self.live.get(&addr)?;
        let new_len = if new_len == 0 {
            Self::ALIGN
        } else {
            Self::align_up(new_len)?
        };
        if new_len <= old_len {
            self.live.insert(addr, new_len);
            if old_len > new_len {
                self.insert_free(addr + new_len, old_len - new_len);
            }
            return Some(addr);
        }

        // Allocate and copy before freeing the old block: the new allocation may reuse its address range.
        let new_addr = self.alloc(new_len, mem)?;
        let copy = old_len.min(new_len);
        for i in 0..copy {
            if let Ok(b) = mem.read_u8(addr + i) {
                let _ = mem.write_u8(new_addr + i, b);
            }
        }
        let _ = self.free(addr);
        Some(new_addr)
    }

    pub fn len_of(&self, addr: u32) -> Option<u32> {
        self.live.get(&addr).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::GuestMem;

    fn make_heap(max: u32) -> (Heap, GuestMem) {
        let mut mem = GuestMem::new();
        let heap_id = mem.vm.create_chunk(
            layout::HEAP_BASE,
            layout::pru(max),
            heirloom_mem_paged32::Prot::RW,
        ).unwrap();
        mem.vm.commit(heap_id, 0, layout::pru(max)).unwrap();
        let heap = Heap::new(heap_id, layout::HEAP_BASE, max, max);
        (heap, mem)
    }

    #[test]
    fn alloc_free_reuse() {
        let (mut h, mut mem) = make_heap(0x10000);
        let a = h.alloc(64, &mut mem).unwrap();
        h.free(a).unwrap();
        let b = h.alloc(64, &mut mem).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn realloc_grows_preserves_bytes() {
        let (mut h, mut mem) = make_heap(0x10000);
        let a = h.alloc(16, &mut mem).unwrap();
        mem.write_u8(a, 0xAB).unwrap();
        mem.write_u8(a + 1, 0xCD).unwrap();
        let b = h.realloc(a, 64, &mut mem).unwrap();
        assert_eq!(mem.read_u8(b).unwrap(), 0xAB);
        assert_eq!(mem.read_u8(b + 1).unwrap(), 0xCD);
    }

    #[test]
    fn free_bad_ptr() {
        let (mut h, _mem) = make_heap(0x10000);
        assert!(h.free(0x12345678).is_err());
    }

    #[test]
    fn exhaustion_at_max() {
        let max = 0x10000;
        let (mut h, mut mem) = make_heap(max);
        let mut total = 0u32;
        loop {
            match h.alloc(0x1000, &mut mem) {
                Some(_) => total += 0x1000,
                None => break,
            }
        }
        assert!(total <= max);
    }
}