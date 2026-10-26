//! Half-open integer rectangles: `x0 <= x < x1`, `y0 <= y < y1`.
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Rect { pub x0: i32, pub y0: i32, pub x1: i32, pub y1: i32 }

impl Rect {
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect { Rect { x0, y0, x1, y1 } }
    /// From the inclusive corners Amiga APIs use (`xMin,yMin,xMax,yMax`).
    pub const fn from_inclusive(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect { Rect { x0, y0, x1: x1 + 1, y1: y1 + 1 } }
    pub const fn from_size(x: i32, y: i32, w: i32, h: i32) -> Rect { Rect { x0: x, y0: y, x1: x + w, y1: y + h } }
    pub const fn width(&self) -> i32 { self.x1 - self.x0 }
    pub const fn height(&self) -> i32 { self.y1 - self.y0 }
    pub const fn is_empty(&self) -> bool { self.x1 <= self.x0 || self.y1 <= self.y0 }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let r = Rect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        if r.is_empty() { Rect::default() } else { r }
    }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() { return *o; }
        if o.is_empty() { return *self; }
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn contains(&self, x: i32, y: i32) -> bool { x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1 }
    pub fn translate(&self, dx: i32, dy: i32) -> Rect { Rect { x0: self.x0 + dx, y0: self.y0 + dy, x1: self.x1 + dx, y1: self.y1 + dy } }
    /// `self` minus `o`: up to four disjoint rectangles covering the rest.
    pub fn subtract(&self, o: &Rect) -> Vec<Rect> {
        let i = self.intersect(o);
        if i.is_empty() { return if self.is_empty() { Vec::new() } else { alloc::vec![*self] }; }
        let mut v = Vec::new();
        if i.y0 > self.y0 { v.push(Rect::new(self.x0, self.y0, self.x1, i.y0)); }
        if i.x0 > self.x0 { v.push(Rect::new(self.x0, i.y0, i.x0, i.y1)); }
        if i.x1 < self.x1 { v.push(Rect::new(i.x1, i.y0, self.x1, i.y1)); }
        if i.y1 < self.y1 { v.push(Rect::new(self.x0, i.y1, self.x1, self.y1)); }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtract_leaves_the_complement() {
        let a = Rect::new(0, 0, 10, 10);
        let parts = a.subtract(&Rect::new(3, 3, 6, 6));
        assert_eq!(parts.iter().map(|r| r.width() * r.height()).sum::<i32>(), 100 - 9);
        assert!(parts.iter().all(|p| p.intersect(&Rect::new(3, 3, 6, 6)).is_empty()));
        assert_eq!(a.subtract(&Rect::new(20, 20, 30, 30)), alloc::vec![a], "negative control: disjoint is unchanged");
        assert!(a.subtract(&a).is_empty());
    }
    #[test]
    fn inclusive_corners_cover_one_more() {
        assert_eq!(Rect::from_inclusive(2, 3, 4, 3), Rect::new(2, 3, 5, 4));
        assert!(Rect::new(5, 5, 5, 9).is_empty());
    }
}
