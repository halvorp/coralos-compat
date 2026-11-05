//! The chunky bitmap: one pixel VALUE per pixel. Indexed depths (1..=8) hold a pen index below `1 << depth` in one byte per
//! pixel; the direct depths 16 and 32 hold the whole pixel word (`u32` per pixel). The API speaks `u32` pixel values for every
//! depth, so the drawing primitives serve both without a second code path.
use crate::rect::Rect;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Store { Idx(Vec<u8>), Dir(Vec<u32>) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap { w: i32, h: i32, depth: u8, pix: Store }

/// A row of pixels in its storage form.
pub enum Row<'a> { Idx(&'a [u8]), Dir(&'a [u32]) }
/// A mutable row of pixels in its storage form.
pub enum RowMut<'a> { Idx(&'a mut [u8]), Dir(&'a mut [u32]) }

/// The depths a bitmap can have: indexed 1, 2, 4, 8 (and the Amiga's 3, 5, 6, 7 planes) and direct 16, 32.
pub fn valid_depth(depth: u8) -> bool { (1..=8).contains(&depth) || depth == 16 || depth == 32 }

impl Bitmap {
    /// `None` for a depth that is neither 1..=8 nor 16/32, or a size too large to allocate sensibly (> 64 Mpixel).
    pub fn new(w: i32, h: i32, depth: u8) -> Option<Bitmap> {
        if w <= 0 || h <= 0 || !valid_depth(depth) || (w as i64) * (h as i64) > 64 << 20 { return None; }
        let n = (w * h) as usize;
        let pix = if depth > 8 { Store::Dir(alloc::vec![0; n]) } else { Store::Idx(alloc::vec![0; n]) };
        Some(Bitmap { w, h, depth, pix })
    }
    pub fn width(&self) -> i32 { self.w }
    pub fn height(&self) -> i32 { self.h }
    pub fn depth(&self) -> u8 { self.depth }
    /// A direct (16 or 32 bit) bitmap: its pixel values are colours, not pen indexes.
    pub fn is_direct(&self) -> bool { self.depth > 8 }
    pub fn rect(&self) -> Rect { Rect::new(0, 0, self.w, self.h) }
    /// All bits a pixel of this depth can hold.
    pub fn pen_mask(&self) -> u32 { if self.depth == 32 { u32::MAX } else { (1u32 << self.depth) - 1 } }
    /// The bits that carry COLOUR: the whole pixel when indexed; at 16 bits x1R5G5B5 the low 15, at 32 bits x8R8G8B8 the low 24
    /// (the unused bit/byte is not colour, so inverting a pixel leaves it).
    pub fn color_mask(&self) -> u32 { match self.depth { 16 => 0x7FFF, 32 => 0x00FF_FFFF, _ => self.pen_mask() } }
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { return None; }
        let i = (y * self.w + x) as usize;
        Some(match &self.pix { Store::Idx(v) => u32::from(v[i]), Store::Dir(v) => v[i] })
    }
    /// Store a pixel value, clipped to the bitmap; bits above the depth are dropped. Returns whether the pixel exists.
    pub fn set(&mut self, x: i32, y: i32, pen: u32) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { return false; }
        let (i, v) = ((y * self.w + x) as usize, pen & self.pen_mask());
        match &mut self.pix { Store::Idx(p) => p[i] = v as u8, Store::Dir(p) => p[i] = v }
        true
    }
    /// Every pixel of an INDEXED bitmap, row-major (`None` for a direct one: use [`Bitmap::row_px`]).
    pub fn indexed(&self) -> Option<&[u8]> { match &self.pix { Store::Idx(v) => Some(v), Store::Dir(_) => None } }
    /// Every pixel of an indexed bitmap. Panics on a direct bitmap: callers that may hold one use [`Bitmap::indexed`] or
    /// [`Bitmap::row_px`].
    pub fn pixels(&self) -> &[u8] { self.indexed().expect("Bitmap::pixels on a direct (16/32-bit) bitmap") }
    /// One row of an indexed bitmap (panics on a direct one, as [`Bitmap::pixels`]).
    pub fn row(&self, y: i32) -> &[u8] { &self.pixels()[(y * self.w) as usize..((y + 1) * self.w) as usize] }
    /// One row in its storage form, any depth.
    pub fn row_px(&self, y: i32) -> Row<'_> {
        let r = (y * self.w) as usize..((y + 1) * self.w) as usize;
        match &self.pix { Store::Idx(v) => Row::Idx(&v[r]), Store::Dir(v) => Row::Dir(&v[r]) }
    }
    /// One mutable row in its storage form, any depth. The caller keeps values within [`Bitmap::pen_mask`].
    pub fn row_px_mut(&mut self, y: i32) -> RowMut<'_> {
        let r = (y * self.w) as usize..((y + 1) * self.w) as usize;
        match &mut self.pix { Store::Idx(v) => RowMut::Idx(&mut v[r]), Store::Dir(v) => RowMut::Dir(&mut v[r]) }
    }
    /// Pixels `x0..x1` of row `y` as `u32` values appended to `out` (any depth; the row-wise read every converter uses).
    pub fn read_span(&self, y: i32, x0: i32, x1: i32, out: &mut Vec<u32>) {
        match self.row_px(y) {
            Row::Idx(r) => out.extend(r[x0 as usize..x1 as usize].iter().map(|&p| u32::from(p))),
            Row::Dir(r) => out.extend_from_slice(&r[x0 as usize..x1 as usize]),
        }
    }
    /// Write `vals` (already within the depth) into row `y` from `x`.
    pub fn write_span(&mut self, y: i32, x: i32, vals: &[u32]) {
        let x = x as usize;
        match self.row_px_mut(y) {
            RowMut::Idx(r) => for (d, &v) in r[x..x + vals.len()].iter_mut().zip(vals) { *d = v as u8; },
            RowMut::Dir(r) => r[x..x + vals.len()].copy_from_slice(vals),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn set_get_clip_and_depth_masking() {
        let mut b = Bitmap::new(4, 3, 2).unwrap();
        assert!(b.set(3, 2, 0xFF) && b.get(3, 2) == Some(3), "bits above depth 2 are dropped");
        assert!(!b.set(4, 0, 1) && !b.set(-1, 0, 1) && b.get(0, 3).is_none());
        assert!(Bitmap::new(0, 3, 1).is_none() && Bitmap::new(3, 3, 9).is_none() && Bitmap::new(3, 3, 0).is_none());
        assert!(Bitmap::new(3, 3, 24).is_none() && Bitmap::new(3, 3, 33).is_none(), "negative: 24 and 33 are not storage depths");
        assert_eq!(Bitmap::new(2, 2, 8).unwrap().pen_mask(), 0xFF);
    }
    #[test]
    fn direct_depths_hold_whole_words_and_name_their_colour_bits() {
        let mut b = Bitmap::new(3, 2, 32).unwrap();
        assert!(b.is_direct() && b.indexed().is_none());
        b.set(2, 1, 0xAB12_3456u32);
        assert_eq!(b.get(2, 1), Some(0xAB12_3456));
        assert_eq!((b.pen_mask(), b.color_mask()), (u32::MAX, 0x00FF_FFFF));
        let mut s = Bitmap::new(3, 1, 16).unwrap();
        s.set(0, 0, 0x1_FFFFu32);
        assert_eq!(s.get(0, 0), Some(0xFFFF), "16-bit pixels keep 16 bits");
        assert_eq!(s.color_mask(), 0x7FFF);
        let mut v = Vec::new();
        b.read_span(1, 1, 3, &mut v);
        assert_eq!(v, [0, 0xAB12_3456]);
        b.write_span(0, 0, &[1, 2]);
        assert_eq!((b.get(0, 0), b.get(1, 0), b.get(2, 0)), (Some(1), Some(2), Some(0)));
        let mut i = Bitmap::new(3, 1, 8).unwrap();
        i.write_span(0, 1, &[7, 9]);
        assert_eq!(i.row(0), &[0, 7, 9]);
    }
}
