//! CopyBits with a srcRect and dstRect of different sizes (StretchBits): the two-axis DDA of Bill Atkinson's QuickDraw
//! (Stretch.a `StretchBits`, `SetupStretch`, and its general `STRETCH` / `SHRINK` row routines; the fast table-driven
//! cases (double, triple, half, ...) are the same mapping), as integer arithmetic.
//!
//! Vertically: `verror` starts at -srcHeight/2; each source row read adds dstHeight; a shrinking transfer ORs further source
//! rows into the row buffer while `verror <= 0`; each destination row drawn subtracts srcHeight and, while `verror >= 0`,
//! the same row is drawn again (a stretch repeats rows). Horizontally a 16-bit fraction does the same per pixel: stretching
//! repeats a source pixel until the carry, shrinking ORs the source pixels that fall into one destination pixel.
//! Source pixels the DDA reads beyond the srcRect are the bitmap's own neighbours (the original reads whole rows).
use alloc::vec;
use alloc::vec::Vec;
use retro_raster::{Bitmap, Rect};

/// One source row as `0`/`1` pens, `sw` of them plus the bitmap's neighbours past the rectangle (reads outside: white).
fn read_row(src: &Bitmap, sr: Rect, row: i32, sw: i32, extra: i32) -> Vec<u8> {
    (0..sw + extra).map(|x| u8::from(src.get(sr.x0 + x, sr.y0 + row) == Some(1))).collect()
}

/// `Stretch.a` STRETCH / SHRINK: map `sw` source pens to `dw` destination pens.
pub fn stretch_row(src: &[u8], sw: i32, dw: i32) -> Vec<u8> {
    if sw <= 0 || dw <= 0 { return Vec::new(); } // SetupStretch aborts on a non-positive ratio; never size a buffer from it
    let (sw_u, dw_u) = (sw as usize, dw as usize);
    let mut dst = vec![0u8; dw_u];
    let at = |i: usize| src.get(i).copied().unwrap_or(0);
    if dw == sw { for (i, d) in dst.iter_mut().enumerate() { *d = at(i); } return dst; }
    let err_carry = |err: &mut u32, f: u32| { *err += f; let c = *err >= 0x1_0000; *err &= 0xFFFF; c };
    if dw > sw {
        let f = (((i64::from(sw)) << 16) / i64::from(dw)) as u32 & 0xFFFF; // FixRatio(denom, numer): the low word
        let (mut err, mut s) = (f >> 1, 0usize);
        for d in dst.iter_mut() { // emit one destination pixel, then advance the source on a carry
            *d = at(s);
            if err_carry(&mut err, f) { s += 1; }
        }
    } else {
        let f = (((i64::from(dw)) << 16) / i64::from(sw)) as u32 & 0xFFFF; // FixRatio(numer, denom)
        let (mut err, mut d, mut s, mut cur) = (f >> 1, 0usize, 0usize, 0u8);
        while d < dw_u {
            cur |= at(s);
            s += 1;
            if err_carry(&mut err, f) { dst[d] = cur; d += 1; cur = 0; }
            if s > sw_u + dw_u + 16 { break; } // a fraction of 0 never carries: stop instead of spinning
        }
    }
    dst
}

/// Call `emit(dst_row, pens)` for every destination row `0..dh` of `sr` stretched to `dw` x `dh`, top to bottom.
pub fn stretch<E: FnMut(i32, &[u8]) -> Result<(), crate::Refusal>>(src: &Bitmap, sr: Rect, dw: i32, dh: i32, mut emit: E) -> Result<(), crate::Refusal> {
    let (sw, sh) = (sr.width(), sr.height());
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 { return Ok(()); } // SetupStretch: numer or denom <= 0 aborts
    let extra = (dw.max(sw) / 8).max(16) + 16; // the DDA may read a little past the rect
    let (mut verror, mut next_src, mut vert) = (-(sh / 2), 0i32, 0i32);
    let src_ok = |row: i32| src.get(sr.x0, sr.y0 + row).is_some() || src.get(sr.x0 + sw - 1, sr.y0 + row).is_some();
    'rows: loop {
        if !src_ok(next_src) { return Ok(()); } // NEXTSRC: the source bitmap is exhausted
        let mut buf = read_row(src, sr, next_src, sw, extra);
        next_src += 1;
        verror += dh;
        while verror <= 0 && src_ok(next_src) { // SRCLOOP: merge further source rows
            for (b, n) in buf.iter_mut().zip(read_row(src, sr, next_src, sw, extra)) { *b |= n; }
            next_src += 1;
            verror += dh;
        }
        let row = stretch_row(&buf, sw, dw);
        loop {
            emit(vert, &row)?;
            vert += 1;
            if vert >= dh { break 'rows; }
            verror -= sh;
            if verror < 0 { break; }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use retro_raster::Bitmap;
    fn bits(s: &str) -> Vec<u8> { s.bytes().map(|b| u8::from(b == b'#')).collect() }
    fn show(v: &[u8]) -> String { v.iter().map(|&p| if p == 1 { '#' } else { '.' }).collect() }
    /// SetupStretch DOUBLE / TRIPLE: every source pixel becomes two (three) destination pixels.
    #[test]
    fn whole_number_stretches_repeat_each_pixel() {
        assert_eq!(show(&stretch_row(&bits("#.#."), 4, 8)), "##..##..");
        assert_eq!(show(&stretch_row(&bits("#.#"), 3, 9)), "###...###");
        assert_ne!(show(&stretch_row(&bits("#.#."), 4, 8)), "#.#.#.#.", "negative control: not a plain tiling");
    }
    /// SetupStretch HALF: a destination pixel is the OR of the two source pixels that fall into it (the original keeps
    /// thin black lines when shrinking instead of dropping them).
    #[test]
    fn shrinking_ors_the_source_pixels_into_one() {
        assert_eq!(show(&stretch_row(&bits("#..#.#.."), 8, 4)), "###.");
        assert_eq!(show(&stretch_row(&bits("#......."), 8, 4)), "#...", "a lone pixel survives in the first pair only");
    }
    #[test]
    fn a_ratio_of_one_is_a_copy_and_odd_ratios_keep_the_width() {
        assert_eq!(show(&stretch_row(&bits("#.#.#"), 5, 5)), "#.#.#");
        assert_eq!(stretch_row(&vec![1; 40], 40, 24).len(), 24);
        assert!(stretch_row(&vec![1; 40], 40, 24).iter().all(|&p| p == 1), "an all-black row stays black when shrunk");
        assert!(stretch_row(&vec![0; 16], 16, 36).iter().all(|&p| p == 0), "negative control: white stays white");
    }
    /// Stretch.a SRCLOOP: shrinking vertically ORs the source rows that fall into one destination row. Source rows 1 and 3
    /// carry the only black pixels, so sampling row 0 (or 2) instead of ORing would leave both destination rows white.
    #[test]
    fn vertical_shrink_ors_source_rows() {
        let mut b = Bitmap::new(8, 4, 1).unwrap();
        b.set(3, 1, 1);
        b.set(5, 3, 1);
        let mut rows: Vec<(i32, Vec<u8>)> = Vec::new();
        stretch(&b, Rect::new(0, 0, 8, 4), 8, 2, |r, p| { rows.push((r, p.to_vec())); Ok(()) }).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(show(&rows[0].1), "...#....");
        assert_eq!(show(&rows[1].1), ".....#..");
        let mut white = Bitmap::new(8, 4, 1).unwrap();
        white.set(0, 0, 0);
        let mut n = 0;
        stretch(&white, Rect::new(0, 0, 8, 4), 8, 2, |_, p| { assert!(p.iter().all(|&x| x == 0), "negative control: white stays white"); n += 1; Ok(()) }).unwrap();
        assert_eq!(n, 2);
    }
    #[test]
    fn stretch_row_refuses_degenerate_sizes_instead_of_panicking() {
        assert!(stretch_row(&[1, 1], -3, 4).is_empty() && stretch_row(&[1, 1], 4, -3).is_empty() && stretch_row(&[], 0, 0).is_empty());
        assert_eq!(show(&stretch_row(&[1], 4, 4)), "#...", "a short source reads as white past its end");
    }
}
