//! Shape scan conversion: a rectangle, oval or round rectangle becomes a pixel [`Region`] in local coordinates; Frame,
//! Paint, Erase, Invert and Fill all draw such a region (the frame is the shape minus the shape inset by the pen).
//!
//! Ovals use the exact ellipse inscribed in the rectangle, sampled at each row's centre with integer arithmetic (no
//! floating point: this crate is `no_std`), so the result is symmetric about both axes. Real QuickDraw's pixels
//! on a curve differ by at most a pixel at some rows; the invariants tests rely on are exact bounds, symmetry and
//! a filled interior.
use retro_raster::{Rect, Region};

fn isqrt(n: u64) -> u64 {
    let (mut x, mut y) = (n, (n + 1) / 2);
    if n == 0 { return 0; }
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

/// Row `k` (0-based from the top) of an ellipse inscribed in a `w` x `h` box whose left edge is 0:
/// the half-open [left, right) pixel span, or None for an empty box.
fn ellipse_row(w: i32, h: i32, k: i32) -> Option<(i32, i32)> {
    if w <= 0 || h <= 0 { return None; }
    let (w, h, t) = (i64::from(w), i64::from(h), i64::from(2 * k + 1 - h)); // t: row centre relative to the box centre, in half pixels
    let s = isqrt(((h * h - t * t).max(0) as u64) * 4096); // sqrt(h^2 - t^2) * 64
    let dx64 = w * s as i64 / (2 * h);
    let c64 = w * 32;
    let (l, r) = ((c64 - dx64 + 32) >> 6, (c64 + dx64 + 32) >> 6);
    Some((l.clamp(0, w) as i32, r.clamp(0, w) as i32))
}

pub fn rect(r: Rect) -> Region { Region::from_rect(r) }

/// The span of row `k` of an oval in box `r` (absolute coordinates).
fn oval_span(r: Rect, k: i32) -> Option<(i32, i32)> { ellipse_row(r.width(), r.height(), k).map(|(l, rt)| (r.x0 + l, r.x0 + rt)) }

/// The span of row `k` of a round rectangle in box `r`; `ow` x `oh` is the corner ellipse (clamped to the rectangle).
fn round_span(r: Rect, ow: i32, oh: i32, k: i32) -> Option<(i32, i32)> {
    let (w, h) = (r.width(), r.height());
    if k < 0 || k >= h { return None; }
    let (ow, oh) = (ow.clamp(0, w.max(0)), oh.clamp(0, h.max(0)));
    if ow == 0 || oh == 0 { return Some((r.x0, r.x1)); }
    let inset = if k < oh / 2 { ellipse_row(ow, oh, k).map(|(l, _)| l) }
        else if k >= h - (oh - oh / 2) { ellipse_row(ow, oh, k - (h - oh)).map(|(l, _)| l) }
        else { Some(0) };
    let i = inset.unwrap_or(0);
    Some((r.x0 + i, r.x1 - i))
}

pub fn oval(r: Rect) -> Region { Shape::Oval.region(r) }

/// `ow` x `oh` is the corner ellipse's size (clamped to the rectangle, as QuickDraw does).
pub fn round_rect(r: Rect, ow: i32, oh: i32) -> Region { Shape::RoundRect { ow, oh }.region(r) }

/// Which shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape { Rect, Oval, RoundRect { ow: i32, oh: i32 } }

impl Shape {
    /// Row `k` (0-based from the top of `r`) of the shape as an absolute `[x0, x1)` span.
    fn span(self, r: Rect, k: i32) -> Option<(i32, i32)> {
        match self {
            Shape::Rect => (k >= 0 && k < r.height() && !r.is_empty()).then_some((r.x0, r.x1)),
            Shape::Oval => if k < 0 || k >= r.height() { None } else { oval_span(r, k) },
            Shape::RoundRect { ow, oh } => round_span(r, ow, oh, k),
        }
    }
    /// Scan conversion is ONE pass over the rows (linear in the height); a guest-chosen huge rectangle costs
    /// proportionally, never quadratically.
    pub fn region(self, r: Rect) -> Region {
        if self == Shape::Rect { return rect(r); }
        Region::from_rows((0..r.height().max(0)).map(|k| (r.y0 + k, self.span(r, k).into_iter().collect())))
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
        Region::from_rows((0..r.height().max(0)).map(|k| {
            let spans = match (self.span(r, k), inner.span(inner_r, k - ph)) {
                (Some(o), Some(i)) => [(o.0, o.1.min(i.0)), (o.0.max(i.1), o.1)].into_iter().collect(),
                (Some(o), None) => alloc::vec![o],
                (None, _) => alloc::vec![],
            };
            (r.y0 + k, spans)
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
        for k in 0..r.height().max(0) { if let Some((l, rt)) = shape.span(r, k) { g.or_rect(Rect::new(l, r.y0 + k, rt, r.y0 + k + 1)); } }
        let mut f = g.clone();
        let inner_r = Rect::new(r.x0 + pw, r.y0 + ph, r.x1 - pw, r.y1 - ph);
        if !inner_r.is_empty() {
            let inner = match shape { Shape::RoundRect { ow, oh } => Shape::RoundRect { ow: (ow - 2 * pw).max(0), oh: (oh - 2 * ph).max(0) }, s => s };
            let mut ig = Region::new();
            for k in 0..inner_r.height() { if let Some((l, rt)) = inner.span(inner_r, k) { ig.or_rect(Rect::new(l, inner_r.y0 + k, rt, inner_r.y0 + k + 1)); } }
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
