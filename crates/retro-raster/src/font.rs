//! Bitmap fonts in the Amiga TextFont model (graphics/text.h): one-bit glyph strips of `modulo` bytes per row,
//! a (bit offset, bit width) CharLoc per glyph, optional CharSpace (advance) and CharKern, a default glyph at
//! `hi + 1`. The engine builds a [`Font`] from the guest's `struct TextFont`, so ROM fonts, disk fonts and a guest's
//! own TextFont all draw through this one renderer.
use crate::{bitmap::Bitmap, paint::{stencil, Brush, Clip, JAM2}, rect::Rect};
use alloc::vec::Vec;

pub const FSF_UNDERLINED: u8 = 1;
pub const FSF_BOLD: u8 = 2;
pub const FSF_ITALIC: u8 = 4;
pub const FPF_PROPORTIONAL: u8 = 0x20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    pub y_size: u16,
    pub x_size: u16,
    pub baseline: u16,
    pub bold_smear: u16,
    pub flags: u8,
    pub lo: u8,
    pub hi: u8,
    pub modulo: u16,
    pub data: Vec<u8>,
    /// Per glyph (0 ..= hi-lo+1, the last one the default glyph): bit offset in the strip << 16 | bit width.
    pub loc: Vec<u32>,
    pub space: Option<Vec<i16>>,
    pub kern: Option<Vec<i16>>,
}

/// The extent of drawn text relative to the cursor position (the baseline origin): a TextExtent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent { pub width: i32, pub rect: Rect }

impl Font {
    /// topaz 8 from the AROS ROM font data (`topaz8.rs`).
    pub fn topaz8() -> Font {
        use crate::topaz8::*;
        Font { y_size: Y_SIZE, x_size: X_SIZE, baseline: BASELINE, bold_smear: BOLD_SMEAR, flags: 0x01 | 0x02, lo: LO_CHAR, hi: HI_CHAR,
            modulo: MODULO, data: CHAR_DATA.to_vec(), loc: CHAR_LOC.to_vec(), space: None, kern: None }
    }
    /// Glyph slot for a character: itself inside lo..=hi, else the default glyph.
    pub fn slot(&self, ch: u8) -> usize {
        if ch >= self.lo && ch <= self.hi { (ch - self.lo) as usize } else { (self.hi - self.lo) as usize + 1 }
    }
    fn width_of(&self, slot: usize) -> i32 { self.loc.get(slot).map_or(0, |l| (l & 0xFFFF) as i32) }
    /// The pixels the cursor moves after this character.
    pub fn advance(&self, ch: u8) -> i32 {
        let s = self.slot(ch);
        let sp = match &self.space { Some(v) => v.get(s).copied().map_or(self.x_size as i32, i32::from), None => self.x_size as i32 };
        sp + self.kern.as_ref().and_then(|k| k.get(s)).copied().map_or(0, i32::from)
    }
    pub fn text_length(&self, text: &[u8]) -> i32 { text.iter().map(|&c| self.advance(c)).sum() }
    fn kern_of(&self, slot: usize) -> i32 { self.kern.as_ref().and_then(|k| k.get(slot)).copied().map_or(0, i32::from) }
    /// Is glyph pixel (gx, gy) set?
    pub fn glyph_bit(&self, slot: usize, gx: i32, gy: i32) -> bool {
        let Some(&l) = self.loc.get(slot) else { return false };
        let (off, w) = ((l >> 16) as i32, (l & 0xFFFF) as i32);
        if gx < 0 || gx >= w || gy < 0 || gy >= self.y_size as i32 { return false; }
        let bit = off + gx;
        self.data.get(gy as usize * self.modulo as usize + (bit >> 3) as usize).map_or(false, |b| (b >> (7 - (bit & 7))) & 1 != 0)
    }
    /// TextExtent of `text` drawn at the cursor with soft `style` bits.
    pub fn extent(&self, text: &[u8], style: u8) -> Extent {
        let width = self.text_length(text);
        let smear = if style & FSF_BOLD != 0 { self.bold_smear as i32 } else { 0 };
        let it = if style & FSF_ITALIC != 0 { (self.y_size as i32 - 1) / 2 } else { 0 };
        Extent { width, rect: Rect::new(0, -(self.baseline as i32), width + smear + it, self.y_size as i32 - self.baseline as i32) }
    }
    /// TextFit: how many characters of `text` (from the start, or from the end if `from_end`) fit in `bits` pixels
    /// of advance; returns that count.
    pub fn fit(&self, text: &[u8], bits: i32, from_end: bool) -> usize {
        let (mut used, mut n) = (0, 0);
        let it: &mut dyn Iterator<Item = &u8> = if from_end { &mut text.iter().rev() } else { &mut text.iter() };
        for &c in it {
            used += self.advance(c);
            if used > bits { break; }
            n += 1;
        }
        n
    }
}

/// Text(): draw `text` with the cursor `(x, y)` on the BASELINE. JAM2 paints each character cell's background across the
/// advance; underline is one row under the baseline; bold overstrikes by `bold_smear`; italic shears the upper rows.
/// Returns (cursor advance, touched rectangle).
pub fn draw_text(dst: &mut Bitmap, font: &Font, text: &[u8], x: i32, y: i32, b: &Brush, style: u8, clip: &Clip) -> (i32, Rect) {
    let top = y - font.baseline as i32;
    let mut cx = x;
    let mut touched = Rect::default();
    for &ch in text {
        let slot = font.slot(ch);
        let adv = font.advance(ch);
        let gx0 = cx + font.kern_of(slot);
        let cell = Rect::new(cx, top, cx + adv.max(0), top + font.y_size as i32);
        let smear = if style & FSF_BOLD != 0 { font.bold_smear as i32 } else { 0 };
        let ink = |px: i32, py: i32| -> bool {
            let gy = py - top;
            let shear = if style & FSF_ITALIC != 0 { (font.y_size as i32 - 1 - gy) / 2 } else { 0 };
            let mut on = false;
            for s in 0..=smear { on |= font.glyph_bit(slot, px - gx0 - s - shear, gy); }
            if style & FSF_UNDERLINED != 0 && py == y + 1 && px >= cx && px < cx + adv { on = true; }
            on
        };
        let mut area = cell;
        if style & FSF_UNDERLINED != 0 { area.y1 = area.y1.max(y + 2); }
        if style & (FSF_BOLD | FSF_ITALIC) != 0 || font.kern.is_some() {
            area.x1 = area.x1.max(gx0 + font.width_of(slot) + smear + if style & FSF_ITALIC != 0 { font.y_size as i32 / 2 } else { 0 });
        }
        // JAM1 and COMPLEMENT only ever touch ink pixels, so a wider area is harmless; JAM2 paints the whole `area`.
        let area = if b.mode & JAM2 != 0 { cell.union(&Rect::new(cell.x0, area.y0, cell.x1, area.y1)) } else { area };
        touched = touched.union(&stencil(dst, area, b, clip, ink));
        cx += adv;
    }
    (cx - x, touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::JAM1;

    #[test]
    fn topaz8_metrics_and_the_default_glyph() {
        let f = Font::topaz8();
        assert_eq!((f.y_size, f.x_size, f.baseline, f.lo, f.hi, f.modulo), (8, 8, 6, 32, 255, 192));
        assert_eq!(f.text_length(b"Hello"), 40);
        assert_eq!(f.slot(b'A'), 33);
        assert_eq!(f.slot(10), 224, "below lo: the default glyph");
        assert_eq!(f.loc.len(), 225);
    }
    #[test]
    fn the_capital_a_has_the_rom_shape() {
        // AROS topaz8 'A': a blank top row, six rows of letter ending on the baseline row (6), a blank descender row
        let f = Font::topaz8();
        let rows: Vec<u8> = (0..8).map(|gy| (0..8).fold(0u8, |a, gx| a << 1 | f.glyph_bit(f.slot(b'A'), gx, gy) as u8)).collect();
        assert_eq!(rows, [0x00, 0x18, 0x3c, 0x66, 0x66, 0x7e, 0x66, 0x00]);
    }
    #[test]
    fn draw_text_jam2_fills_cells_jam1_does_not_and_the_cursor_advances() {
        let f = Font::topaz8();
        let mut a = Bitmap::new(32, 16, 3).unwrap();
        let mut c = a.clone();
        let clip = Clip::rect(a.rect());
        let (adv, t1) = draw_text(&mut a, &f, b"AB", 4, 10, &Brush::new(1, 0, JAM1), 0, &clip);
        for y in 0..16 { for x in 0..32 { c.set(x, y, 2); } }
        let (_, t2) = draw_text(&mut c, &f, b"AB", 4, 10, &Brush::new(1, 3, JAM2), 0, &clip);
        assert_eq!(adv, 16);
        assert_eq!(t2, Rect::new(4, 4, 20, 12), "cells: top = y - baseline");
        assert!(Rect::new(4, 4, 20, 12).intersect(&t1) == t1);
        assert_eq!(a.get(4 + 3, 5), Some(1), "row 1 of A is 0x18: x=3,4");
        assert_eq!(a.get(4, 4), Some(0), "JAM1 leaves the paper");
        assert_eq!(c.get(4, 4), Some(3), "JAM2 writes the background");
        assert_eq!(c.get(4, 12), Some(2), "outside the cell: untouched (negative control)");
    }
    #[test]
    fn bold_underline_and_extent() {
        let f = Font::topaz8();
        let mut b = Bitmap::new(16, 16, 2).unwrap();
        let clip = Clip::rect(b.rect());
        draw_text(&mut b, &f, b"I", 0, 8, &Brush::new(1, 0, JAM1), FSF_UNDERLINED, &clip);
        assert!((0..8).all(|x| b.get(x, 9) == Some(1)), "underline one row below the baseline");
        assert_eq!(f.extent(b"ab", 0).rect, Rect::new(0, -6, 16, 2));
        assert_eq!(f.extent(b"ab", FSF_BOLD).rect.x1, 17, "bold smears one pixel right");
    }
    #[test]
    fn fit_counts_characters_from_either_end() {
        let f = Font::topaz8();
        assert_eq!(f.fit(b"abcdef", 20, false), 2);
        assert_eq!(f.fit(b"abcdef", 48, false), 6);
        assert_eq!(f.fit(b"abcdef", 7, true), 0, "negative control: not even one fits");
        assert_eq!(f.fit(b"abcdef", 24, true), 3);
    }
    #[test]
    fn proportional_space_and_kern_change_the_advance() {
        let mut f = Font::topaz8();
        let n = f.loc.len();
        f.space = Some(alloc::vec![5; n]);
        f.kern = Some(alloc::vec![1; n]);
        assert_eq!(f.text_length(b"abc"), 18);
    }
}
