//! Tier 1's decoded-block cache: blocks keyed by guest pc, valid while the code page's write generation is the one
//! they were decoded at (heirloom-mem-flat32's code watch: every write to the page, from the guest or from the host,
//! and `icbi` change it) and while the CPU's set of bound native addresses is the one they were decoded against.
//!
//! A block starts at a pc that is not a native hook and runs to the first terminal instruction (a branch; `sc` is
//! always a block of its own), the end of its 4 KiB page, the instruction before a bound native address, or the
//! instruction before one that cannot be fetched. The table is direct-mapped (a colliding block replaces the old
//! one) and the decoded instructions live in one arena that is dropped whole when it passes [`ARENA_OPS`]: a cache,
//! bounded by eviction, never a refusal.
use crate::cpu::{Cpu, Trap};
use crate::t1op::{decode, Op, K};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use heirloom_mem_flat32::{GuestMem, CODE_PAGE_SHIFT};

const TABLE_BITS: u32 = 16;
/// Decoded instructions kept before the whole cache is dropped and refilled (16 bytes each: 16 MiB).
pub(crate) const ARENA_OPS: usize = 1 << 20;
const PAGE: u32 = 1 << CODE_PAGE_SHIFT;

#[derive(Clone, Copy, Default)]
struct Slot { pc: u32, len: u32, start: u32, gen: u64 }

/// Where execution at a pc goes.
pub(crate) enum Found {
    Block { start: usize, len: usize, gen: u64 },
    Native,
    /// A VOLATILE page (its code keeps being rewritten after it was decoded): run tier 0 while pc stays in it.
    Interp,
}

/// Stale lookups on one page after which it is volatile (decoding it again would cost more than interpreting it).
pub(crate) const VOLATILE_AFTER: u32 = 64;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Blocks decoded (first time, after an eviction or after their page changed).
    pub decodes: u64,
    /// Lookups that found a block whose page had been written since it was decoded.
    pub stale: u64,
    /// Whole-cache drops (arena full, or the bound natives changed).
    pub flushes: u64,
    /// Pages found volatile (rewritten again and again after decoding): tier 0 runs them.
    pub volatile_pages: u64,
    /// Instructions tier 1 handed to tier 0 on volatile pages.
    pub interp_insns: u64,
}

pub(crate) struct Cache {
    table: Vec<Slot>,
    pub(crate) ops: Vec<Op>,
    /// The natives set the blocks were cut against, and versions of it known to be equal to it.
    natives: Arc<BTreeSet<u32>>,
    natives_gens: Vec<u64>,
    pub(crate) stats: CacheStats,
    /// Stale lookups per page, and the pages found volatile.
    stale_pages: BTreeMap<u32, u32>,
    volatile: BTreeSet<u32>,
    /// NEGATIVE-CONTROL MUTANT: ignore page generations (a write to code never invalidates).
    pub(crate) ignore_generations: bool,
}

impl Cache {
    pub(crate) fn new() -> Cache {
        Cache { table: vec![Slot::default(); 1 << TABLE_BITS], ops: Vec::with_capacity(4096), natives: Arc::new(BTreeSet::new()), natives_gens: vec![0], stats: CacheStats::default(), stale_pages: BTreeMap::new(), volatile: BTreeSet::new(), ignore_generations: false }
    }

    fn flush(&mut self) {
        self.table.fill(Slot::default());
        self.ops.clear();
        self.stats.flushes += 1;
    }

    /// Make the blocks agree with `cpu`'s bound natives (a context switch may have swapped the set). Cheap when the
    /// version is one already seen; a changed set drops every block.
    #[inline]
    pub(crate) fn sync_natives(&mut self, cpu: &Cpu) {
        if self.natives_gens.contains(&cpu.natives_gen) { return; }
        if !Arc::ptr_eq(&self.natives, &cpu.natives) && self.natives != cpu.natives {
            self.flush();
            self.natives = cpu.natives.clone();
            self.natives_gens.clear();
        }
        if self.natives_gens.len() >= 16 { self.natives_gens.remove(0); }
        self.natives_gens.push(cpu.natives_gen);
    }

    /// The valid block at `pc` if one is cached: (start, len, gen). The hot path.
    #[inline(always)]
    pub(crate) fn hit(&self, mem: &GuestMem, pc: u32) -> Option<(usize, usize, u64)> {
        let s = &self.table[(pc >> 2) as usize & ((1 << TABLE_BITS) - 1)];
        if s.pc == pc && s.len != 0 && (s.gen == mem.code_gen(pc) || self.ignore_generations) { Some((s.start as usize, s.len as usize, s.gen)) } else { None }
    }

    /// The block at `pc` (decoding it if needed), or `Native`. A first instruction that cannot be fetched is the
    /// trap tier 0 gives.
    pub(crate) fn find(&mut self, mem: &mut GuestMem, pc: u32) -> Result<Found, Trap> {
        if let Some((start, len, gen)) = self.hit(mem, pc) { return Ok(Found::Block { start, len, gen }); }
        self.miss(mem, pc)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn miss(&mut self, mem: &mut GuestMem, pc: u32) -> Result<Found, Trap> {
        let idx = (pc >> 2) as usize & ((1 << TABLE_BITS) - 1);
        let s = self.table[idx];
        let page = mem.strip(pc) >> CODE_PAGE_SHIFT;
        if s.len != 0 && s.pc == pc {
            self.stats.stale += 1;
            let c = self.stale_pages.entry(page).or_default();
            *c += 1;
            if *c == VOLATILE_AFTER { self.volatile.insert(page); self.stats.volatile_pages += 1; }
        }
        if self.natives.contains(&pc) { return Ok(Found::Native); }
        if !self.volatile.is_empty() && self.volatile.contains(&page) { return Ok(Found::Interp); }
        self.decode(mem, pc, idx)
    }

    fn decode(&mut self, mem: &mut GuestMem, pc: u32, idx: usize) -> Result<Found, Trap> {
        if self.ops.len() + (PAGE / 4) as usize > ARENA_OPS { self.flush(); }
        let gen = mem.watch_code_page(pc);
        let page_end = (pc | (PAGE - 1)).wrapping_add(1);
        // the first bound native after pc on this page ends the block before it
        let stop = self.natives.range(pc.wrapping_add(4)..).next().copied().filter(|&n| n < page_end || page_end == 0).unwrap_or(page_end);
        let start = self.ops.len();
        let mut a = pc;
        loop {
            let Ok(word) = mem.read_u32(a) else {
                if a == pc { return Err(Trap::BusFault { pc, addr: pc, write: false }); }
                break;
            };
            let op = decode(a, word);
            if op.k == K::Sc && a != pc { break; } // sc runs alone: it calls the host
            self.ops.push(op);
            a = a.wrapping_add(4);
            if op.terminal() || a == stop || a == 0 { break; }
        }
        let len = self.ops.len() - start;
        self.table[idx] = Slot { pc, len: len as u32, start: start as u32, gen };
        self.stats.decodes += 1;
        Ok(Found::Block { start, len, gen })
    }
}
