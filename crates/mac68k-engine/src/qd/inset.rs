//! InsetRgn for a general region, and FrameRgn built on it. QuickDraw/Regions.a: `FrRgn` copies the region, `InsetRgn`s the
//! copy by the pen size, takes `DiffRgn(rgn, copy)` and draws that, so a frame lies INSIDE the region and is as thick as the pen.
//! `InsetRgn` shrinks (positive dh/dv) or grows (negative) a shape by running the inversion points through `RgnOp` INSET
//! first horizontally then vertically: a pixel survives iff every pixel within `dh` on its row and within `dv` in its column
//! is inside, i.e. an erosion by a (2dh+1) x (2dv+1) rectangle, separable, so two passes over scan rows.
use alloc::vec::Vec;
use retro_raster::Region;

type Spans = Vec<(i32, i32)>;

fn merge(mut v: Spans) -> Spans {
    v.sort_unstable();
    let mut out: Spans = Vec::new();
    for s in v.into_iter().filter(|s| s.0 < s.1) {
        match out.last_mut() { Some(l) if s.0 <= l.1 => l.1 = l.1.max(s.1), _ => out.push(s) }
    }
    out
}

fn intersect(a: &Spans, b: &Spans) -> Spans {
    let mut out = Spans::new();
    for x in a { for y in b { let (l, r) = (x.0.max(y.0), x.1.min(y.1)); if l < r { out.push((l, r)); } } }
    merge(out)
}

/// The region's scan rows from `top` for `n` rows.
fn rows_of(g: &Region, top: i32, n: i32) -> Vec<Spans> {
    (top..top + n).map(|y| merge(g.rects().iter().filter(|r| r.y0 <= y && y < r.y1).map(|r| (r.x0, r.x1)).collect())).collect()
}

/// `g` shrunk by `dh` left and right and `dv` top and bottom (grown if negative).
pub fn inset_region(g: &Region, dh: i32, dv: i32) -> Region {
    if g.is_empty() || (dh == 0 && dv == 0) { return g.clone(); }
    let b = g.bounds();
    let pad_v = dv.min(0).abs(); // growing vertically adds rows above and below
    let (top, n) = (b.y0 - pad_v, b.height() + 2 * pad_v);
    // horizontal pass: every maximal run [l, r) of every row becomes [l + dh, r - dh)
    let h: Vec<Spans> = rows_of(g, top, n).into_iter().map(|row| merge(row.into_iter().map(|(l, r)| (l + dh, r - dh)).collect())).collect();
    // vertical pass: row y is the intersection (shrink) or union (grow) of rows y-|dv| ..= y+|dv|
    let k = dv.abs();
    let rows = (0..n).map(|i| {
        let window = (i - k..=i + k).map(|j| if j >= 0 && j < n { h[j as usize].clone() } else { Spans::new() });
        let spans = if dv >= 0 { window.reduce(|a, b| intersect(&a, &b)).unwrap_or_default() } else { merge(window.flatten().collect()) };
        (top + i, spans)
    });
    Region::from_rows(rows)
}

/// FrameRgn's pixels: the region minus itself inset by the pen (`DiffRgn(rgn, InsetRgn(copy, pnSize))`).
pub fn frame_region(g: &Region, pw: i32, ph: i32) -> Region {
    let mut f = g.clone();
    f.clear_region(&inset_region(g, pw, ph));
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use retro_raster::Rect;
    fn l_shape() -> Region { let mut g = Region::from_rect(Rect::new(0, 0, 10, 4)); g.or_rect(Rect::new(0, 4, 4, 10)); g }
    /// Regions.a InsetRgn: a rectangle inset by (1,2) is the smaller rectangle, whichever code path computes it.
    #[test]
    fn a_rectangle_insets_like_inset_rect() {
        assert_eq!(inset_region(&Region::from_rect(Rect::new(0, 0, 10, 8)), 1, 2), Region::from_rect(Rect::new(1, 2, 9, 6)));
    }
    /// The L: every pixel of the inset is deeper than dh from a left/right edge and dv from a top/bottom edge.
    #[test]
    fn an_l_shape_insets_to_its_core_and_grows_back_bigger() {
        let g = l_shape();
        let s = inset_region(&g, 1, 1);
        assert_eq!(s.bounds(), Rect::new(1, 1, 9, 9));
        assert!(s.contains(1, 1) && s.contains(8, 2) && s.contains(2, 8) && !s.contains(5, 5) && !s.contains(0, 0) && !s.contains(3, 8));
        let mut s_chk = s.clone(); s_chk.clear_region(&g);
        assert!(s_chk.is_empty(), "an inset lies inside the original");
        let grown = inset_region(&g, -1, -1);
        assert!(grown.contains(-1, -1) && grown.contains(10, 3) && grown.contains(4, 5), "negative insets grow");
        assert_ne!(inset_region(&g, 2, 2), s, "negative control: a bigger inset is smaller");
    }
    /// A frame is the region minus its inset: the one-pixel ring inside the L (hand-derived above: the inset keeps rows 1-2 at
    /// [1,9) and rows 3-8 at [1,3)), and a pen too thick to leave a core frames the whole shape.
    #[test]
    fn a_frame_is_the_ring_inside_the_region() {
        let f = frame_region(&l_shape(), 1, 1);
        assert!(f.contains(0, 0) && f.contains(5, 3) && f.contains(0, 5) && f.contains(3, 5) && f.contains(9, 0), "the ring is drawn");
        assert!(!f.contains(5, 1) && !f.contains(1, 5) && !f.contains(1, 1), "the inset core is not");
        assert_eq!(frame_region(&l_shape(), 2, 2), l_shape(), "no core left: the whole L");
    }
}
