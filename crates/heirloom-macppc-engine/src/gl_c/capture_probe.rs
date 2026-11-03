//! The capture probe (h86 glcall9): on the Z83 a draw's capture - the guest's index and vertex bytes copied into the batch and
//! byte-swapped, ~2 host instructions a byte - runs at ~19,000 TSC cycles a KiB while the engine's heap copies at ~1,000. This
//! takes the capture apart ON THE MACHINE, on every [`crate::perf::CAPTURE_SAMPLE`]-th draw and once a window, with passes
//! that change nothing the guest or the worker sees: the batch's spare capacity is written (then overwritten by the capture
//! itself), a scratch buffer receives a second copy of the same draw, and guest RAM and a heap block are only READ.
//! [`crate::perf::CaptureProbe`] says what each pass means; [`crate::perf::capture_line`] prints them.
use alloc::vec::Vec;

/// Bytes of the batch's spare capacity a sampled draw writes before its capture (a WC3 draw is ~1.2 KB; 16 KiB covers the
/// draws the frame's bytes are in and stays inside the Atom's 24 KiB L1D).
pub(crate) const DST_TOUCH: usize = 16 << 10;
/// Bytes each read pass reads (a quarter of the Z8350's 1 MiB L2: a second pass reads them from L2 when they are cacheable).
pub(crate) const READ_BYTES: usize = 256 << 10;

/// The probe's own buffers, kept between windows (warm capacity: no allocation on a sampled draw once grown).
#[derive(Default)]
pub(crate) struct ProbeBufs {
    /// The second capture of a sampled draw.
    pub scratch: Vec<u8>,
    /// The heap block the read passes compare guest RAM with (allocated at the first window's pass).
    pub heap: Vec<u8>,
}

/// Write `n` bytes of `v`'s spare capacity (growing it first if needed; the growth is not part of what the caller times when
/// it calls [`reserve`] before) and return how many it wrote. The capture that follows overwrites them; `len` is unchanged.
pub(crate) fn touch_spare(v: &mut Vec<u8>, n: usize) -> usize {
    let s = v.spare_capacity_mut();
    let k = n.min(s.len());
    for b in &mut s[..k] {
        b.write(0);
    }
    core::hint::black_box(s.as_ptr());
    k
}

/// Grow `v`'s spare capacity to at least `n` bytes (outside any timed pass).
pub(crate) fn reserve(v: &mut Vec<u8>, n: usize) {
    v.reserve(n);
}

/// Read every 8 bytes of `b` once (the sum keeps the reads).
pub(crate) fn read_pass(b: &[u8]) -> u64 {
    let s = b.chunks_exact(8).fold(0u64, |a, c| a.wrapping_add(u64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])));
    core::hint::black_box(s)
}

/// The guest-RAM range the read passes take for a draw whose first array starts at `off`: [`READ_BYTES`] from `off`'s page,
/// moved down to fit inside RAM of `len` bytes; None when RAM is smaller than one pass.
pub(crate) fn ram_window(off: usize, len: usize) -> Option<core::ops::Range<usize>> {
    if len < READ_BYTES {
        return None;
    }
    let start = (off & !0xFFF).min(len - READ_BYTES);
    Some(start..start + READ_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spare_touch_writes_capacity_only_and_the_ram_window_stays_inside_ram() {
        let mut v = alloc::vec![7u8; 10];
        reserve(&mut v, 100);
        let k = touch_spare(&mut v, 64);
        assert_eq!((k, v.len()), (64, 10), "the touch leaves the batch's length and bytes alone");
        assert!(v.iter().all(|&b| b == 7));
        assert_eq!(touch_spare(&mut v, usize::MAX), v.capacity() - 10, "never past the capacity");
        assert_eq!(ram_window(0x12345, 1 << 20), Some(0x12000..0x12000 + READ_BYTES));
        assert_eq!(ram_window((1 << 20) - 10, 1 << 20), Some((1 << 20) - READ_BYTES..1 << 20), "moved down to end at RAM's end");
        assert_eq!(ram_window(0, READ_BYTES - 1), None);
        assert_eq!(read_pass(&[1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 9]), 3, "whole words only");
    }
}
