//! Styled TextEdit's data model as pure functions (Inside Macintosh VI "Text Edit Styles"; Executor's MIT `teIMV.cpp` and `tesave.h`
//! for the layouts and rules). A styled record keeps its `TEStyleHandle` in the four bytes at `txFont` (`txSize` is -1,
//! `lineHeight` and `fontAscent` -1), and the style data lives in guest memory exactly as the ROM keeps it:
//!
//! ```text
//! TEStyleRec: nRuns 0  nStyles 2  styleTab 4 (STHandle)  lhTab 8 (LHHandle)  teRefCon 12  nullStyle 16 (NullStHandle)  runs 20..
//!   StyleRun (4 bytes): startChar (word), styleIndex (word); runs[nRuns] is the sentinel { teLength + 1, -1 }
//! STElement (18): stCount 0  stHeight 2  stAscent 4  stFont 6  stFace 8 (byte, +1 pad)  stSize 10  stColor 12 (RGBColor)
//! LHElement (4): lhHeight, lhAscent         NullStRec (8): teReserved (long), nullScrap (StScrpHandle)
//! StScrpRec: scrpNStyles (word), ScrpSTElement[] (20): scrpStartChar (long)  scrpHeight  scrpAscent  scrpFont  scrpFace  scrpSize  scrpColor
//! TextStyle (12): tsFont (word)  tsFace (byte, +1 pad)  tsSize (word)  tsColor (RGBColor)
//! ```
//! The model here is a run list over per-character style indices, so an edit is "expand, splice, re-encode".
use alloc::vec::Vec;

pub const DO_FONT: i16 = 1;
pub const DO_FACE: i16 = 2;
pub const DO_SIZE: i16 = 4;
pub const DO_COLOR: i16 = 8;
pub const ADD_SIZE: i16 = 16;
pub const DO_TOGGLE: i16 = 32;

pub const STYLE_REC_HEAD: u32 = 20;
pub const ST_ELEMENT: u32 = 18;
pub const SCRAP_ELEMENT: u32 = 20;
pub const TEXT_STYLE: u32 = 12;

/// A `TextStyle`: what a run is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs { pub font: i16, pub face: u8, pub size: i16, pub color: [u16; 3] }

impl Attrs {
    pub fn read(b: &[u8]) -> Option<Attrs> {
        let w = |i: usize| Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]));
        Some(Attrs { font: w(0)? as i16, face: *b.get(2)?, size: w(4)? as i16, color: [w(6)?, w(8)?, w(10)?] })
    }
    pub fn write(&self) -> [u8; 12] {
        let mut o = [0u8; 12];
        o[0..2].copy_from_slice(&(self.font as u16).to_be_bytes());
        o[2] = self.face;
        o[4..6].copy_from_slice(&(self.size as u16).to_be_bytes());
        for (i, c) in self.color.iter().enumerate() { o[6 + 2 * i..8 + 2 * i].copy_from_slice(&c.to_be_bytes()); }
        o
    }
}

/// One entry of the style table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleEl { pub count: i16, pub height: i16, pub ascent: i16, pub attrs: Attrs }

/// The style data of one record, decoded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleTab {
    /// (startChar, styleIndex), without the sentinel.
    pub runs: Vec<(u16, i16)>,
    pub styles: Vec<StyleEl>,
}

/// Each character's style index.
pub fn expand(runs: &[(u16, i16)], len: usize) -> Vec<i16> {
    let mut v = alloc::vec![0i16; len];
    for (i, &(start, idx)) in runs.iter().enumerate() {
        let end = runs.get(i + 1).map_or(len, |r| usize::from(r.0)).min(len);
        for c in v.iter_mut().take(end).skip(usize::from(start).min(len)) { *c = idx; }
    }
    v
}

/// Runs from per-character indices: adjacent equal indices merge; text with no characters keeps one run at 0 (style `fallback`).
pub fn compress(per_char: &[i16], fallback: i16) -> Vec<(u16, i16)> {
    let mut runs: Vec<(u16, i16)> = Vec::new();
    for (i, &s) in per_char.iter().enumerate() {
        if runs.last().map(|r| r.1) != Some(s) { runs.push((i as u16, s)); }
    }
    if runs.is_empty() { runs.push((0, fallback)); }
    runs
}

/// `adjust_attrs` (Executor teIMV.cpp): the attributes `mode` selects come from `new`, the rest stay. `continuous` is the selection's
/// common face for `doToggle`.
pub fn adjust(orig: &Attrs, new: &Attrs, mode: i16, continuous: Option<&Attrs>) -> Attrs {
    let mut d = *orig;
    if mode & DO_FONT != 0 { d.font = new.font; }
    if mode & DO_FACE != 0 {
        d.face = match (mode & DO_TOGGLE != 0, continuous) {
            (true, Some(c)) => (orig.face & !new.face) | (new.face ^ c.face),
            _ => new.face,
        };
    }
    if mode & ADD_SIZE != 0 { d.size = new.size.wrapping_add(orig.size); } else if mode & DO_SIZE != 0 { d.size = new.size; }
    if mode & DO_COLOR != 0 { d.color = new.color; }
    d
}

impl StyleTab {
    /// The index of a style with these attributes, adding an element (with the given metrics) if there is none.
    pub fn index_of(&mut self, a: &Attrs, height: i16, ascent: i16) -> i16 {
        if let Some(i) = self.styles.iter().position(|s| s.attrs == *a) { return i as i16; }
        self.styles.push(StyleEl { count: 0, height, ascent, attrs: *a });
        (self.styles.len() - 1) as i16
    }
    /// Recount how many runs use each style and drop styles no run uses (indices are renumbered; at least one style stays).
    pub fn stabilise(&mut self) {
        let mut used = alloc::vec![0i16; self.styles.len()];
        for &(_, s) in &self.runs { if let Some(u) = used.get_mut(s as usize) { *u += 1; } }
        if used.iter().all(|&u| u == 0) { return; }
        let mut remap = alloc::vec![-1i16; self.styles.len()];
        let mut kept = Vec::new();
        for (i, st) in self.styles.iter().enumerate() {
            if used[i] > 0 { remap[i] = kept.len() as i16; kept.push(StyleEl { count: used[i], ..*st }); }
        }
        for r in &mut self.runs { r.1 = remap.get(r.1 as usize).copied().unwrap_or(0).max(0); }
        self.styles = kept;
    }
    /// Apply `f` to the style of every character in `start..stop`, re-encoding the runs.
    pub fn map_range(&mut self, len: usize, start: usize, stop: usize, mut f: impl FnMut(&mut Self, Attrs) -> (Attrs, i16, i16)) {
        let mut per = expand(&self.runs, len);
        let mut cache: Vec<(i16, i16)> = Vec::new(); // old index -> new index
        for c in per.iter_mut().take(stop.min(len)).skip(start.min(len)) {
            if let Some(&(_, n)) = cache.iter().find(|(o, _)| o == c) { *c = n; continue; }
            let old = *c;
            let attrs = self.styles.get(old as usize).map_or(Attrs::default(), |s| s.attrs);
            let (na, h, a) = f(self, attrs);
            let n = self.index_of(&na, h, a);
            cache.push((old, n));
            *c = n;
        }
        self.runs = compress(&per, 0);
        self.stabilise();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(font: i16, face: u8, size: i16) -> Attrs { Attrs { font, face, size, color: [0; 3] } }

    #[test]
    fn runs_expand_and_compress_round_trip_and_adjacent_equal_styles_merge() {
        let runs = [(0u16, 0i16), (3, 1), (5, 0)];
        let per = expand(&runs, 8);
        assert_eq!(per, [0, 0, 0, 1, 1, 0, 0, 0]);
        assert_eq!(compress(&per, 0), runs);
        assert_eq!(compress(&[2, 2, 2], 0), [(0, 2)]);
        assert_eq!(compress(&[], 7), [(0, 7)], "empty text keeps one run");
    }

    #[test]
    fn adjust_changes_only_what_the_mode_names() {
        let o = a(3, 1, 12);
        let n = a(22, 2, 18);
        assert_eq!(adjust(&o, &n, DO_FONT, None), a(22, 1, 12));
        assert_eq!(adjust(&o, &n, DO_FACE, None), a(3, 2, 12));
        assert_eq!(adjust(&o, &n, DO_SIZE, None), a(3, 1, 18));
        assert_eq!(adjust(&o, &n, ADD_SIZE, None), a(3, 1, 30), "addSize adds");
        assert_eq!(adjust(&o, &n, 0, None), o, "mode 0 changes nothing");
        // toggle: orig bold(1), new = bold(1), continuous = bold -> the face bit comes OFF
        assert_eq!(adjust(&a(3, 1, 12), &a(0, 1, 0), DO_FACE | DO_TOGGLE, Some(&a(0, 1, 0))).face, 0);
        assert_eq!(adjust(&a(3, 0, 12), &a(0, 1, 0), DO_FACE | DO_TOGGLE, Some(&a(0, 0, 0))).face, 1, "toggling an unset bit sets it");
    }

    #[test]
    fn map_range_restyles_a_range_splits_runs_and_drops_styles_nobody_uses() {
        let mut t = StyleTab { runs: alloc::vec![(0, 0)], styles: alloc::vec![StyleEl { count: 1, height: 10, ascent: 8, attrs: a(3, 0, 12) }] };
        t.map_range(10, 2, 5, |_, o| (Attrs { face: 1, ..o }, 10, 8));
        assert_eq!(t.runs, [(0, 0), (2, 1), (5, 0)]);
        assert_eq!(t.styles.len(), 2);
        assert_eq!(t.styles[0].count, 2, "two runs use plain");
        // restyle everything bold: the plain style is no longer used and goes
        t.map_range(10, 0, 10, |_, o| (Attrs { face: 1, ..o }, 10, 8));
        assert_eq!((t.runs.clone(), t.styles.len(), t.styles[0].attrs.face), (alloc::vec![(0, 0)], 1, 1));
    }
}
