//! The shared-memory byte ring that carries frames from the guest engine to `heirloom-gl.engine` (single producer, single
//! consumer). Layout, little-endian, identical in the worker's C twin (`rgl_ring.h`):
//! `0 u32 magic | 4 u16 abi_version | 6 u16 0 | 8 u32 capacity | 12 u32 0 | 16 u64 head | 24 u64 tail | 64.. data`.
//! `head` and `tail` are TOTAL byte counts (never wrapped); the data index is `pos % capacity`.
//!
//! Guarantees: a stream, not a frame queue, so a frame larger than the ring still passes (the consumer reassembles
//! while the producer waits for space); the producer never overwrites unread bytes; `tail` is published with Release
//! after the bytes are written and `head` with Release after they are read; attach refuses a region with the wrong magic,
//! ABI version or a capacity that does not fit it.
use crate::wire::ABI_VERSION;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub const RING_MAGIC: u32 = 0x524C_4752; // "RGLR" little-endian
pub const RING_HEADER: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RingError {
    TooSmall,
    BadMagic,
    BadVersion(u16),
    /// The header's capacity does not fit the region it sits in (or is zero).
    BadCapacity,
    Misaligned,
}

/// A view over ring memory (owned elsewhere: a grant, a mapped file, or [`OwnedRing`]).
pub struct Ring {
    base: *mut u8,
    cap: usize,
}

// SAFETY: the view only touches the region through atomics (head/tail) and disjoint byte ranges that the SPSC protocol
// hands to exactly one side at a time; the pointer's validity is the constructor's contract.
unsafe impl Send for Ring {}

impl Ring {
    /// Write a fresh header into `base[..total]` and view it.
    ///
    /// # Safety
    /// `base` must point to `total` writable bytes, 8-byte aligned, valid for the life of the view, and not be viewed
    /// by another producer/consumer pair.
    pub unsafe fn init(base: *mut u8, total: usize) -> Result<Ring, RingError> {
        if total <= RING_HEADER {
            return Err(RingError::TooSmall);
        }
        if base as usize % 8 != 0 {
            return Err(RingError::Misaligned);
        }
        let cap = u32::try_from(total - RING_HEADER).map_err(|_| RingError::BadCapacity)?;
        // SAFETY: in bounds by the check above.
        unsafe {
            core::ptr::write_bytes(base, 0, RING_HEADER);
            core::ptr::copy_nonoverlapping(RING_MAGIC.to_le_bytes().as_ptr(), base, 4);
            core::ptr::copy_nonoverlapping(ABI_VERSION.to_le_bytes().as_ptr(), base.add(4), 2);
            core::ptr::copy_nonoverlapping(cap.to_le_bytes().as_ptr(), base.add(8), 4);
        }
        Ok(Ring { base, cap: cap as usize })
    }

    /// View a ring somebody else initialised, validating its header.
    ///
    /// # Safety
    /// `base` must point to `total` readable+writable bytes, 8-byte aligned, valid for the life of the view.
    pub unsafe fn attach(base: *mut u8, total: usize) -> Result<Ring, RingError> {
        if total <= RING_HEADER {
            return Err(RingError::TooSmall);
        }
        if base as usize % 8 != 0 {
            return Err(RingError::Misaligned);
        }
        // SAFETY: the first 64 bytes are in bounds.
        let h = unsafe { core::slice::from_raw_parts(base, RING_HEADER) };
        if u32::from_le_bytes([h[0], h[1], h[2], h[3]]) != RING_MAGIC {
            return Err(RingError::BadMagic);
        }
        let v = u16::from_le_bytes([h[4], h[5]]);
        if v != ABI_VERSION {
            return Err(RingError::BadVersion(v));
        }
        let cap = u32::from_le_bytes([h[8], h[9], h[10], h[11]]) as usize;
        if cap == 0 || cap > total - RING_HEADER {
            return Err(RingError::BadCapacity);
        }
        Ok(Ring { base, cap })
    }

    fn head(&self) -> &AtomicU64 {
        // SAFETY: offset 16 of an 8-aligned base is 8-aligned and inside the header.
        unsafe { AtomicU64::from_ptr(self.base.add(16).cast()) }
    }
    fn tail(&self) -> &AtomicU64 {
        // SAFETY: as `head`, offset 24.
        unsafe { AtomicU64::from_ptr(self.base.add(24).cast()) }
    }
    pub fn capacity(&self) -> usize {
        self.cap
    }
    /// Bytes written and not yet read.
    pub fn pending(&self) -> usize {
        (self.tail().load(Ordering::Acquire) - self.head().load(Ordering::Acquire)) as usize
    }

    /// Producer: copy as much of `bytes` as fits and return how many were taken (0 = full: wait and retry).
    pub fn push(&self, bytes: &[u8]) -> usize {
        let tail = self.tail().load(Ordering::Relaxed);
        let used = (tail - self.head().load(Ordering::Acquire)) as usize;
        let n = bytes.len().min(self.cap - used);
        let start = (tail % self.cap as u64) as usize;
        let first = n.min(self.cap - start);
        // SAFETY: [start, start+first) and [0, n-first) lie inside the data area and are unread bytes' complement
        // (space the consumer has released), so no concurrent reader touches them.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.add(RING_HEADER + start), first);
            core::ptr::copy_nonoverlapping(bytes.as_ptr().add(first), self.base.add(RING_HEADER), n - first);
        }
        self.tail().store(tail + n as u64, Ordering::Release);
        n
    }

    /// Consumer: append up to `max` pending bytes to `out`; returns how many.
    pub fn pop(&self, out: &mut Vec<u8>, max: usize) -> usize {
        let head = self.head().load(Ordering::Relaxed);
        let avail = (self.tail().load(Ordering::Acquire) - head) as usize;
        let n = avail.min(max);
        let start = (head % self.cap as u64) as usize;
        let first = n.min(self.cap - start);
        // SAFETY: [start, start+first) and [0, n-first) hold bytes the producer published with Release (tail >= head+n).
        unsafe {
            out.extend_from_slice(core::slice::from_raw_parts(self.base.add(RING_HEADER + start), first));
            out.extend_from_slice(core::slice::from_raw_parts(self.base.add(RING_HEADER), n - first));
        }
        self.head().store(head + n as u64, Ordering::Release);
        n
    }
}

/// Ring memory owned by a `Vec` (tests, host tools, a recorded ring image).
pub struct OwnedRing {
    mem: Vec<u64>,
}

impl OwnedRing {
    /// A ring with `capacity` data bytes.
    pub fn new(capacity: usize) -> Result<OwnedRing, RingError> {
        let words = (RING_HEADER + capacity).div_ceil(8);
        let mut mem = alloc::vec![0u64; words];
        // SAFETY: the Vec's buffer is 8-aligned, `words*8` bytes long, and outlives this call; we only write the header.
        unsafe { Ring::init(mem.as_mut_ptr().cast(), words * 8)? };
        Ok(OwnedRing { mem })
    }
    pub fn ring(&self) -> Ring {
        // SAFETY: initialised in `new`; the memory lives as long as `self`, which the borrow of `&self` bounds in practice
        // (callers must not use the view after dropping the OwnedRing).
        unsafe { Ring::attach(self.mem.as_ptr() as *mut u8, self.mem.len() * 8).expect("OwnedRing header is valid by construction") }
    }
    /// The whole region as bytes (header + data): a recorded ring image another process can map.
    pub fn image(&self) -> &[u8] {
        // SAFETY: `u64` has no padding; the slice covers exactly the Vec's initialised words.
        unsafe { core::slice::from_raw_parts(self.mem.as_ptr().cast(), self.mem.len() * 8) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_wrap_the_ring_and_arrive_in_order() {
        let o = OwnedRing::new(10).unwrap();
        let r = o.ring();
        let mut got = Vec::new();
        let mut sent = 0u8;
        for _ in 0..50 {
            let chunk: Vec<u8> = (0..7).map(|i| sent.wrapping_add(i)).collect();
            let n = r.push(&chunk);
            sent = sent.wrapping_add(n as u8);
            r.pop(&mut got, 4);
        }
        while r.pending() > 0 {
            r.pop(&mut got, 100);
        }
        assert!(got.iter().enumerate().all(|(i, b)| *b == i as u8), "a byte was lost, repeated or reordered");
    }

    #[test]
    fn the_producer_never_overwrites_unread_bytes() {
        let o = OwnedRing::new(8).unwrap();
        let r = o.ring();
        assert_eq!(r.push(&[1; 20]), 8);
        assert_eq!(r.push(&[2; 1]), 0, "full: the producer must wait");
        let mut out = Vec::new();
        r.pop(&mut out, 3);
        assert_eq!(r.push(&[3; 5]), 3);
        r.pop(&mut out, 100);
        assert_eq!(out, [1, 1, 1, 1, 1, 1, 1, 1, 3, 3, 3]);
    }

    #[test]
    fn a_stream_far_larger_than_the_ring_crosses_two_threads() {
        let o = OwnedRing::new(64).unwrap();
        let (p, c) = (o.ring(), o.ring());
        let total = 1 << 20;
        let t = std::thread::spawn(move || {
            let mut out = Vec::new();
            while out.len() < total {
                c.pop(&mut out, 1000);
                std::thread::yield_now();
            }
            out
        });
        let mut i = 0usize;
        while i < total {
            let end = (i + 100).min(total);
            let chunk: Vec<u8> = (i..end).map(|x| (x % 251) as u8).collect();
            let mut at = 0;
            while at < chunk.len() {
                at += p.push(&chunk[at..]);
                std::thread::yield_now();
            }
            i = end;
        }
        let out = t.join().unwrap();
        assert!(out.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
    }

    #[test]
    fn attach_refuses_a_region_that_is_not_this_ring() {
        let mut m = alloc::vec![0u64; 16];
        let p = m.as_mut_ptr().cast::<u8>();
        // SAFETY: `m` is 128 aligned bytes.
        unsafe {
            assert_eq!(Ring::attach(p, 128).err(), Some(RingError::BadMagic));
            let _ = Ring::init(p, 128).unwrap();
            assert!(Ring::attach(p, 128).is_ok());
            assert_eq!(Ring::attach(p, 100).err(), Some(RingError::BadCapacity), "capacity larger than the region");
            *p.add(4) = 9;
            assert_eq!(Ring::attach(p, 128).err(), Some(RingError::BadVersion(9)));
        }
        assert_eq!(unsafe { Ring::init(m.as_mut_ptr().cast(), 64) }.err(), Some(RingError::TooSmall));
    }
}
