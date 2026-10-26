//! Drawing into a [`Bitmap`] under a clip and a draw mode: the one stencil primitive (fills, templates, glyphs and
//! pattern lines all go through it), pixels and lines.
use crate::{bitmap::Bitmap, rect::Rect, region::Region};
use alloc::vec::Vec;

pub const JAM1: u8 = 0;
pub const JAM2: u8 = 1;
pub const COMPLEMENT: u8 = 2;
pub const INVERSVID: u8 = 4;

/// What to paint with: pens, draw mode (graphics/rastport.h RP_DrawMode bits) and the RastPort write mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brush { pub fg: u8, pub bg: u8, pub mode: u8, pub mask: u8 }
impl Brush {
    pub fn new(fg: u8, bg: u8, mode: u8) -> Brush { Brush { fg, bg, mode, mask: 0xFF } }
}

/// Where drawing may land: a rectangle and, optionally, a region inside it.
#[derive(Debug, Clone, Copy)]
pub struct Clip<'a> { pub rect: Rect, pub region: Option<&'a Region> }
impl<'a> Clip<'a> {
    pub fn rect(rect: Rect) -> Clip<'a> { Clip { rect, region: None } }
    pub fn allows(&self, x: i32, y: i32) -> bool { self.rect.contains(x, y) && self.region.map_or(true, |g| g.contains(x, y)) }
    /// The pieces of `r` drawing may touch.
    pub fn pieces(&self, r: Rect) -> Vec<Rect> {
        let r = r.intersect(&self.rect);
        if r.is_empty() { return Vec::new(); }
        match self.region {
            None => alloc::vec![r],
            Some(g) => g.rects().iter().map(|p| p.intersect(&r)).filter(|p| !p.is_empty()).collect(),
        }
    }
}

fn combine(old: u8, new: u8, mask: u8) -> u8 { (old & !mask) | (new & mask) }

/// Paint every pixel of `area` whose stencil bit `ink(x, y)` (absolute bitmap coordinates) is set (`INVERSVID` flips
/// the bit): JAM1 writes the foreground pen there; JAM2 writes foreground there and background elsewhere;
/// COMPLEMENT inverts the pixel there. Returns the touched rectangle (empty if the clip rejected everything).
pub fn stencil(dst: &mut Bitmap, area: Rect, b: &Brush, clip: &Clip, ink: impl Fn(i32, i32) -> bool) -> Rect {
    let full = dst.pen_mask();
    stencil_with(dst, area, clip, |x, y, old| {
        let bit = ink(x, y) ^ (b.mode & INVERSVID != 0);
        let new = if b.mode & COMPLEMENT != 0 { if bit { Some(old ^ full) } else { None } }
            else if bit { Some(b.fg) } else if b.mode & JAM2 != 0 { Some(b.bg) } else { None };
        new.map(|v| combine(old, v, b.mask))
    })
}

/// The loop under every stencil: for each pixel of `area` the clip allows, `f(x, y, old)` gives the pixel's new pen
/// (`None` leaves it). A caller whose rule is not a brush (a raster operation over the destination pens, as QuickDraw's
/// Boolean transfer modes are) supplies its own `f`. Returns the touched rectangle.
pub fn stencil_with(dst: &mut Bitmap, area: Rect, clip: &Clip, mut f: impl FnMut(i32, i32, u8) -> Option<u8>) -> Rect {
    let mut touched = Rect::default();
    for p in clip.pieces(area.intersect(&dst.rect())) {
        for y in p.y0..p.y1 {
            for x in p.x0..p.x1 {
                if let Some(v) = f(x, y, dst.get(x, y).unwrap_or(0)) { dst.set(x, y, v); }
            }
        }
        touched = touched.union(&p);
    }
    touched
}

/// A repeating AreaPtrn: 16 pixels wide, `rows.len()` (a power of two) tall, anchored at the bitmap origin.
#[derive(Debug, Clone)]
pub struct Pattern { pub rows: Vec<u16> }
impl Pattern {
    pub fn bit(&self, x: i32, y: i32) -> bool {
        let r = self.rows[(y.rem_euclid(self.rows.len() as i32)) as usize];
        (r >> (15 - x.rem_euclid(16))) & 1 != 0
    }
}

/// RectFill: the foreground (or the pattern) over `area`.
pub fn fill_rect(dst: &mut Bitmap, area: Rect, b: &Brush, pat: Option<&Pattern>, clip: &Clip) -> Rect {
    stencil(dst, area, b, clip, |x, y| pat.map_or(true, |p| p.bit(x, y)))
}

/// WritePixel with the foreground pen (JAM/COMPLEMENT apply).
pub fn plot(dst: &mut Bitmap, x: i32, y: i32, b: &Brush, clip: &Clip) -> Rect {
    stencil(dst, Rect::new(x, y, x + 1, y + 1), b, clip, |_, _| true)
}

/// Draw: Bresenham from (x0,y0) to (x1,y1) inclusive. `ptrn` is the 16-bit LinePtrn, `phase` the pattern counter
/// (15 = first bit), which advances once per plotted pixel and is returned for the next segment.
pub fn line(dst: &mut Bitmap, (x0, y0): (i32, i32), (x1, y1): (i32, i32), b: &Brush, ptrn: u16, phase: &mut u8, clip: &Clip) -> Rect {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    let mut touched = Rect::default();
    loop {
        let bit = (ptrn >> (*phase & 15)) & 1 != 0;
        *phase = (*phase + 15) & 15;
        touched = touched.union(&stencil(dst, Rect::new(x, y, x + 1, y + 1), b, clip, |_, _| bit));
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x += sx; }
        if e2 <= dx { err += dx; y += sy; }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bm() -> Bitmap { Bitmap::new(16, 8, 3).unwrap() }
    fn px(b: &Bitmap) -> Vec<u8> { b.pixels().to_vec() }

    #[test]
    fn jam1_paints_ink_only_and_jam2_paints_the_paper_too() {
        let stripe = |x: i32, _| x % 2 == 0;
        let (mut a, mut c) = (bm(), bm());
        for b in [&mut a, &mut c] { fill_rect(b, Rect::new(0, 0, 16, 8), &Brush::new(0, 0, JAM2), None, &Clip::rect(Rect::new(0, 0, 16, 8))); }
        let clip = Clip::rect(Rect::new(0, 0, 16, 8));
        stencil(&mut a, Rect::new(0, 0, 4, 1), &Brush::new(5, 2, JAM1), &clip, stripe);
        stencil(&mut c, Rect::new(0, 0, 4, 1), &Brush::new(5, 2, JAM2), &clip, stripe);
        assert_eq!(&a.pixels()[..4], &[5, 0, 5, 0], "JAM1 leaves the paper");
        assert_eq!(&c.pixels()[..4], &[5, 2, 5, 2], "JAM2 writes the background");
    }
    #[test]
    fn complement_inverts_within_depth_and_twice_restores() {
        let mut b = bm();
        let clip = Clip::rect(b.rect());
        fill_rect(&mut b, Rect::new(2, 2, 6, 4), &Brush::new(5, 0, JAM1), None, &clip);
        let before = px(&b);
        fill_rect(&mut b, Rect::new(0, 0, 16, 8), &Brush::new(1, 0, COMPLEMENT), None, &clip);
        assert_eq!(b.get(3, 3), Some(5 ^ 7));
        assert_eq!(b.get(0, 0), Some(7), "depth 3: complement of 0 is 7, not 255");
        fill_rect(&mut b, Rect::new(0, 0, 16, 8), &Brush::new(1, 0, COMPLEMENT), None, &clip);
        assert_eq!(px(&b), before);
    }
    #[test]
    fn stencil_with_runs_the_callers_rule_inside_the_clip_only() {
        let mut b = bm();
        let clip = Clip::rect(Rect::new(1, 1, 3, 2));
        let t = stencil_with(&mut b, Rect::new(0, 0, 16, 8), &clip, |x, _, old| Some(old | (x as u8 + 1)));
        assert_eq!(b.pixels().iter().filter(|&&p| p != 0).count(), 2);
        assert_eq!((b.get(1, 1), b.get(2, 1), b.get(0, 1)), (Some(2), Some(3), Some(0)));
        assert_eq!(t, Rect::new(1, 1, 3, 2));
    }
    #[test]
    fn inversvid_swaps_ink_and_paper() {
        let mut b = bm();
        let clip = Clip::rect(b.rect());
        stencil(&mut b, Rect::new(0, 0, 4, 1), &Brush::new(5, 2, JAM2 | INVERSVID), &clip, |x, _| x < 2);
        assert_eq!(&b.pixels()[..4], &[2, 2, 5, 5]);
    }
    #[test]
    fn the_clip_rect_and_region_both_limit_drawing() {
        let mut b = bm();
        let g = { let mut g = Region::from_rect(Rect::new(0, 0, 16, 8)); g.clear_rect(Rect::new(4, 0, 8, 8)); g };
        let clip = Clip { rect: Rect::new(2, 0, 10, 4), region: Some(&g) };
        let t = fill_rect(&mut b, Rect::new(0, 0, 16, 8), &Brush::new(1, 0, JAM1), None, &clip);
        assert_eq!(b.pixels().iter().filter(|&&p| p == 1).count(), (2 + 2) * 4, "x 2..4 and 8..10, rows 0..4");
        assert!(!t.is_empty());
        let none = fill_rect(&mut b, Rect::new(20, 20, 30, 30), &Brush::new(1, 0, JAM1), None, &clip);
        assert!(none.is_empty(), "negative control: nothing to touch outside");
    }
    #[test]
    fn write_mask_protects_other_planes() {
        let mut b = bm();
        let clip = Clip::rect(b.rect());
        b.set(1, 1, 0b110);
        let br = Brush { fg: 0b001, bg: 0, mode: JAM1, mask: 0b001 };
        plot(&mut b, 1, 1, &br, &clip);
        assert_eq!(b.get(1, 1), Some(0b111));
        let br2 = Brush { fg: 0b001, bg: 0, mode: JAM1, mask: 0 };
        plot(&mut b, 2, 1, &br2, &clip);
        assert_eq!(b.get(2, 1), Some(0));
    }
    #[test]
    fn lines_hit_their_endpoints_and_diagonals_are_one_pixel_per_step() {
        let mut b = bm();
        let clip = Clip::rect(b.rect());
        let mut ph = 15;
        line(&mut b, (1, 1), (6, 6), &Brush::new(1, 0, JAM1), 0xFFFF, &mut ph, &clip);
        assert!((1..=6).all(|i| b.get(i, i) == Some(1)));
        assert_eq!(b.pixels().iter().filter(|&&p| p == 1).count(), 6);
        let mut c = bm();
        let mut ph2 = 15;
        line(&mut c, (0, 0), (7, 0), &Brush::new(1, 0, JAM1), 0xF0F0, &mut ph2, &clip);
        assert_eq!(&c.pixels()[..8], &[1, 1, 1, 1, 0, 0, 0, 0], "pattern bit 15 first");
        assert_eq!(ph2, 7, "phase continues for the next segment");
        let mut d = bm();
        line(&mut d, (7, 5), (0, 5), &Brush::new(1, 0, JAM1), 0xFFFF, &mut 15, &clip);
        assert_eq!(d.row(5)[..8], [1; 8], "either direction");
    }
    #[test]
    fn area_pattern_repeats_from_the_origin() {
        let mut b = bm();
        let clip = Clip::rect(b.rect());
        let p = Pattern { rows: alloc::vec![0xAAAA, 0x5555] };
        fill_rect(&mut b, Rect::new(0, 0, 4, 2), &Brush::new(3, 0, JAM1), Some(&p), &clip);
        assert_eq!(&b.row(0)[..4], &[3, 0, 3, 0]);
        assert_eq!(&b.row(1)[..4], &[0, 3, 0, 3]);
    }
}
