//! Off-screen BitMap display lists (the plane route for double-buffered programs: a button bank or a file lister drawn off screen, then copied in):
//! the named ops and holes drawn into a bitmap that is NOT a window canvas, in BITMAP coordinates, pruned by the same rule as a
//! window's list (`heirloom_present::plane::prune_recs`). A copy blit into a window splices them (blits.rs); anything the engine
//! cannot vouch for (an unnamed draw, a guest write to the planes behind its back: `sum`) is a hole.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use heirloom_present::plane::{prune_recs, push_rec, Rec};
use heirloom_raster::Rect;

#[derive(Default)]
pub struct Surface {
    /// Bumped on every invalidation (diagnostics, tests).
    pub gen: u64,
    pub rec: Vec<Rec>,
    /// FNV-1a over the plane bytes as the engine last left them (bitmaps.rs); a different sum on the next read = a foreign write.
    pub sum: u64,
    pub bitmap_rect: Rect,
}

#[derive(Default)]
pub struct Surfaces { by_bm: BTreeMap<u32, Surface> }

impl Surfaces {
    pub fn new() -> Self { Self::default() }
    pub fn get(&self, bm: u32) -> Option<&Surface> { self.by_bm.get(&bm) }
    pub fn get_mut(&mut self, bm: u32) -> Option<&mut Surface> { self.by_bm.get_mut(&bm) }
    pub fn entry(&mut self, bm: u32) -> &mut Surface { self.by_bm.entry(bm).or_default() }
    pub fn remove(&mut self, bm: u32) -> Option<Surface> { self.by_bm.remove(&bm) }
    /// Forget every op of `bm`: one hole over `whole`, a new generation.
    pub fn invalidate(&mut self, bm: u32, whole: Rect) {
        let s = self.entry(bm);
        s.rec.clear();
        s.rec.push(Rec::Hole(whole));
        s.gen += 1;
    }
    /// The recs of `bm` intersecting `r`, cloned so the caller can drop the borrow before recording elsewhere.
    pub fn slice(&self, bm: u32, r: Rect) -> Vec<Rec> {
        self.by_bm.get(&bm).map(|s| s.rec.iter().filter(|rec| !rec.bbox().intersect(&r).is_empty()).cloned().collect()).unwrap_or_default()
    }
    pub fn prune(&mut self, bm: u32) { if let Some(s) = self.by_bm.get_mut(&bm) { s.prune(); } }
}

impl Surface {
    pub fn new(bitmap_rect: Rect) -> Self { Self { bitmap_rect, ..Self::default() } }
    pub fn add_rect(&mut self, rec: Rec) { push_rec(&mut self.rec, rec); self.prune(); }
    pub fn prune(&mut self) { prune_recs(&mut self.rec, self.bitmap_rect); }
}

pub fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for &b in bytes { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::{DrawOp, PenRole};

    fn fill(r: Rect) -> Rec {
        Rec::Op { op: DrawOp::Fill { rect: heirloom_ui_abi::GuestRect { left: r.x0, top: r.y0, width: r.width(), height: r.height() }, pen_role: PenRole::Background }, bbox: r }
    }

    #[test]
    fn a_later_hole_drops_only_the_ops_it_covers() {
        let whole = Rect::from_size(0, 0, 100, 100);
        let inside = Rect::from_size(10, 10, 8, 8);
        let outside = Rect::from_size(80, 80, 8, 8);
        let mut s = Surface::new(whole);
        s.add_rect(fill(inside));
        s.add_rect(fill(outside));
        s.add_rect(Rec::Hole(Rect::from_size(0, 0, 50, 50)));
        assert_eq!(s.rec.len(), 2);
        assert!(matches!(s.rec[0], Rec::Op { .. }));
        assert_eq!(s.rec[0].bbox(), outside);
        assert!(matches!(s.rec[1], Rec::Hole(_)));
    }

    #[test]
    fn slicing_clones_intersections_and_invalidation_starts_a_new_generation() {
        let whole = Rect::from_size(0, 0, 100, 100);
        let mut all = Surfaces::new();
        all.entry(7).bitmap_rect = whole;
        all.entry(7).add_rect(fill(Rect::from_size(2, 2, 4, 4)));
        assert_eq!(all.slice(7, Rect::from_size(20, 20, 4, 4)).len(), 0);
        assert_eq!(all.slice(7, Rect::from_size(3, 3, 4, 4)).len(), 1);
        all.invalidate(7, whole);
        assert_eq!(all.get(7).unwrap().gen, 1);
        assert!(matches!(all.get(7).unwrap().rec[0], Rec::Hole(_)));
    }

    #[test]
    fn fnv_is_offset_basis_for_nothing_and_changes_with_plane_bytes() {
        assert_eq!(fnv1a(0xcbf29ce484222325, &[]), 0xcbf29ce484222325);
        assert_ne!(fnv1a(0xcbf29ce484222325, &[0]), fnv1a(0xcbf29ce484222325, &[1]));
    }
}