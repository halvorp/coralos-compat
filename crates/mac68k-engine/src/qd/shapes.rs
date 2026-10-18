//! Shape scan conversion: a rectangle, oval or round rectangle becomes a pixel [`Region`] in local coordinates; Frame,
//! Paint, Erase, Invert and Fill all draw such a region (the frame is the shape minus the shape inset by the pen).
//!
//! Ovals and round rectangles are scan-converted by the algorithm Bill Atkinson's QuickDraw uses (DrawArc.a `INITOVAL` /
//! `BUMPOVAL`, driven by PutOval.a), re-derived here as integer arithmetic (this crate is `no_std`): per scanline the left and
//! right edges are 16.16 fixed-point values that grow or shrink by half a pixel at a time until a 64-bit running square
//! (an odd-number sum scaled by the squared aspect ratio) matches the row's `R^2 - y^2`. The result is NOT symmetric about
//! the vertical axis for every size (the right edge carries a +1/2 bias), which is exactly what the original draws.
use retro_raster::{Rect, Region};
use alloc::vec::Vec;
use alloc::vec;

/// The rows of an oval (`ow` x `oh` corners) inscribed in a `w` x `h` box whose left edge is 0: row `k` is the half-open
/// span `[left, right)` (`left <= right`; equal = nothing drawn on that row). `ow == w, oh == h` is the whole oval; a smaller
/// corner ellipse gives a round rectangle, whose middle rows keep the edges of the last corner row.
fn oval_rows(w: i32, h: i32, ow: i32, oh: i32) -> Vec<(i32, i32)> {
    if w <= 0 || h <= 0 { return Vec::new(); }
    let (ow, oh) = (ow.clamp(0, w), oh.clamp(0, h));
    if ow == 0 || oh == 0 { return vec![(0, w); h as usize]; }
    const HALF: i64 = 0x8000;
    // INITOVAL: left = ow/2, right = w - ow/2 + 1/2; ovaly = 1 - oh; rsqysq = 2*oh - 1; square = 0;
    // oddnum = aspect^2 where aspect = FixRatio(oh, ow) (a 32.32 value), oddbump = 2 * oddnum.
    let (mut left, mut right) = (i64::from(ow) * HALF, (i64::from(w) << 16) - i64::from(ow) * HALF + HALF);
    let (mut ovaly, mut rsq, mut square) = (i64::from(1 - oh), i64::from(2 * oh - 1), 0i128);
    let aspect = (i64::from(oh) << 16) / i64::from(ow);
    let (mut oddnum, oddbump) = (i128::from(aspect) * i128::from(aspect), 2 * i128::from(aspect) * i128::from(aspect));
    let (skip_top, skip_bot) = (oh / 2, oh / 2 + h - oh); // PutOval.a: rows that bump the state
    (0..h).map(|k| {
        if k < skip_top || k >= skip_bot { // BUMPOVAL: only the integer part of the square is compared
            let y1 = ovaly + 1;
            ovaly += 2;
            while (square >> 32) < i128::from(rsq) { right += HALF; left -= HALF; square += oddnum; oddnum += oddbump; }
            while (square >> 32) > i128::from(rsq) { right -= HALF; left += HALF; oddnum -= oddbump; square -= oddnum; }
            rsq -= 4 * y1;
        }
        let (l, r) = ((left >> 16).clamp(0, i64::from(w)) as i32, (right >> 16).clamp(0, i64::from(w)) as i32);
        (l, r.max(l))
    }).collect()
}

pub fn rect(r: Rect) -> Region { Region::from_rect(r) }

pub fn oval(r: Rect) -> Region { Shape::Oval.region(r) }

/// `ow` x `oh` is the corner ellipse's size (clamped to the rectangle, as QuickDraw does).
pub fn round_rect(r: Rect, ow: i32, oh: i32) -> Region { Shape::RoundRect { ow, oh }.region(r) }

/// Which shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape { Rect, Oval, RoundRect { ow: i32, oh: i32 } }

impl Shape {
    /// Every row of the shape in `r`, top to bottom, as absolute `[x0, x1)` spans (`None` = nothing on that row).
    fn rows(self, r: Rect) -> Vec<Option<(i32, i32)>> {
        let (w, h) = (r.width(), r.height());
        let rel = match self {
            Shape::Rect => return (0..h.max(0)).map(|_| (!r.is_empty()).then_some((r.x0, r.x1))).collect(),
            Shape::Oval => oval_rows(w, h, w, h),
            Shape::RoundRect { ow, oh } => oval_rows(w, h, ow, oh),
        };
        rel.into_iter().map(|(l, rt)| (rt > l).then_some((r.x0 + l, r.x0 + rt))).collect()
    }
    /// Scan conversion is ONE pass over the rows (linear in the height); a guest-chosen huge rectangle costs
    /// proportionally, never quadratically.
    pub fn region(self, r: Rect) -> Region {
        if self == Shape::Rect { return rect(r); }
        Region::from_rows(self.rows(r).into_iter().enumerate().map(|(k, s)| (r.y0 + k as i32, s.into_iter().collect())))
    }
    /// The frame: the shape minus the same shape inset by the pen (`pw` wide, `ph` tall) on every side, row by row.
    pub fn frame(self, r: Rect, pw: i32, ph: i32) -> Region {
        let inner_r = Rect::new(r.x0 + pw, r.y0 + ph, r.x1 - pw, r.y1 - ph);
        if inner_r.is_empty() { return self.region(r); }
        let inner = match self { Shape::RoundRect { ow, oh } => Shape::RoundRect { ow: (ow - 2 * pw).max(0), oh: (oh - 2 * ph).max(0) }, s => s };
        if self == Shape::Rect {
            let mut outer = rect(r);
            outer.clear_rect(inner_r);
            return outer;
        }
        let inner_rows = inner.rows(inner_r);
        Region::from_rows(self.rows(r).into_iter().enumerate().map(|(k, outer)| {
            let spans = match (outer, usize::try_from(k as i32 - ph).ok().and_then(|i| inner_rows.get(i).copied().flatten())) {
                (Some(o), Some(i)) => [(o.0, o.1.min(i.0)), (o.0.max(i.1), o.1)].into_iter().filter(|s| s.1 > s.0).collect(),
                (Some(o), None) => alloc::vec![o],
                (None, _) => alloc::vec![],
            };
            (r.y0 + k as i32, spans)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oval_has_exact_bounds_symmetry_and_a_filled_interior() {
        let r = Rect::new(10, 20, 50, 60);
        let g = oval(r);
        assert_eq!(g.bounds(), r);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let (mx, my) = (r.x0 + r.x1 - 1 - x, r.y0 + r.y1 - 1 - y);
                assert_eq!(g.contains(x, y), g.contains(mx, y), "left-right symmetry at {x},{y}");
                assert_eq!(g.contains(x, y), g.contains(x, my), "top-bottom symmetry at {x},{y}");
            }
        }
        assert!(g.contains(30, 40) && !g.contains(10, 20), "centre in, corner out");
    }
    #[test]
    fn round_rect_cuts_only_the_corners_and_degenerates_to_a_rect() {
        let r = Rect::new(0, 0, 40, 30);
        let g = round_rect(r, 16, 16);
        assert!(!g.contains(0, 0) && !g.contains(39, 29) && g.contains(20, 0) && g.contains(0, 15) && g.contains(8, 8));
        assert_eq!(round_rect(r, 0, 16), rect(r), "no corner width: a plain rectangle");
        assert_eq!(round_rect(r, 99, 99).bounds(), r, "oversized corners clamp to the box");
    }
    #[test]
    fn frame_is_the_ring_between_the_shape_and_its_inset() {
        let r = Rect::new(0, 0, 10, 10);
        let f = Shape::Rect.frame(r, 2, 1);
        assert!(f.contains(0, 5) && f.contains(1, 5) && !f.contains(2, 5) && f.contains(5, 0) && !f.contains(5, 1) && f.contains(5, 9));
        assert_eq!(Shape::Rect.frame(Rect::new(0, 0, 3, 3), 2, 2), rect(Rect::new(0, 0, 3, 3)), "a pen thicker than the shape fills it");
    }
    /// The pre-linear construction (one `or_rect` per row, frame by `clear_region`): the oracle for the row-wise one.
    fn slow(shape: Shape, r: Rect, pw: i32, ph: i32) -> (Region, Region) {
        let mut g = Region::new();
        for (k, sp) in shape.rows(r).into_iter().enumerate() { if let Some((l, rt)) = sp { g.or_rect(Rect::new(l, r.y0 + k as i32, rt, r.y0 + k as i32 + 1)); } }
        let mut f = g.clone();
        let inner_r = Rect::new(r.x0 + pw, r.y0 + ph, r.x1 - pw, r.y1 - ph);
        if !inner_r.is_empty() {
            let inner = match shape { Shape::RoundRect { ow, oh } => Shape::RoundRect { ow: (ow - 2 * pw).max(0), oh: (oh - 2 * ph).max(0) }, s => s };
            let mut ig = Region::new();
            for (k, sp) in inner.rows(inner_r).into_iter().enumerate() { if let Some((l, rt)) = sp { ig.or_rect(Rect::new(l, inner_r.y0 + k as i32, rt, inner_r.y0 + k as i32 + 1)); } }
            f.clear_region(&ig);
        }
        (g, f)
    }
    #[test]
    fn the_linear_scan_conversion_equals_the_row_by_row_or_rect_oracle() {
        for shape in [Shape::Oval, Shape::RoundRect { ow: 10, oh: 14 }, Shape::RoundRect { ow: 3, oh: 0 }] {
            for (w, h, pw, ph) in [(37, 29, 1, 1), (40, 40, 3, 2), (9, 50, 4, 4), (6, 6, 1, 1), (31, 3, 2, 1)] {
                let r = Rect::new(-5, 7, -5 + w, 7 + h);
                let (g, f) = slow(shape, r, pw, ph);
                assert_eq!(shape.region(r), g, "{shape:?} {w}x{h}");
                assert_eq!(shape.frame(r, pw, ph), f, "{shape:?} frame {w}x{h} pen {pw}x{ph}");
            }
        }
        assert_ne!(Shape::Oval.region(Rect::new(0, 0, 30, 30)), Shape::RoundRect { ow: 10, oh: 10 }.region(Rect::new(0, 0, 30, 30)), "negative control: different shapes differ");
    }
    #[test]
    fn a_huge_oval_is_scan_converted_in_linear_time() {
        let t = std::time::Instant::now();
        let r = Rect::new(-32767, -32767, 32767, 32767);
        let (g, f) = (Shape::Oval.region(r), Shape::Oval.frame(r, 3, 3));
        assert!(g.rects().len() > 1000 && f.rects().len() > g.rects().len());
        assert!(t.elapsed().as_secs() < 5, "took {:?}", t.elapsed());
    }
}

#[cfg(test)]
mod original_algorithm {
    use super::*;
    /// Hand-traced through DrawArc.a INITOVAL/BUMPOVAL (aspect 1.0, rsqysq 7, 15, 15, 7; the square grows by the odd numbers
    /// 1, 3, 5, ... half a pixel each side, then the second loop undoes an overshoot): the classic 4x4 circle.
    #[test]
    fn the_four_by_four_oval_is_the_classic_circle() {
        assert_eq!(oval_rows(4, 4, 4, 4), vec![(1, 3), (0, 4), (0, 4), (1, 3)]);
        assert_ne!(oval_rows(4, 4, 4, 4), oval_rows(4, 4, 2, 2), "negative control: a smaller corner ellipse is a different shape");
    }
    /// PutOval.a SKIPTOP/SKIPBOT: a round rectangle bumps the oval state only through the corner rows, so its middle rows
    /// keep the last corner row's edges, i.e. the full width.
    #[test]
    fn a_round_rectangles_middle_rows_are_full_width() {
        let rows = oval_rows(20, 12, 8, 6);
        assert!(rows[3..9].iter().all(|&r| r == (0, 20)), "{rows:?}");
        assert!(rows[0].0 > 0 && rows[11].0 > 0 && rows[0].1 < 20);
    }
    /// A capsule: the corner ellipse as big as the box is the plain oval.
    #[test]
    fn corner_ellipse_as_big_as_the_box_is_the_oval() {
        assert_eq!(oval_rows(31, 26, 31, 26), oval_rows(31, 26, 99, 99), "oversized corners clamp to the box (INITOVAL OK3/OK4)");
    }
}
