//! A map from guest pc to a value (tier 2's block map, tier 1's natives set): open addressing with linear probing over a power-of-two table, Fibonacci-hashed
//! word index, backward-shift deletion (no tombstones), grown x2 at half load. A lookup is one multiply and, at the load
//! kept, ~1.5 probes on a hit and ~2.5 on a miss, against a BTreeMap's ~log2(n) compares with a branch mispredict each
//! (the dispatcher looked every miss of its direct-mapped table up in a BTreeMap of every published block: 200k in a WC3
//! run). Growth is fallible (`try_reserve`): a map that cannot grow says so, and the JIT drops its code instead of
//! failing (a cache bounded by eviction, never a refusal).
use alloc::vec::Vec;

/// No guest instruction is at an odd address: the empty key.
const NONE: u32 = 1;
const MIN_BITS: u32 = 10;

pub(crate) struct PcMap<V: Copy> {
    keys: Vec<u32>,
    vals: Vec<V>,
    bits: u32,
    len: usize,
    zero: V,
}

impl<V: Copy> PcMap<V> {
    pub(crate) fn new(zero: V) -> PcMap<V> {
        let n = 1usize << MIN_BITS;
        PcMap { keys: alloc::vec![NONE; n], vals: alloc::vec![zero; n], bits: MIN_BITS, len: 0, zero }
    }
    #[inline(always)]
    fn home(&self, pc: u32) -> usize { ((pc >> 2).wrapping_mul(0x9E37_79B9) >> (32 - self.bits)) as usize }
    #[inline(always)]
    fn mask(&self) -> usize { self.keys.len() - 1 }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize { self.len }

    /// The slot holding `pc`, if any.
    #[inline(always)]
    fn find(&self, pc: u32) -> Option<usize> {
        let mut i = self.home(pc);
        loop {
            let k = self.keys[i];
            if k == pc { return Some(i); }
            if k == NONE { return None; }
            i = (i + 1) & self.mask();
        }
    }
    #[inline(always)]
    pub(crate) fn get(&self, pc: u32) -> Option<&V> { self.find(pc).map(|i| &self.vals[i]) }
    pub(crate) fn contains(&self, pc: u32) -> bool { self.find(pc).is_some() }

    /// Insert or replace; false when the table had to grow and could not (nothing changed then).
    pub(crate) fn insert(&mut self, pc: u32, v: V) -> bool {
        debug_assert!(pc != NONE);
        if let Some(i) = self.find(pc) { self.vals[i] = v; return true; }
        if 2 * (self.len + 1) > self.keys.len() && !self.grow() { return false; }
        self.put_new(pc, v);
        true
    }
    fn put_new(&mut self, pc: u32, v: V) {
        let mut i = self.home(pc);
        while self.keys[i] != NONE { i = (i + 1) & self.mask(); }
        self.keys[i] = pc;
        self.vals[i] = v;
        self.len += 1;
    }
    fn grow(&mut self) -> bool {
        let n = self.keys.len() * 2;
        let (mut keys, mut vals) = (Vec::new(), Vec::new());
        if keys.try_reserve_exact(n).is_err() || vals.try_reserve_exact(n).is_err() { return false; }
        keys.resize(n, NONE);
        vals.resize(n, self.zero);
        let old_keys = core::mem::replace(&mut self.keys, keys);
        let old_vals = core::mem::replace(&mut self.vals, vals);
        self.bits += 1;
        self.len = 0;
        for (k, v) in old_keys.into_iter().zip(old_vals) { if k != NONE { self.put_new(k, v); } }
        true
    }

    /// Remove `pc`; the entries after it in its probe run shift back so no lookup ever stops short.
    pub(crate) fn remove(&mut self, pc: u32) -> Option<V> {
        let mut i = self.find(pc)?;
        let v = self.vals[i];
        let mask = self.mask();
        let mut j = i;
        loop {
            j = (j + 1) & mask;
            let k = self.keys[j];
            if k == NONE { break; }
            let h = self.home(k);
            // the entry at j may move to the hole at i unless its home lies cyclically in (i, j]
            let stays = if i <= j { i < h && h <= j } else { i < h || h <= j };
            if !stays {
                self.keys[i] = k;
                self.vals[i] = self.vals[j];
                i = j;
            }
        }
        self.keys[i] = NONE;
        self.vals[i] = self.zero;
        self.len -= 1;
        Some(v)
    }

    /// Keep the entries `f` answers true for.
    pub(crate) fn retain(&mut self, mut f: impl FnMut(u32, &V) -> bool) {
        let gone: Vec<u32> = self.keys.iter().zip(&self.vals).filter(|(k, v)| **k != NONE && !f(**k, v)).map(|(k, _)| *k).collect();
        for k in gone { self.remove(k); }
    }

    pub(crate) fn clear(&mut self) {
        self.keys.fill(NONE);
        self.vals.fill(self.zero);
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::PcMap;
    use alloc::collections::BTreeMap;

    /// Random inserts / removes / lookups against a BTreeMap model, with keys crowded into few home slots.
    #[test]
    fn agrees_with_a_btreemap_under_random_inserts_and_removes() {
        let mut m = PcMap::new(0u64);
        let mut model = BTreeMap::new();
        let mut s = 0x2545_F491u32;
        for step in 0..200_000u32 {
            s ^= s << 13; s ^= s >> 17; s ^= s << 5;
            // 4096 distinct word addresses, spread so many share a home slot after the table grows
            let pc = (s % 4096) << 2 | ((s >> 20) & 0x3) << 24;
            match s >> 30 {
                0 | 1 => { assert!(m.insert(pc, step as u64)); model.insert(pc, step as u64); }
                2 => assert_eq!(m.remove(pc), model.remove(&pc)),
                _ => assert_eq!(m.get(pc).copied(), model.get(&pc).copied()),
            }
            if step % 50_000 == 0 { m.retain(|k, _| k & 4 == 0); model.retain(|k, _| k & 4 == 0); }
        }
        assert_eq!(m.len(), model.len());
        for (k, v) in &model { assert_eq!(m.get(*k), Some(v)); }
    }
}
