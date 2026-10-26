use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};

use crate::{Prot, PAGE_SIZE};

/// Guarantee: `pages` is sparse — only committed pages exist.
/// Guarantee: `generation` only increases, never wraps (checked_add → Trap).
pub struct Chunk {
    pub base: u32,
    pub max_size: u32,
    pub prot: Prot,
    pub generation: u64,
    pub pages: BTreeMap<u32, Box<[u8; PAGE_SIZE as usize]>>,
    pub guards: BTreeSet<u32>,
}

impl Chunk {
    pub fn new(base: u32, max_size: u32, prot: Prot) -> Self {
        Self {
            base,
            max_size,
            prot,
            generation: 0,
            pages: BTreeMap::new(),
            guards: BTreeSet::new(),
        }
    }
}