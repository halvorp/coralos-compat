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

/// `g` shrunk by `dh` left and right and `dv` top and bottom (grown if negative).
///
/// Cost depends on the region's number of bands, never on the distance: the region is constant between its band edges, so
/// the vertical window [y-|dv|, y+|dv|] only changes at an edge +/- |dv|; one window is evaluated per such change.
pub fn inset_region(g: &Region, dh: i32, dv: i32) -> Region {
    if g.is_empty() || (dh == 0 && dv == 0) { return g.clone(); }
    let mut ys: Vec<i32> = g.rects().iter().flat_map(|r| [r.y0, r.y1]).collect();
    ys.sort_unstable();
    ys.dedup();
    // horizontal pass per band: every maximal run [l, r) becomes [l + dh, r - dh)
    let bands: Vec<(i32, i32, Spans)> = ys.windows(2).map(|w| {
        let row = merge(g.rects().iter().filter(|r| r.y0 <= w[0] && w[0] < r.y1).map(|r| (r.x0 + dh, r.x1 - dh)).collect());
        (w[0], w[1], row)
    }).collect();
    let k = dv.abs();
    let (first, last) = (ys[0], ys[ys.len() - 1]);
    let mut cuts: Vec<i32> = ys.iter().flat_map(|&b| [b - k, b + k]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(i32, i32, Spans)> = Vec::new();
    for w in cuts.windows(2) {
        let (lo, hi) = (w[0] - k, w[0] + k + 1); // the rows [lo, hi) this segment's output depends on
        let touching = bands.iter().filter(|b| b.0 < hi && lo < b.1);
        let spans = if dv >= 0 {
            if lo < first || hi > last { Spans::new() } else { touching.map(|b| b.2.clone()).reduce(|a, b| intersect(&a, &b)).unwrap_or_default() }
        } else { merge(touching.flat_map(|b| b.2.iter().copied()).collect()) };
        if spans.is_empty() { continue; }
        match out.last_mut() { Some(l) if l.1 == w[0] && l.2 == spans => l.1 = w[1], _ => out.push((w[0], w[1], spans)) }
    }
    let mut r = Region::new();
    for (y0, y1, spans) in out { for (x0, x1) in spans { r.or_rect(retro_raster::Rect::new(x0, y0, x1, y1)); } }
    r
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
    /// A guest-chosen huge dv costs per band, not per row: this finished in ~14 s before the fix (and a timeout is the failure).
    #[test]
    fn a_huge_inset_distance_is_cheap_and_still_right() {
        let g = l_shape();
        let big = inset_region(&g, -1, -32767);
        assert_eq!(big.bounds(), Rect::new(-1, -32767, 11, 32767 + 10));
        assert!(big.contains(5, 0) && big.contains(2, 32767 + 9) && !big.contains(5, 32767 + 9), "grown columns follow the shape");
        assert!(inset_region(&g, 0, 32767).is_empty(), "negative control: shrinking by more than the height leaves nothing");
        assert_eq!(frame_region(&g, 1, 20000), g, "a tall pen frames the whole shape");
    }
    #[test]
    fn a_frame_is_the_ring_inside_the_region() {
        let f = frame_region(&l_shape(), 1, 1);
        assert!(f.contains(0, 0) && f.contains(5, 3) && f.contains(0, 5) && f.contains(3, 5) && f.contains(9, 0), "the ring is drawn");
        assert!(!f.contains(5, 1) && !f.contains(1, 5) && !f.contains(1, 1), "the inset core is not");
        assert_eq!(frame_region(&l_shape(), 2, 2), l_shape(), "no core left: the whole L");
    }
}
