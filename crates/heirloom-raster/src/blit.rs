//! Minterm blits between chunky bitmaps. A minterm is the Amiga blitter's ABC truth table: A = mask, B = source,
//! C = destination; bit 7 of the minterm is ABC, bit 0 is the all-complemented term. EVERY minterm is supported
//! (computed bitwise per pen), so 0xC0 copy, 0x30 invert-source, 0x50 invert-destination, 0x60 xor, 0x80 and and 0xE0 or are
//! all the same code, not special cases.
use crate::{bitmap::Bitmap, paint::Clip, rect::Rect};
use alloc::vec::Vec;

/// One pen through the truth table.
pub fn minterm(m: u8, a: u32, b: u32, c: u32) -> u32 {
    let mut r = 0;
    if m & 0x80 != 0 { r |= a & b & c; }
    if m & 0x40 != 0 { r |= a & b & !c; }
    if m & 0x20 != 0 { r |= a & !b & c; }
    if m & 0x10 != 0 { r |= a & !b & !c; }
    if m & 0x08 != 0 { r |= !a & b & c; }
    if m & 0x04 != 0 { r |= !a & b & !c; }
    if m & 0x02 != 0 { r |= !a & !b & c; }
    if m & 0x01 != 0 { r |= !a & !b & !c; }
    r
}

/// A per-pixel A (mask) input for [`blit`]: `None` = all ones.
pub type MaskFn<'a> = Option<&'a dyn Fn(i32, i32) -> bool>;

/// Snapshot of the source rectangle (clipped to the source bitmap; outside reads are not part of the blit).
/// Returns (clipped source rect, pixel values row-major).
fn snapshot(src: &Bitmap, sx: i32, sy: i32, w: i32, h: i32) -> (Rect, Vec<u32>) {
    let r = Rect::from_size(sx, sy, w, h).intersect(&src.rect());
    let mut v = Vec::with_capacity((r.width().max(0) * r.height().max(0)) as usize);
    if !r.is_empty() { for y in r.y0..r.y1 { src.read_span(y, r.x0, r.x1, &mut v); } }
    (r, v)
}

/// Blit `w`x`h` from `src` at (sx,sy) to `dst` at (dx,dy). `plane_mask` selects which pen bits are written (BltBitMap's
/// mask). The source is snapshotted first so overlapping blits within one bitmap are correct in every direction
/// (pass the same bitmap's copy through [`blit_within`]). Returns the destination rectangle touched.
pub fn blit(dst: &mut Bitmap, dx: i32, dy: i32, src: &Bitmap, sx: i32, sy: i32, w: i32, h: i32, mt: u8, plane_mask: u32, mask: MaskFn, clip: &Clip) -> Rect {
    let (r, pens) = snapshot(src, sx, sy, w, h);
    apply(dst, dx + (r.x0 - sx), dy + (r.y0 - sy), r, &pens, mt, plane_mask, mask, clip)
}

/// Same, source and destination being one bitmap.
pub fn blit_within(bm: &mut Bitmap, dx: i32, dy: i32, sx: i32, sy: i32, w: i32, h: i32, mt: u8, plane_mask: u32, mask: MaskFn, clip: &Clip) -> Rect {
    let (r, pens) = snapshot(bm, sx, sy, w, h);
    apply(bm, dx + (r.x0 - sx), dy + (r.y0 - sy), r, &pens, mt, plane_mask, mask, clip)
}

#[allow(clippy::too_many_arguments)]
fn apply(dst: &mut Bitmap, dx: i32, dy: i32, r: Rect, pens: &[u32], mt: u8, plane_mask: u32, mask: MaskFn, clip: &Clip) -> Rect {
    let area = Rect::from_size(dx, dy, r.width(), r.height());
    let full = dst.pen_mask();
    let mut touched = Rect::default();
    // srcCopy of every bit with no mask: whole rows (the hot path of an offscreen world shown in a window), no truth table
    let plain_copy = mask.is_none() && (plane_mask & full) == full && mt & 0xF0 == 0xC0;
    let mut row: Vec<u32> = Vec::new();
    for p in clip.pieces(area.intersect(&dst.rect())) {
        if plain_copy {
            for y in p.y0..p.y1 {
                let at = ((y - dy) * r.width() + (p.x0 - dx)) as usize;
                row.clear();
                row.extend(pens[at..at + p.width() as usize].iter().map(|&v| v & full));
                dst.write_span(y, p.x0, &row);
            }
            touched = touched.union(&p);
            continue;
        }
        for y in p.y0..p.y1 {
            for x in p.x0..p.x1 {
                let (ix, iy) = (x - dx, y - dy);
                let a = if mask.map_or(true, |m| m(x, y)) { u32::MAX } else { 0 };
                let s = pens[(iy * r.width() + ix) as usize];
                let old = dst.get(x, y).unwrap_or(0);
                let new = minterm(mt, a, s, old) & full;
                dst.set(x, y, (old & !plane_mask) | (new & plane_mask));
            }
        }
        touched = touched.union(&p);
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fill(b: &mut Bitmap, v: u32) { for y in 0..b.height() { for x in 0..b.width() { b.set(x, y, v); } } }

    #[test]
    fn named_minterms_are_the_blitter_functions() {
        for (m, f) in [(0xC0u8, (|s: u32, _d: u32| s) as fn(u32, u32) -> u32), (0x30, |s, _| !s), (0x50, |_, d| !d), (0x60, |s, d| s ^ d), (0x80, |s, d| s & d), (0xE0, |s, d| s | d)] {
            for s in [0u32, 3, 5, 7, 0x00FF_8040] { for d in [0u32, 1, 6, 7, 0x0012_3456] {
                assert_eq!(minterm(m, u32::MAX, s, d), f(s, d), "minterm {m:#x} s={s} d={d}");
            } }
        }
    }
    #[test]
    fn copy_with_a_clip_and_a_plane_mask() {
        let mut src = Bitmap::new(8, 8, 3).unwrap();
        let mut dst = Bitmap::new(8, 8, 3).unwrap();
        fill(&mut src, 5); fill(&mut dst, 2);
        let t = blit(&mut dst, 1, 1, &src, 0, 0, 4, 4, 0xC0, 0xFF, None, &Clip::rect(Rect::new(0, 0, 3, 8)));
        assert_eq!(t, Rect::new(1, 1, 3, 5));
        assert_eq!((dst.get(1, 1), dst.get(2, 4), dst.get(3, 1), dst.get(0, 0)), (Some(5), Some(5), Some(2), Some(2)));
        let mut d2 = Bitmap::new(8, 8, 3).unwrap();
        fill(&mut d2, 2);
        let c2 = Clip::rect(d2.rect());
        blit(&mut d2, 0, 0, &src, 0, 0, 2, 2, 0xC0, 0b001, None, &c2);
        assert_eq!(d2.get(0, 0), Some(0b011), "only plane 0 came across: 010 -> 011");
    }
    #[test]
    fn overlapping_blit_within_a_bitmap_is_correct_both_ways() {
        let mut b = Bitmap::new(8, 1, 3).unwrap();
        for x in 0..8 { b.set(x, 0, x as u32); }
        let clip = Clip::rect(b.rect());
        blit_within(&mut b, 2, 0, 0, 0, 6, 1, 0xC0, 0xFF, None, &clip);
        assert_eq!(b.row(0), &[0, 1, 0, 1, 2, 3, 4, 5], "scrolled right without smearing");
        blit_within(&mut b, 0, 0, 2, 0, 6, 1, 0xC0, 0xFF, None, &clip);
        assert_eq!(&b.row(0)[..6], &[0, 1, 2, 3, 4, 5]);
    }
    #[test]
    fn source_outside_the_bitmap_shifts_the_destination_with_it() {
        let mut src = Bitmap::new(4, 4, 3).unwrap();
        let mut dst = Bitmap::new(8, 8, 3).unwrap();
        fill(&mut src, 7);
        let cl = Clip::rect(dst.rect());
        let t = blit(&mut dst, 4, 4, &src, -2, -2, 4, 4, 0xC0, 0xFF, None, &cl);
        assert_eq!(t, Rect::new(6, 6, 8, 8));
        assert_eq!((dst.get(6, 6), dst.get(5, 5)), (Some(7), Some(0)));
    }
    #[test]
    fn a_mask_selects_which_destination_pixels_change() {
        let mut src = Bitmap::new(4, 1, 3).unwrap();
        fill(&mut src, 7);
        let mut dst = Bitmap::new(4, 1, 3).unwrap();
        let even = |x: i32, _y: i32| x % 2 == 0;
        let cl = Clip::rect(dst.rect());
        blit(&mut dst, 0, 0, &src, 0, 0, 4, 1, 0xC0, 0xFF, Some(&even), &cl);
        assert_eq!(dst.row(0), &[7, 0, 7, 0]);
    }
    #[test]
    fn invert_and_or_xor_destination_arithmetic() {
        let mut dst = Bitmap::new(2, 1, 3).unwrap();
        dst.set(0, 0, 0b101u32);
        let mut src = Bitmap::new(2, 1, 3).unwrap();
        src.set(0, 0, 0b011u32);
        let clip = Clip::rect(dst.rect());
        let mut d = dst.clone();
        blit(&mut d, 0, 0, &src, 0, 0, 1, 1, 0xE0, 0xFF, None, &clip);
        assert_eq!(d.get(0, 0), Some(0b111));
        let mut d = dst.clone();
        blit(&mut d, 0, 0, &src, 0, 0, 1, 1, 0x60, 0xFF, None, &clip);
        assert_eq!(d.get(0, 0), Some(0b110));
        let mut d = dst.clone();
        blit(&mut d, 0, 0, &src, 0, 0, 1, 1, 0x50, 0xFF, None, &clip);
        assert_eq!(d.get(0, 0), Some(0b010), "invert destination, source irrelevant");
    }
    #[test]
    fn direct_copy_takes_whole_rows_and_matches_the_truth_table_path() {
        let mut src = Bitmap::new(5, 3, 32).unwrap();
        for y in 0..3 { for x in 0..5 { src.set(x, y, 0x0010_2030u32 * (x + 1) as u32 + y as u32); } }
        let (mut a, mut b) = (Bitmap::new(6, 4, 32).unwrap(), Bitmap::new(6, 4, 32).unwrap());
        let cl = Clip::rect(a.rect());
        blit(&mut a, 1, 1, &src, 0, 0, 5, 3, 0xCA, u32::MAX, None, &cl);
        let all = |_: i32, _: i32| true;
        blit(&mut b, 1, 1, &src, 0, 0, 5, 3, 0xCA, u32::MAX, Some(&all), &cl); // a mask function forces the per-pixel path
        assert_eq!(a, b, "fast path == truth-table path");
        assert_eq!(a.get(5, 3), src.get(4, 2));
        let odd = |x: i32, _: i32| x % 2 == 1;
        let mut c = Bitmap::new(6, 4, 32).unwrap();
        blit(&mut c, 1, 1, &src, 0, 0, 5, 3, 0xCA, u32::MAX, Some(&odd), &cl);
        assert_eq!((c.get(1, 1), c.get(2, 1)), (src.get(0, 0), Some(0)), "negative: a mask still selects pixels (odd x only)");
        let mut x = Bitmap::new(2, 1, 32).unwrap();
        x.set(0, 0, 0x0F00_00FFu32);
        let mut s1 = Bitmap::new(2, 1, 32).unwrap();
        s1.set(0, 0, 0x0000_FF0Fu32);
        blit(&mut x, 0, 0, &s1, 0, 0, 1, 1, 0x6A, u32::MAX, None, &Clip::rect(Rect::new(0, 0, 2, 1)));
        assert_eq!(x.get(0, 0), Some(0x0F00_FFF0), "xor acts on the pixel value");
    }
}
