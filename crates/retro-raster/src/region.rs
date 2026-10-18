//! Regions: a set of pixels as disjoint rectangles in CANONICAL form (horizontal bands with maximal merging), so two
//! regions covering the same pixels compare equal and export as the same RegionRectangle chain.
use crate::rect::Rect;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Region { rects: Vec<Rect> }

impl Region {
    pub fn new() -> Region { Region::default() }
    pub fn from_rect(r: Rect) -> Region { let mut g = Region::new(); g.or_rect(r); g }
    pub fn rects(&self) -> &[Rect] { &self.rects }
    pub fn is_empty(&self) -> bool { self.rects.is_empty() }
    pub fn bounds(&self) -> Rect { self.rects.iter().fold(Rect::default(), |a, r| a.union(r)) }
    pub fn contains(&self, x: i32, y: i32) -> bool { self.rects.iter().any(|r| r.contains(x, y)) }
    pub fn clear(&mut self) { self.rects.clear(); }
    pub fn translate(&mut self, dx: i32, dy: i32) { for r in &mut self.rects { *r = r.translate(dx, dy); } }

    pub fn or_rect(&mut self, r: Rect) {
        if r.is_empty() { return; }
        let mut pieces = alloc::vec![r];
        for e in &self.rects { pieces = pieces.iter().flat_map(|p| p.subtract(e)).collect(); }
        self.rects.extend(pieces);
        self.canon();
    }
    pub fn clear_rect(&mut self, r: Rect) {
        self.rects = self.rects.iter().flat_map(|e| e.subtract(&r)).collect();
        self.canon();
    }
    pub fn and_rect(&mut self, r: Rect) {
        self.rects = self.rects.iter().map(|e| e.intersect(&r)).filter(|e| !e.is_empty()).collect();
        self.canon();
    }
    pub fn xor_rect(&mut self, r: Rect) {
        let mut only_r = Region::from_rect(r);
        only_r.clear_region(self);
        self.clear_rect(r);
        self.or_region(&only_r);
    }
    pub fn or_region(&mut self, o: &Region) { for r in &o.rects { self.or_rect(*r); } }
    pub fn clear_region(&mut self, o: &Region) { for r in &o.rects { self.clear_rect(*r); } }
    pub fn and_region(&mut self, o: &Region) {
        self.rects = self.rects.iter().flat_map(|a| o.rects.iter().map(move |b| a.intersect(b))).filter(|e| !e.is_empty()).collect();
        self.canon();
    }
    pub fn xor_region(&mut self, o: &Region) {
        let mut only_o = o.clone();
        only_o.clear_region(self);
        self.clear_region(o);
        self.or_region(&only_o);
    }

    /// Slab decomposition: for each y interval between rectangle edges, the merged x spans; equal neighbouring slabs
    /// are joined. Rectangles come out sorted by (y0, x0).
    fn canon(&mut self) {
        if self.rects.is_empty() { return; }
        let mut ys: Vec<i32> = self.rects.iter().flat_map(|r| [r.y0, r.y1]).collect();
        ys.sort_unstable();
        ys.dedup();
        let mut bands: Vec<(i32, i32, Vec<(i32, i32)>)> = Vec::new();
        for w in ys.windows(2) {
            let mut spans: Vec<(i32, i32)> = self.rects.iter().filter(|r| r.y0 <= w[0] && r.y1 >= w[1]).map(|r| (r.x0, r.x1)).collect();
            if spans.is_empty() { continue; }
            spans.sort_unstable();
            let mut merged: Vec<(i32, i32)> = Vec::new();
            for s in spans {
                match merged.last_mut() { Some(l) if s.0 <= l.1 => l.1 = l.1.max(s.1), _ => merged.push(s) }
            }
            match bands.last_mut() {
                Some(b) if b.1 == w[0] && b.2 == merged => b.1 = w[1],
                _ => bands.push((w[0], w[1], merged)),
            }
        }
        self.rects = bands.into_iter().flat_map(|(y0, y1, spans)| spans.into_iter().map(move |(x0, x1)| Rect::new(x0, y0, x1, y1))).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(g: &Region) -> i32 { g.rects().iter().map(|r| r.width() * r.height()).sum() }

    #[test]
    fn or_of_overlapping_rects_counts_each_pixel_once() {
        let mut g = Region::new();
        g.or_rect(Rect::new(0, 0, 10, 10));
        g.or_rect(Rect::new(5, 5, 15, 15));
        assert_eq!(area(&g), 100 + 100 - 25);
        assert!(g.contains(12, 12) && g.contains(0, 0) && !g.contains(12, 2));
        assert_eq!(g.bounds(), Rect::new(0, 0, 15, 15));
    }
    #[test]
    fn canonical_form_is_independent_of_how_the_region_was_built() {
        let mut a = Region::from_rect(Rect::new(0, 0, 10, 10));
        a.or_rect(Rect::new(10, 0, 20, 10));
        assert_eq!(a, Region::from_rect(Rect::new(0, 0, 20, 10)), "adjacent rects merge into one");
        let mut b = Region::from_rect(Rect::new(0, 0, 20, 10));
        b.clear_rect(Rect::new(5, 0, 15, 10));
        let mut c = Region::from_rect(Rect::new(0, 0, 5, 10));
        c.or_rect(Rect::new(15, 0, 20, 10));
        assert_eq!(b, c);
        assert_ne!(b, a, "negative control: different pixels differ");
    }
    #[test]
    fn and_xor_clear_match_set_algebra_pixel_by_pixel() {
        let a = { let mut g = Region::from_rect(Rect::new(0, 0, 8, 8)); g.or_rect(Rect::new(10, 3, 14, 12)); g };
        let b = { let mut g = Region::from_rect(Rect::new(4, 4, 12, 9)); g.clear_rect(Rect::new(6, 5, 8, 7)); g };
        let (mut and, mut xor, mut clr, mut or) = (a.clone(), a.clone(), a.clone(), a.clone());
        and.and_region(&b); xor.xor_region(&b); clr.clear_region(&b); or.or_region(&b);
        for y in -1..14 { for x in -1..16 {
            let (ia, ib) = (a.contains(x, y), b.contains(x, y));
            assert_eq!(and.contains(x, y), ia && ib, "and {x},{y}");
            assert_eq!(xor.contains(x, y), ia != ib, "xor {x},{y}");
            assert_eq!(clr.contains(x, y), ia && !ib, "clear {x},{y}");
            assert_eq!(or.contains(x, y), ia || ib, "or {x},{y}");
        } }
    }
    #[test]
    fn xor_rect_twice_is_the_identity_and_empty_ops_stay_empty() {
        let mut g = Region::from_rect(Rect::new(0, 0, 10, 10));
        let before = g.clone();
        g.xor_rect(Rect::new(3, 3, 20, 5));
        assert_ne!(g, before);
        g.xor_rect(Rect::new(3, 3, 20, 5));
        assert_eq!(g, before);
        let mut e = Region::new();
        e.and_rect(Rect::new(0, 0, 5, 5));
        assert!(e.is_empty() && e.bounds().is_empty());
    }
}
