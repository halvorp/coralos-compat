//! The scanline fill behind `AreaEnd`: the 1-bit mask (graphics/areafill.c's `areafillpolygon` / `areafillellipse` / `areaoutlinepolygonmask`) that
//! `BltPattern` then paints through. Pure geometry, no guest memory: vertices in, a mask of `bytes_per_row * rows` bytes out, the bits MSB first
//! (pixel `x` of a row is bit `0x80 >> (x & 7)` of byte `x >> 3`: the big-endian layout of a bitplane, which is what `LineInTmpRas` writes on an m68k).
//!
//! The rules, as AROS's areafill.c has them (the algorithm is Harrington's, pages 79-91, as that file says; only its rules are used here):
//! * a four-edge polygon whose corners turn the same way (convex) is filled by stepping each edge in 16.16 fixed point, one step per scanline the
//!   edge is active in (`y1 <= y < y2`; the step is the truncated quotient), the run on a scanline being the smallest to the largest edge x
//!   (arithmetic shift down);
//! * any other polygon is filled even-odd: per scanline `MinY..=MaxY` every non-horizontal edge with `min(y1,y2) <= y < max(y1,y2)` crosses at
//!   `x1 + (y - y1) * (x2 - x1) / (y2 - y1)` (C division, truncating), the crossings are sorted and taken in pairs, a pair being the inclusive run;
//! * then the outline is added: a Bresenham walk from each vertex to the next.
//! * an ellipse is the runs `cx - x ..= cx + x` at `cy -/+ y` of the midpoint algorithm of `DrawEllipse`.
use alloc::vec;
use alloc::vec::Vec;

/// `BytesPerRow` of the mask for a `width` wide area: whole words (areaend.c).
pub fn bytes_per_row(width: i32) -> i32 { if width & 0x0f != 0 { ((width >> 3) & 0xfffe) + 2 } else { (width >> 3) & 0xfffe } }

/// The inclusive pixel box a fill covers and its mask.
pub struct Mask { pub min_x: i32, pub min_y: i32, pub max_x: i32, pub max_y: i32, pub bytes_per_row: i32, pub bits: Vec<u8> }

impl Mask {
    pub fn new(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> Mask {
        let bpr = bytes_per_row(max_x - min_x + 1);
        Mask { min_x, min_y, max_x, max_y, bytes_per_row: bpr, bits: vec![0; (bpr * (max_y - min_y + 1)).max(0) as usize] }
    }
    /// `LineInTmpRas`: the inclusive run `xl..=xr` of row `y` (absolute coordinates). Pixels outside the buffer are not written.
    fn line(&mut self, xl: i32, xr: i32, y: i32) {
        let (xl, xr, row) = (xl - self.min_x, xr - self.min_x, y - self.min_y);
        if xl > xr || row < 0 { return; }
        for x in xl.max(0)..=xr {
            if x >= self.bytes_per_row * 8 { break; }
            if let Some(b) = self.bits.get_mut((row * self.bytes_per_row + (x >> 3)) as usize) { *b |= 0x80 >> (x & 7); }
        }
    }
    /// Is pixel (x, y) (absolute) set?
    #[cfg(test)]
    pub fn get(&self, x: i32, y: i32) -> bool {
        let (x, row) = (x - self.min_x, y - self.min_y);
        x >= 0 && row >= 0 && x < self.bytes_per_row * 8 && self.bits.get((row * self.bytes_per_row + (x >> 3)) as usize).is_some_and(|b| b & (0x80 >> (x & 7)) != 0)
    }
}

/// The bounds of polygon vertices (inclusive).
pub fn bounds(v: &[(i32, i32)]) -> (i32, i32, i32, i32) {
    v.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |b, &(x, y)| (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y)))
}

/// Do the four corners of `v[0..4]` turn the same way, with at least one real turn (areafillpolygon's convex test)?
fn convex_quad(v: &[(i32, i32)]) -> bool {
    let mut winding = 0i32;
    for i in 0..4 {
        let (p, q, r) = (v[i], v[(i + 1) & 3], v[(i + 2) & 3]);
        let (ax, ay, bx, by) = (q.0 - p.0, q.1 - p.1, r.0 - q.0, r.1 - q.1);
        let cross = ax * by - ay * bx;
        if cross != 0 {
            if winding != 0 && ((cross < 0) != (winding < 0)) { return false; }
            winding = cross;
        }
    }
    winding != 0
}

/// Fill the polygon `v` (its last vertex is the first again: `v.len() - 1` edges) into a fresh mask over `bounds(v)`.
pub fn polygon(v: &[(i32, i32)]) -> Mask {
    let (x0, y0, x1, y1) = bounds(v);
    let mut m = Mask::new(x0, y0, x1, y1);
    let edges = v.len() - 1;
    if edges == 4 && convex_quad(v) {
        // (edge_x, step, min_y, max_y) per edge, 16.16 fixed point
        let mut e: Vec<(i32, i32, i32, i32)> = (0..4).map(|i| {
            let ((xa, ya), (xb, yb)) = (v[i], v[(i + 1) & 3]);
            if ya < yb { (xa * 65536, ((xb - xa) * 65536) / (yb - ya), ya, yb) }
            else if yb < ya { (xb * 65536, ((xa - xb) * 65536) / (ya - yb), yb, ya) }
            else { (xa * 65536, 0, ya, ya) }
        }).collect();
        for y in y0..=y1 {
            let (mut left, mut right) = (i32::MAX, -i32::MAX);
            for (ex, step, lo, hi) in e.iter_mut() {
                if y >= *lo && y < *hi {
                    left = left.min(*ex);
                    right = right.max(*ex);
                    *ex += *step;
                }
            }
            if left <= right { m.line(left >> 16, right >> 16, y); }
        }
    } else {
        for y in y0..=y1 {
            let mut cross: Vec<i32> = (0..edges).filter_map(|i| {
                let ((xa, ya), (xb, yb)) = (v[i], v[i + 1]);
                (ya != yb && y >= ya.min(yb) && y < ya.max(yb)).then(|| xa + ((y - ya) * (xb - xa)) / (yb - ya))
            }).collect();
            cross.sort_unstable();
            for p in cross.chunks_exact(2) { m.line(p[0], p[1], y); }
        }
    }
    outline(&mut m, v);
    m
}

/// `areaoutlinepolygonmask`: the polygon's boundary walked edge by edge.
fn outline(m: &mut Mask, v: &[(i32, i32)]) {
    for w in v.windows(2) {
        let ((mut x, mut y), (x2, y2)) = (w[0], w[1]);
        let (dx, dy) = ((x2 - x).abs(), (y2 - y).abs());
        let (sx, sy) = (if x < x2 { 1 } else { -1 }, if y < y2 { 1 } else { -1 });
        let mut error = dx - dy;
        loop {
            m.line(x, x, y);
            if x == x2 && y == y2 { break; }
            let twice = error << 1;
            if twice > -dy { error -= dy; x += sx; }
            if twice < dx { error += dx; y += sy; }
        }
    }
}

/// `areafillellipse`: the filled ellipse of centre (cx, cy) and radii (a, b), over the box cx-a..=cx+a, cy-b..=cy+b.
pub fn ellipse(cx: i32, cy: i32, a: i32, b: i32) -> Mask {
    let mut m = Mask::new(cx - a, cy - b, cx + a, cy + b);
    let (mut x, mut y) = (a, 0);
    let (t1, t4) = (a * a, b * b);
    let (t2, t3, t5, t6) = (t1 << 1, t1 << 2, t4 << 1, t4 << 2);
    let mut t8 = (a * t5) << 1;
    let mut t9 = 0;
    let mut d1 = t2 - a * t5 + (t4 >> 1);
    let mut d2 = (t1 >> 1) - t8 + t5;
    let both = |m: &mut Mask, x: i32, y: i32| { m.line(cx - x, cx + x, cy - y); m.line(cx - x, cx + x, cy + y); };
    while d2 < 0 && y < b {
        if x >= 0 { both(&mut m, x, y); }
        y += 1;
        t9 += t3;
        if d1 < 0 { d1 += t9 + t2; d2 += t9; }
        else { x -= 1; t8 -= t6; d1 += t9 + t2 - t8; d2 += t9 + t5 - t8; }
    }
    loop {
        x -= 1;
        t8 -= t6;
        if d2 < 0 {
            if x >= 0 { both(&mut m, x, y); } else { break; }
            y += 1;
            t9 += t3;
            d2 += t9 + t5 - t8;
        } else { d2 += t5 - t8; }
        if !(x > 0 && y < b) { break; }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(m: &Mask) -> Vec<Vec<i32>> { (m.min_y..=m.max_y).map(|y| (m.min_x..=m.max_x).filter(|&x| m.get(x, y)).collect()).collect() }

    #[test]
    fn bytes_per_row_is_whole_words() {
        assert_eq!([1, 16, 17, 32, 33].map(bytes_per_row), [2, 2, 4, 4, 6]);
    }

    #[test]
    fn an_axis_aligned_rectangle_fills_its_rows_and_outline_closes_the_bottom_edge() {
        let m = polygon(&[(2, 1), (6, 1), (6, 4), (2, 4), (2, 1)]);
        let r = rows(&m);
        // rows 1..3 come from the edge crossings (y < max); the last row (4) is only the outline's bottom edge
        assert_eq!(r[0], vec![2, 3, 4, 5, 6]);
        assert_eq!(r[3], vec![2, 3, 4, 5, 6], "the outline walk sets the bottom edge");
        assert!(r.iter().all(|row| row.len() == 5));
    }

    #[test]
    fn a_concave_polygon_is_even_odd_and_a_notch_stays_empty() {
        // a "U" opening upwards: 8 edges, so the even-odd path
        let m = polygon(&[(0, 0), (2, 0), (2, 3), (4, 3), (4, 0), (6, 0), (6, 5), (0, 5), (0, 0)]);
        assert!(m.get(3, 1) == false, "inside the notch (between the arms) nothing is set above the base");
        assert!(m.get(1, 1) && m.get(5, 1), "the arms are filled");
        assert!(m.get(3, 4), "the base under the notch is filled");
        // NEGATIVE CONTROL: the same fill of the convex hull would set (3,1)
        let hull = polygon(&[(0, 0), (6, 0), (6, 5), (0, 5), (0, 0)]);
        assert!(hull.get(3, 1));
    }

    #[test]
    fn a_kite_takes_the_fixed_point_path_and_a_bow_tie_does_not() {
        // a convex kite (a clock hand): symmetric about x = 10
        let kite = polygon(&[(10, 0), (13, 10), (10, 40), (7, 10), (10, 0)]);
        let r = rows(&kite);
        assert!(r.iter().all(|row| !row.is_empty()), "every row has a run");
        let widths: Vec<usize> = r.iter().map(|row| row.len()).collect();
        assert!(widths[10] > widths[0] && widths[10] > widths[39], "widest at the shoulders: {widths:?}");
        assert_eq!(r[10], (7..=13).collect::<Vec<_>>(), "at y = 10 the two lower edges start at their own x, 7 and 13");
        // NEGATIVE CONTROL: a bow tie (corners turn both ways) is not convex, so it goes even-odd and leaves the crossing rows' middle out
        assert!(!convex_quad(&[(0, 0), (4, 4), (4, 0), (0, 4)]));
        assert!(convex_quad(&[(10, 0), (13, 10), (10, 40), (7, 10)]));
    }

    #[test]
    fn an_ellipse_is_symmetric_and_fills_its_middle_row_across() {
        let m = ellipse(10, 8, 6, 4);
        assert_eq!((m.min_x, m.max_x, m.min_y, m.max_y), (4, 16, 4, 12));
        assert!((4..=16).all(|x| m.get(x, 8)), "the middle row spans the full width: {:?}", rows(&m)[4]);
        for dy in 0..=3 { assert_eq!(rows(&m)[(4 - dy) as usize], rows(&m)[(4 + dy) as usize], "symmetric about the centre row"); }
        assert!(!m.get(4, 4) && !m.get(16, 12), "the corners of the box stay empty");
    }
}
