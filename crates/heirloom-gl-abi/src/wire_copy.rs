//! Bytes appended to a ring frame with no `memcpy` call: guest arrays (big-endian components) copied and converted to the wire's
//! little-endian order in ONE pass, fixed-size headers and records as inline stores, call arguments as one store a word.
//!
//! Why its own module: on CoralOS a loaded class's `memcpy` is its opener's - compiler_builtins' `mem` family on
//! x86_64-unknown-none: REP MOVSB + REP MOVSQ + REP MOVSB, three microcoded string operations for every call whatever the length
//! (macppc.engine's memcpy) - and at opt-level "s" `Vec::extend_from_slice` is an out-of-line call into it, even for a 4-byte
//! field. The capture made ~5 such calls a draw (header, indices, then each array's record and data) followed by a second
//! pass to swap the bytes, and a GL call 1 + one per argument; a host test binary links its libc's memcpy instead, so no host
//! measurement saw them. Here every run is 8-byte loads, a register byte swap and 8-byte stores, the source range proved once
//! per array and the destination reserved once.
use crate::snapshot::SnapshotError;
use alloc::vec::Vec;

/// Append `count` runs of `run` bytes - the i-th starting at `src[begin + i * stride]` - to `out`, every `elem`-byte component
/// reversed (guest big-endian to wire little-endian; `elem` 1 copies as is). The bytes are those of copying each run and then
/// reversing each component. The whole range (first run's start to the last run's end) is checked against `src` before
/// anything is written; on an error `out` is unchanged.
pub(crate) fn append_swapped(out: &mut Vec<u8>, src: &[u8], begin: usize, stride: usize, run: usize, count: usize, elem: usize) -> Result<(), SnapshotError> {
    if count == 0 || run == 0 {
        return Ok(());
    }
    let total = count.checked_mul(run).ok_or(SnapshotError::Overflow)?;
    let end = (count - 1).checked_mul(stride).and_then(|o| o.checked_add(begin)).and_then(|o| o.checked_add(run)).ok_or(SnapshotError::Overflow)?;
    if end > src.len() {
        return Err(SnapshotError::OutOfRange);
    }
    debug_assert!(matches!(elem, 1 | 2 | 4 | 8) && run % elem == 0, "a run is whole components of 1, 2, 4 or 8 bytes");
    out.reserve(total);
    let len = out.len();
    // SAFETY: `out` has room for `total` more bytes (reserved above) and they are all written below before `set_len`: `count`
    // runs of `run` bytes at `d + i * run`. Every source read lies in `src[begin .. end]`, proved inside `src` above (the i-th
    // run reads `src[begin + i * stride ..][.. run]`, the last one ending at `end`). `src` and `out` are distinct borrows, so
    // the two never overlap.
    unsafe {
        let d = out.as_mut_ptr().add(len);
        let s = src.as_ptr().add(begin);
        match elem {
            1 => runs::<1>(d, s, stride, run, count),
            2 => runs::<2>(d, s, stride, run, count),
            4 => runs::<4>(d, s, stride, run, count),
            _ => runs::<8>(d, s, stride, run, count),
        }
        out.set_len(len + total);
    }
    Ok(())
}

/// Append the fixed-size `b` to `out` (a frame header or record): a constant-length store the compiler lays out inline, where
/// `extend_from_slice` at opt-level "s" is a call into the opener's `memcpy` (see the module note).
#[inline(always)]
pub(crate) fn put<const N: usize>(out: &mut Vec<u8>, b: [u8; N]) {
    out.reserve(N);
    let len = out.len();
    // SAFETY: `N` bytes reserved above, all written by the one store before `set_len`.
    unsafe {
        out.as_mut_ptr().add(len).cast::<[u8; N]>().write_unaligned(b);
        out.set_len(len + N);
    }
}

/// Append each of `words` as 8 little-endian bytes (a call frame's arguments): one reservation, an 8-byte store a word, no
/// `memcpy` call ([`opaque`]: a loop of plain 8-byte copies is an idiom the compiler turns back into one).
#[inline(always)]
pub(crate) fn put_words(out: &mut Vec<u8>, words: &[u64]) {
    out.reserve(words.len() * 8);
    let len = out.len();
    // SAFETY: `8 * words.len()` bytes reserved above, every one written (word k at `len + 8k`) before `set_len`.
    unsafe {
        let d = out.as_mut_ptr().add(len);
        for (k, w) in words.iter().enumerate() {
            d.add(8 * k).cast::<[u8; 8]>().write_unaligned(opaque(w.to_le()).to_ne_bytes());
        }
        out.set_len(len + 8 * words.len());
    }
}

/// `v` unchanged, through a register the optimiser cannot see into: a loop that only moves 8-byte words stays a loop of moves
/// instead of being rewritten as a `memcpy` call (the call this module exists to avoid). No instruction is emitted.
#[inline(always)]
fn opaque(v: u64) -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let mut v = v;
        // SAFETY: an empty template (an assembler comment): it touches nothing but the register that holds `v`.
        unsafe { core::arch::asm!("/* {0} */", inout(reg) v, options(pure, nomem, nostack, preserves_flags)) };
        v
    }
    #[cfg(not(target_arch = "x86_64"))]
    v
}

/// `count` runs, contiguous (`stride == run`: one long run) or strided (one short run a vertex).
///
/// # Safety
/// `d` is valid for `count * run` byte writes, `s` for reads of every run `s + i * stride .. + run`.
#[inline(always)]
unsafe fn runs<const E: usize>(d: *mut u8, s: *const u8, stride: usize, run: usize, count: usize) {
    if stride == run {
        convert::<E>(d, s, run * count);
    } else {
        let (mut d, mut s) = (d, s);
        for _ in 0..count {
            convert::<E>(d, s, run);
            // wrapping: after the last run the source cursor may point past `src` (it is never read there)
            (d, s) = (d.wrapping_add(run), s.wrapping_add(stride));
        }
    }
}

/// `n` bytes (a multiple of `E`) from `s` to `d`, each `E`-byte component reversed: 8-byte words, then a 4-, 2- and 1-byte tail.
///
/// # Safety
/// `d` valid for `n` byte writes, `s` for `n` byte reads; the two do not overlap.
#[inline(always)]
unsafe fn convert<const E: usize>(d: *mut u8, s: *const u8, n: usize) {
    let mut i = 0;
    while i + 8 <= n {
        let v = u64::from_le_bytes(s.add(i).cast::<[u8; 8]>().read_unaligned());
        d.add(i).cast::<[u8; 8]>().write_unaligned(swap64::<E>(v).to_le_bytes());
        i += 8;
    }
    // the tails a run of `E`-byte components can have (`n` is a multiple of `E`): none of them tested for 8-byte components
    if E <= 4 && i + 4 <= n {
        let v = u32::from_le_bytes(s.add(i).cast::<[u8; 4]>().read_unaligned());
        d.add(i).cast::<[u8; 4]>().write_unaligned(swap32::<E>(v).to_le_bytes());
        i += 4;
    }
    if E <= 2 && i + 2 <= n {
        let v = u16::from_le_bytes(s.add(i).cast::<[u8; 2]>().read_unaligned());
        d.add(i).cast::<[u8; 2]>().write_unaligned(if E == 1 { v } else { v.swap_bytes() }.to_le_bytes());
        i += 2;
    }
    if E == 1 && i < n {
        *d.add(i) = *s.add(i);
    }
}

/// Eight bytes read little-endian, every `E`-byte lane reversed in place.
#[inline(always)]
fn swap64<const E: usize>(v: u64) -> u64 {
    const M: u64 = 0x00FF_00FF_00FF_00FF;
    match E {
        1 => opaque(v), // a plain copy: kept a loop of moves, not a `memcpy` call
        2 => ((v & M) << 8) | ((v >> 8) & M),
        4 => v.swap_bytes().rotate_left(32),
        _ => v.swap_bytes(),
    }
}

/// Four bytes read little-endian, every `E`-byte lane reversed in place (`E` <= 4 here: a run of 8-byte components has no tail).
#[inline(always)]
fn swap32<const E: usize>(v: u32) -> u32 {
    const M: u32 = 0x00FF_00FF;
    match E {
        1 => v,
        2 => ((v & M) << 8) | ((v >> 8) & M),
        _ => v.swap_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference: copy each run, then reverse each component.
    fn reference(src: &[u8], begin: usize, stride: usize, run: usize, count: usize, elem: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..count {
            v.extend_from_slice(&src[begin + i * stride..begin + i * stride + run]);
        }
        v.chunks_exact_mut(elem).for_each(|c| c.reverse());
        v
    }

    #[test]
    fn every_width_stride_and_length_matches_copy_then_reverse() {
        let src: Vec<u8> = (0..2048u32).map(|i| (i.wrapping_mul(37) ^ (i >> 3)) as u8).collect();
        for elem in [1usize, 2, 4, 8] {
            for comps in 1..=4usize {
                let run = comps * elem;
                for pad in [0usize, 1, 3, 4, 8, 13] {
                    let stride = run + pad;
                    for begin in [0usize, 1, 5] {
                        for count in [0usize, 1, 2, 7, 33] {
                            let mut out = vec![0xAA, 0xBB];
                            append_swapped(&mut out, &src, begin, stride, run, count, elem).unwrap();
                            let mut want = vec![0xAA, 0xBB];
                            want.extend(reference(&src, begin, stride, run, count, elem));
                            assert_eq!(out, want, "elem {elem} run {run} stride {stride} begin {begin} count {count}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn words_are_little_endian_at_every_count() {
        for n in 0..20u64 {
            let words: Vec<u64> = (0..n).map(|k| 0x0102_0304_0506_0708u64.wrapping_mul(k + 1)).collect();
            let mut out = vec![9u8];
            put_words(&mut out, &words);
            let mut want = vec![9u8];
            words.iter().for_each(|w| want.extend_from_slice(&w.to_le_bytes()));
            assert_eq!(out, want);
        }
    }

    #[test]
    fn a_range_past_the_source_is_refused_and_writes_nothing() {
        let src = [0u8; 64];
        let mut out = vec![7u8];
        assert_eq!(append_swapped(&mut out, &src, 60, 12, 12, 1, 4), Err(SnapshotError::OutOfRange));
        assert_eq!(append_swapped(&mut out, &src, 0, 16, 12, 5, 4), Err(SnapshotError::OutOfRange));
        assert_eq!(append_swapped(&mut out, &src, usize::MAX - 4, 12, 12, 2, 4), Err(SnapshotError::Overflow));
        assert_eq!(out, [7]);
    }
}
