//! The chunky bitmap: one byte per pixel, a pen index below `1 << depth`.
use crate::rect::Rect;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap { w: i32, h: i32, depth: u8, pix: Vec<u8> }

impl Bitmap {
    /// `None` for a depth outside 1..=8 or a size too large to allocate sensibly (> 64 Mpixel).
    pub fn new(w: i32, h: i32, depth: u8) -> Option<Bitmap> {
        if w <= 0 || h <= 0 || depth == 0 || depth > 8 || (w as i64) * (h as i64) > 64 << 20 { return None; }
        Some(Bitmap { w, h, depth, pix: alloc::vec![0; (w * h) as usize] })
    }
    pub fn width(&self) -> i32 { self.w }
    pub fn height(&self) -> i32 { self.h }
    pub fn depth(&self) -> u8 { self.depth }
    pub fn rect(&self) -> Rect { Rect::new(0, 0, self.w, self.h) }
    /// All pen bits this bitmap can hold.
    pub fn pen_mask(&self) -> u8 { if self.depth == 8 { 0xFF } else { (1u8 << self.depth) - 1 } }
    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { None } else { Some(self.pix[(y * self.w + x) as usize]) }
    }
    /// Store a pen, clipped to the bitmap; bits above the depth are dropped. Returns whether the pixel exists.
    pub fn set(&mut self, x: i32, y: i32, pen: u8) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { return false; }
        let m = self.pen_mask();
        self.pix[(y * self.w + x) as usize] = pen & m;
        true
    }
    pub fn pixels(&self) -> &[u8] { &self.pix }
    pub fn row(&self, y: i32) -> &[u8] { &self.pix[(y * self.w) as usize..((y + 1) * self.w) as usize] }
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
        assert_eq!(Bitmap::new(2, 2, 8).unwrap().pen_mask(), 0xFF);
    }
}
