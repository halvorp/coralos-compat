//! Sparse paged little-endian 32-bit guest address space with chunk protections for the Symbian engine.
#![no_std]
extern crate alloc;

pub mod chunk;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;

pub const PAGE_SIZE: u32 = 4096;
pub const PAGE_MASK: u32 = PAGE_SIZE - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prot {
    pub r: bool,
    pub w: bool,
    pub x: bool,
}

impl Prot {
    pub const RW: Self = Prot {
        r: true,
        w: true,
        x: false,
    };
    pub const RX: Self = Prot {
        r: true,
        w: false,
        x: true,
    };
    pub const R: Self = Prot {
        r: true,
        w: false,
        x: false,
    };

    /// Guarantee: W^X invariant — w and x are never simultaneously true.
    pub fn is_valid(self) -> bool {
        !(self.w && self.x)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ChunkId(u32);

#[derive(Debug, PartialEq, Eq)]
pub enum VmError {
    Overlap,
    NotPageAligned,
    InvalidProt,
    Fault(u32),
    WriteToExec(u32),
    ExecOfWrite(u32),
    PermissionFault(u32),
    BudgetExceeded,
    Trap,
    InvalidChunkId,
    OutOfBounds,
}

pub struct Vm {
    chunks: BTreeMap<ChunkId, chunk::Chunk>,
    base_to_id: BTreeMap<u32, ChunkId>,
    next_id: u32,
    committed_bytes: u64,
    budget: u64,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            chunks: BTreeMap::new(),
            base_to_id: BTreeMap::new(),
            next_id: 0,
            committed_bytes: 0,
            budget: u64::MAX,
        }
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
    }

    pub fn committed_bytes(&self) -> u64 {
        self.committed_bytes
    }

    pub fn generation(&self, id: ChunkId) -> Result<u64, VmError> {
        self.chunks
            .get(&id)
            .map(|chunk| chunk.generation)
            .ok_or(VmError::InvalidChunkId)
    }

    pub fn create_chunk(
        &mut self,
        base: u32,
        max_size: u32,
        prot: Prot,
    ) -> Result<ChunkId, VmError> {
        if base & PAGE_MASK != 0 || max_size & PAGE_MASK != 0 {
            return Err(VmError::NotPageAligned);
        }
        let end = base.checked_add(max_size).ok_or(VmError::Trap)?;
        if !prot.is_valid() {
            return Err(VmError::InvalidProt);
        }

        if let Some((&left_base, left_id)) = self.base_to_id.range(..=base).next_back() {
            let left = self.chunks.get(left_id).ok_or(VmError::Trap)?;
            if left_base + left.max_size > base {
                return Err(VmError::Overlap);
            }
        }
        if let Some((&right_base, _)) = self.base_to_id.range(base..).next() {
            if end > right_base {
                return Err(VmError::Overlap);
            }
        }

        let id = ChunkId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(VmError::Trap)?;
        self.chunks.insert(id, chunk::Chunk::new(base, max_size, prot));
        self.base_to_id.insert(base, id);
        Ok(id)
    }

    pub fn commit(&mut self, id: ChunkId, offset: u32, len: u32) -> Result<(), VmError> {
        let max_size = self
            .chunks
            .get(&id)
            .ok_or(VmError::InvalidChunkId)?
            .max_size;
        Self::validate_page_range(offset, len, max_size)?;
        let end = offset.checked_add(len).ok_or(VmError::OutOfBounds)?;
        let chunk = self.chunks.get_mut(&id).ok_or(VmError::InvalidChunkId)?;

        let mut page = offset;
        while page < end {
            let page_addr = chunk.base + page;
            if chunk.pages.contains_key(&page_addr) {
                page += PAGE_SIZE;
                continue;
            }
            if !chunk.guards.remove(&page_addr)
                && self.committed_bytes + u64::from(PAGE_SIZE) > self.budget
            {
                return Err(VmError::BudgetExceeded);
            }
            chunk
                .pages
                .insert(page_addr, Box::new([0; PAGE_SIZE as usize]));
            self.committed_bytes += u64::from(PAGE_SIZE);
            page += PAGE_SIZE;
        }
        Ok(())
    }

    pub fn decommit(&mut self, id: ChunkId, offset: u32, len: u32) -> Result<(), VmError> {
        let max_size = self
            .chunks
            .get(&id)
            .ok_or(VmError::InvalidChunkId)?
            .max_size;
        Self::validate_page_range(offset, len, max_size)?;
        let end = offset.checked_add(len).ok_or(VmError::OutOfBounds)?;
        let chunk = self.chunks.get_mut(&id).ok_or(VmError::InvalidChunkId)?;

        let mut page = offset;
        while page < end {
            let page_addr = chunk.base + page;
            if chunk.pages.remove(&page_addr).is_some() {
                self.committed_bytes -= u64::from(PAGE_SIZE);
            }
            chunk.guards.insert(page_addr);
            page += PAGE_SIZE;
        }
        Ok(())
    }

    pub fn set_prot(&mut self, id: ChunkId, prot: Prot) -> Result<(), VmError> {
        let chunk = self.chunks.get_mut(&id).ok_or(VmError::InvalidChunkId)?;
        if !prot.is_valid() {
            return Err(VmError::InvalidProt);
        }
        chunk.prot = prot;
        Ok(())
    }

    pub fn read_u8(&self, addr: u32) -> Result<u8, VmError> {
        let id = self.prepare_read(addr, 1)?;
        let chunk = self.chunks.get(&id).ok_or(VmError::Trap)?;
        Ok(chunk.pages[&(addr & !PAGE_MASK)][(addr & PAGE_MASK) as usize])
    }

    pub fn read_u16(&self, addr: u32) -> Result<u16, VmError> {
        let mut bytes = [0; 2];
        self.read_bytes(addr, &mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn read_u32(&self, addr: u32) -> Result<u32, VmError> {
        let mut bytes = [0; 4];
        self.read_bytes(addr, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn write_u8(&mut self, addr: u32, val: u8) -> Result<(), VmError> {
        self.write_bytes(addr, &[val])
    }

    pub fn write_u16(&mut self, addr: u32, val: u16) -> Result<(), VmError> {
        self.write_bytes(addr, &val.to_le_bytes())
    }

    pub fn write_u32(&mut self, addr: u32, val: u32) -> Result<(), VmError> {
        self.write_bytes(addr, &val.to_le_bytes())
    }

    pub fn read_bytes(&self, addr: u32, buf: &mut [u8]) -> Result<(), VmError> {
        if buf.is_empty() {
            return Ok(());
        }
        let len = u32::try_from(buf.len()).map_err(|_| VmError::Fault(addr))?;
        let id = self.prepare_read(addr, len)?;
        let chunk = self.chunks.get(&id).ok_or(VmError::Trap)?;

        for (index, byte) in buf.iter_mut().enumerate() {
            let current = addr + index as u32;
            *byte = chunk.pages[&(current & !PAGE_MASK)][(current & PAGE_MASK) as usize];
        }
        Ok(())
    }

    pub fn write_bytes(&mut self, addr: u32, data: &[u8]) -> Result<(), VmError> {
        if data.is_empty() {
            return Ok(());
        }
        let len = u32::try_from(data.len()).map_err(|_| VmError::Fault(addr))?;
        let id = self.prepare_write(addr, len)?;
        let chunk = self.chunks.get_mut(&id).ok_or(VmError::Trap)?;

        for (index, &byte) in data.iter().enumerate() {
            let current = addr + index as u32;
            chunk.pages.get_mut(&(current & !PAGE_MASK)).ok_or(VmError::Trap)?
                [(current & PAGE_MASK) as usize] = byte;
        }
        Ok(())
    }

    pub fn fetch_u32(&self, addr: u32) -> Result<u32, VmError> {
        let id = self.locate(addr, 4)?;
        let chunk = self.chunks.get(&id).ok_or(VmError::Trap)?;
        if !chunk.prot.r {
            return Err(VmError::PermissionFault(addr));
        }
        if !chunk.prot.x {
            return Err(VmError::ExecOfWrite(addr));
        }
        self.check_pages(chunk, addr, 4)?;

        let mut bytes = [0; 4];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let current = addr + index as u32;
            *byte = chunk.pages[&(current & !PAGE_MASK)][(current & PAGE_MASK) as usize];
        }
        Ok(u32::from_le_bytes(bytes))
    }

    fn validate_page_range(offset: u32, len: u32, max_size: u32) -> Result<(), VmError> {
        if offset & PAGE_MASK != 0 || len & PAGE_MASK != 0 {
            return Err(VmError::NotPageAligned);
        }
        match offset.checked_add(len) {
            Some(end) if end <= max_size => Ok(()),
            _ => Err(VmError::OutOfBounds),
        }
    }

    fn locate(&self, addr: u32, len: u32) -> Result<ChunkId, VmError> {
        let end = addr
            .checked_add(len - 1)
            .ok_or(VmError::Fault(addr))?;
        let (&_, &id) = self
            .base_to_id
            .range(..=addr)
            .next_back()
            .ok_or(VmError::Fault(addr))?;
        let chunk = self.chunks.get(&id).ok_or(VmError::Trap)?;
        if end >= chunk.base + chunk.max_size {
            return Err(VmError::Fault(addr));
        }
        Ok(id)
    }

    fn check_pages(&self, chunk: &chunk::Chunk, addr: u32, len: u32) -> Result<(), VmError> {
        let last = addr + len - 1;
        let mut page = addr & !PAGE_MASK;
        let last_page = last & !PAGE_MASK;
        loop {
            if !chunk.pages.contains_key(&page) {
                return Err(VmError::Fault(addr));
            }
            if page == last_page {
                break;
            }
            page += PAGE_SIZE;
        }
        Ok(())
    }

    fn prepare_read(&self, addr: u32, len: u32) -> Result<ChunkId, VmError> {
        let id = self.locate(addr, len)?;
        let chunk = self.chunks.get(&id).ok_or(VmError::Trap)?;
        self.check_pages(chunk, addr, len)?;
        if !chunk.prot.r {
            return Err(VmError::PermissionFault(addr));
        }
        Ok(id)
    }

    /// Validate every page before the first write, so cross-page accesses are atomic on faults.
    fn prepare_write(&mut self, addr: u32, len: u32) -> Result<ChunkId, VmError> {
        let id = self.locate(addr, len)?;
        let chunk = self.chunks.get(&id).ok_or(VmError::Trap)?;
        self.check_pages(chunk, addr, len)?;
        if !chunk.prot.x && !chunk.prot.w {
            return Err(VmError::PermissionFault(addr));
        }
        if chunk.prot.x {
            let chunk = self.chunks.get_mut(&id).ok_or(VmError::Trap)?;
            chunk.generation = chunk.generation.checked_add(1).ok_or(VmError::Trap)?;
            return Err(VmError::WriteToExec(addr));
        }
        Ok(id)
    }
}