//! RESIDENT ARRAYS, the engine's half (heirloom-gl-abi `snapshot::Keep`, wire v4): a draw's client array whose guest bytes are
//! the ones the worker was last sent travels as an id, not as bytes. Warcraft III's gameplay draws ~800 arrays a frame and 771 of
//! them are byte for byte the previous frame's (292 of 313 KiB, WC3_ARRAY_CENSUS 2026-11-05): before, every one was read,
//! byte-swapped into the batch, carried by the ring and decoded again.
//!
//! WHAT PROVES THE BYTES THE SAME is the bytes: each kept array holds a copy of the guest bytes it was stored from (as they lie
//! in guest RAM, stride and all), and a later draw names it only when guest RAM still holds exactly those bytes. Every write
//! counts, whoever made it (the CPU's stores, the Toolbox, a file read) and whether or not the program flushed: an unflushed
//! write - APPLE_vertex_array_range leaves it undefined, and on the shared (AGP) storage a Mac's GL reads it anyway - is drawn as
//! on that hardware. A compare reads the source once and writes nothing; the copy path read it and wrote it byte-swapped into
//! the batch, the ring and the worker's decode.
//!
//! WHY NOT GUEST RAM's WRITE WATCH (measured 2026-11-05, h86 wc3budget): watching the source pages (heirloom-mem-flat32 watch.rs)
//! would prove "unchanged" without reading, but the watch is tier 2's code watch - every first write to a watched page makes the
//! JIT leave its run, re-read its store map and scan its code pages for stale code. WC3 writes pages beside its static arrays
//! every frame: callgrind priced the scan alone at 0.85-3.6 M host instructions a frame on the guest's core, more than the whole
//! capture this saves.
//!
//! The program's `glFlushVertexArrayRangeAPPLE` / `glVertexArrayRangeAPPLE` drop the arrays they cover (the next draw stores them).
//! An array smaller than [`MIN_KEPT`] is always copied (its id and compare would save little). An array found changed [`CHURN`]
//! draws in a row is VOLATILE - copied, not compared, for [`VOLATILE_SWAPS`] swaps (a dynamic array would pay the compare AND
//! the copy every frame). Ids are dense and reused: an entry not drawn for [`IDLE_SWAPS`] swaps is dropped and its id freed; the
//! next `Store` under that id replaces the worker's bytes.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ops::Range;
use heirloom_gl_abi::snapshot::{Descriptor, Keep};

/// The smallest array kept resident (bytes of guest memory it spans).
pub(crate) const MIN_KEPT: usize = 256;
/// Swaps an entry may go undrawn before its id is freed.
pub(crate) const IDLE_SWAPS: u64 = 64;
/// Draws in a row found changed that make an array volatile, and the swaps it then stays copied.
pub(crate) const CHURN: u8 = 2;
pub(crate) const VOLATILE_SWAPS: u64 = 256;

/// What makes two arrays' wire bytes the same: the guest bytes and how they are read.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    begin: u32,
    end: u32,
    stride: u32,
    size: u8,
    ty: u16,
}

struct Entry {
    id: u32,
    /// The guest bytes of the span when they were stored (empty: nothing the worker holds may be named).
    raw: Vec<u8>,
    used: u64,
    /// Draws in a row that found the bytes changed, and the swap it stays copied until (volatile).
    churn: u8,
    copy_until: u64,
}

/// One window's counts (the perf line's `| resident:`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResidentCounts {
    pub refs: u64,
    pub ref_bytes: u64,
    pub stores: u64,
    pub store_bytes: u64,
    /// Arrays found changed draw after draw and made volatile (copied, not compared).
    pub volatile: u64,
}

impl ResidentCounts {
    pub fn add(&mut self, o: &ResidentCounts) {
        (self.refs, self.ref_bytes, self.stores, self.store_bytes) = (self.refs + o.refs, self.ref_bytes + o.ref_bytes, self.stores + o.stores, self.store_bytes + o.store_bytes);
        self.volatile += o.volatile;
    }
}

#[derive(Default)]
pub(crate) struct Resident {
    /// Off: every array is copied (the A/B; `MacPpc::gl_resident`).
    pub(crate) off: bool,
    map: BTreeMap<Key, Entry>,
    free: Vec<u32>,
    next: u32,
    swaps: u64,
    /// The keys stored by the draw being encoded: forgotten if it did not encode.
    stored: Vec<Key>,
    pub(crate) counts: ResidentCounts,
}

impl Resident {
    /// How the array `d` spanning guest bytes `mem[span]` travels in the draw being encoded.
    pub(crate) fn keep(&mut self, mem: &[u8], d: &Descriptor, span: Range<usize>) -> Keep {
        if self.off || span.len() < MIN_KEPT {
            return Keep::Copy;
        }
        let (Ok(begin), Ok(end), Some(now)) = (u32::try_from(span.start), u32::try_from(span.end), mem.get(span.clone())) else { return Keep::Copy };
        let key = Key { begin, end, stride: d.stride, size: d.size, ty: d.ty as u16 };
        let swaps = self.swaps;
        let id = match self.map.get_mut(&key) {
            Some(e) => {
                e.used = swaps;
                if swaps < e.copy_until {
                    return Keep::Copy; // volatile
                }
                if !e.raw.is_empty() && e.raw == now {
                    e.churn = 0;
                    self.counts.refs += 1;
                    self.counts.ref_bytes += wire_len(d, &span);
                    return Keep::Ref(e.id);
                }
                if !e.raw.is_empty() {
                    e.churn += 1;
                    if e.churn >= CHURN {
                        (e.churn, e.copy_until) = (0, swaps + VOLATILE_SWAPS);
                        e.raw.clear(); // what the worker holds is stale: the next try stores again
                        self.counts.volatile += 1;
                        return Keep::Copy;
                    }
                }
                e.raw.clear();
                e.raw.extend_from_slice(now);
                e.id
            }
            None => {
                let id = self.free.pop().unwrap_or_else(|| { self.next += 1; self.next - 1 });
                self.map.insert(key, Entry { id, raw: now.to_vec(), used: swaps, churn: 0, copy_until: 0 });
                id
            }
        };
        self.stored.push(key);
        self.counts.stores += 1;
        self.counts.store_bytes += wire_len(d, &span);
        Keep::Store(id)
    }

    /// The draw encoded (`ok`), or it did not: then the worker never received what it stored, so those entries are dropped (a
    /// later draw stores them again).
    pub(crate) fn encoded(&mut self, ok: bool) {
        for key in core::mem::take(&mut self.stored) {
            if !ok {
                if let Some(e) = self.map.remove(&key) {
                    self.free.push(e.id);
                }
            }
        }
    }

    /// The program flushed (or redefined) its vertex array range `[at, at + len)`: what it covers is stored again.
    pub(crate) fn flushed(&mut self, at: u32, len: u32) {
        let end = at.saturating_add(len);
        let gone: Vec<Key> = self.map.keys().filter(|k| k.begin < end && at < k.end).copied().collect();
        for k in gone {
            if let Some(e) = self.map.remove(&k) {
                self.free.push(e.id);
            }
        }
    }

    /// A swap: every [`IDLE_SWAPS`] swaps the entries not drawn since the last sweep are dropped and their ids freed.
    pub(crate) fn swap(&mut self) {
        self.swaps += 1;
        if self.swaps % IDLE_SWAPS != 0 {
            return;
        }
        let since = self.swaps - IDLE_SWAPS;
        let free = &mut self.free;
        self.map.retain(|_, e| {
            let keep = e.used >= since;
            if !keep {
                free.push(e.id);
            }
            keep
        });
    }

    /// A new worker: it holds nothing, so nothing is named until stored again.
    pub(crate) fn reset(&mut self) {
        let off = self.off;
        *self = Resident { off, ..Resident::default() };
    }

    /// Entries held (ids live).
    pub(crate) fn len(&self) -> usize { self.map.len() }
}

/// The bytes the array puts on the wire (packed: the stride removed).
fn wire_len(d: &Descriptor, span: &Range<usize>) -> u64 {
    let vbytes = d.size as usize * d.ty.bytes();
    let stride = if d.stride == 0 { vbytes } else { d.stride as usize };
    let verts = (span.len() - vbytes) / stride.max(1) + 1;
    (verts * vbytes) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_gl_abi::snapshot::{ElemType, Role};

    fn d() -> Descriptor { Descriptor { role: Role::Vertex, unit: 0, offset: 0x2000, size: 3, ty: ElemType::F32, stride: 0 } }

    /// Stored, then named while guest RAM holds the same bytes; ANY change inside its span (here one byte, unflushed) makes the
    /// next draw store again; a change outside it changes nothing; a flush over it alone also stores again.
    #[test]
    fn an_array_is_named_only_while_guest_ram_holds_the_bytes_it_was_stored_with() {
        let mut m = vec![0u8; 0x10000];
        let mut r = Resident::default();
        let span = 0x2000..0x2000 + 12 * 600;
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Store(0));
        r.encoded(true);
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Ref(0), "nothing written: named");
        m[0x2000 + 7199] = 1;
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Store(0), "an unflushed write to its last byte: stored again");
        r.encoded(true);
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Ref(0));
        m[0x1FFF] = 1;
        m[0x2000 + 7200] = 1;
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Ref(0), "NEGATIVE CONTROL: writes outside its span change nothing");
        r.flushed(0x2100, 4);
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Store(0), "a flush over it: stored again");
        assert_eq!(r.keep(&m, &d(), 0x2000..0x2000 + 12), Keep::Copy, "a small array is copied");
    }

    /// An array changed between every pair of draws is made volatile after CHURN of them: copied, not compared, until
    /// VOLATILE_SWAPS swaps have passed; then it is stored again.
    #[test]
    fn an_array_changed_every_frame_is_copied_for_a_while() {
        let mut m = vec![0u8; 0x10000];
        let mut r = Resident::default();
        let span = 0x2000..0x3000;
        let mut kept = Vec::new();
        for i in 0..6u8 {
            kept.push(r.keep(&m, &d(), span.clone()));
            r.encoded(true);
            m[0x2004] = i + 1;
            r.swap();
        }
        assert_eq!(kept, [Keep::Store(0), Keep::Store(0), Keep::Copy, Keep::Copy, Keep::Copy, Keep::Copy]);
        for _ in 0..VOLATILE_SWAPS { r.swap(); }
        assert_eq!(r.keep(&m, &d(), span.clone()), Keep::Store(0), "tried again once its time is up");
    }

    /// A draw that did not encode never reached the worker: its stores are forgotten (never named), and their ids reused.
    #[test]
    fn a_store_the_worker_never_got_is_never_named() {
        let m = vec![0u8; 0x10000];
        let mut r = Resident::default();
        assert_eq!(r.keep(&m, &d(), 0x2000..0x3000), Keep::Store(0));
        r.encoded(false);
        assert_eq!(r.keep(&m, &d(), 0x2000..0x3000), Keep::Store(0), "stored again under the freed id, not named");
        r.encoded(true);
        assert_eq!(r.keep(&m, &d(), 0x2000..0x3000), Keep::Ref(0));
    }

    /// An entry not drawn for IDLE_SWAPS swaps frees its id; one drawn every frame keeps it.
    #[test]
    fn an_idle_array_gives_its_id_back() {
        let m = vec![0u8; 0x10000];
        let mut r = Resident::default();
        let (a, b) = (0x2000..0x3000, 0x8000..0x9000);
        assert_eq!((r.keep(&m, &d(), a.clone()), r.keep(&m, &d(), b.clone())), (Keep::Store(0), Keep::Store(1)));
        r.encoded(true);
        for _ in 0..2 * IDLE_SWAPS {
            assert_eq!(r.keep(&m, &d(), a.clone()), Keep::Ref(0));
            r.encoded(true);
            r.swap();
        }
        assert_eq!(r.len(), 1, "b went undrawn: dropped");
        assert_eq!(r.keep(&m, &d(), 0xC000..0xD000), Keep::Store(1), "its id is reused");
    }
}
