//! Content planes (ruling 17, piece 5): QuickDraw is already a display-list API, so each window port keeps a retained
//! DISPLAY LIST next to its authoritative 1-bit canvas, and the host replays it with the kit's own typography and theme.
//!
//! * **The canvas stays the truth.** `CopyBits`/`GetPixel` readback is exact because the bitmap is still drawn; the list is
//!   a second, semantic account of the same drawing. Where the list cannot say a drawing EXACTLY, it says it in pixels:
//!   [`finish`](super::target::finish) records a `Pixels` op for every drawing no recorder below named (an icon blit, a
//!   `CopyBits`, a mixed-pattern transfer mode). Exactness holds by construction (every shortcut below is guarded so it cannot disagree with the bitmap); the route census shows how much of a
//!   window went the raw way.
//! * **Pens are roles, never colours.** [`ink`] maps a (transfer mode, pattern) pair to a theme role: black ink is `Text`,
//!   white is `Surface`, a half-tone is `TextMuted`. A pattern/mode pair with no role (xor with a gray) goes raw.
//!   `InvertRect` over plain surface is the Mac highlight idiom and records `Selection`; inverting twice cancels; inverting
//!   anything else (text, black) goes raw, because "invert" over text is not a colour.
//! * **Text** keeps its string, a font role (Monaco/Courier -> `Mono`, a bold face -> `Title`, everything else `Ui`) and the
//!   point size; the host picks the face. A run only partly inside the clip goes raw (a `Text` op has no clip).
//! * **The list stays bounded without a cap**: an opaque op drops every earlier op it fully covers (the `EraseRect` that
//!   starts every redraw empties the list), and inverting twice removes the highlight.
//! * **One present per step per window**, as `present_content` is. A host that refuses planes by name
//!   (`UiRefusal::Unsupported`) puts the engine back on the canvas route for good (`Planes::unsupported`).
use super::target::{Home, Target};
use crate::machine::Mac;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::{DrawOp, FontRole, GuestPoint, GuestRect, GuestUi, PenRole, Plane, UiRefusal};
use retro_raster::{Rect, Region};

/// Monaco (4) and Courier (22) are the monospaced Mac fonts; the face table is data, not a heuristic.
const MONO_FONTS: &[u16] = &[4, 22];
const DEFAULT_PX: u16 = 12;
const PLANE_ID: u32 = 0;

/// One recorded op and the pixel box it covers (for pruning and overlap tests).
struct Rec { op: DrawOp, bounds: Rect }

#[derive(Default)]
pub struct Planes {
    lists: BTreeMap<u32, Vec<Rec>>,
    dirty: BTreeMap<u32, Rect>,
    presented: BTreeSet<u32>,
    /// The host refused planes by name: the canvas route is used from now on and nothing more is recorded.
    pub unsupported: bool,
}

impl Planes {
    /// The recorded ops of a window, oldest first (what a present sends).
    pub fn ops(&self, win: u32) -> Vec<DrawOp> { self.lists.get(&win).map(|l| l.iter().map(|r| r.op.clone()).collect()).unwrap_or_default() }
    /// A window was resized or reshaped: its drawing is gone and the next present is a whole frame.
    pub fn reset(&mut self, win: u32) { self.lists.remove(&win); self.dirty.remove(&win); self.presented.remove(&win); }
    pub fn forget(&mut self, win: u32) { self.reset(win); }

    fn touch(&mut self, win: u32, r: Rect) { let d = self.dirty.entry(win).or_default(); *d = d.union(&r); }

    /// Add an op; an OPAQUE one first drops every op it covers completely.
    fn push(&mut self, win: u32, op: DrawOp, bounds: Rect, opaque: bool) {
        let list = self.lists.entry(win).or_default();
        if opaque { list.retain(|r| !covers(&bounds, &r.bounds)); }
        list.push(Rec { op, bounds });
        self.touch(win, bounds);
    }

    /// Remove the highlight `Fill` over exactly `r`, if it is the last thing drawn anywhere in `r` (an invert undone by
    /// inverting again). With drawing recorded after it the second invert is NOT an undo of the list: it goes raw instead.
    fn remove_selection(&mut self, win: u32, r: Rect) -> bool {
        let Some(list) = self.lists.get_mut(&win) else { return false };
        let at = list.iter().rposition(|x| x.bounds == r && matches!(x.op, DrawOp::Fill { pen_role: PenRole::Selection, .. }));
        match at {
            Some(i) if list[i + 1..].iter().all(|x| x.bounds.intersect(&r).is_empty()) => { list.remove(i); self.touch(win, r); true }
            _ => false,
        }
    }

    /// Is everything already drawn under `r` plain surface (so an invert there is a highlight)?
    fn only_surface_under(&self, win: u32, r: &Rect) -> bool {
        self.lists.get(&win).is_none_or(|l| l.iter().filter(|x| !x.bounds.intersect(r).is_empty()).all(|x| matches!(x.op, DrawOp::Fill { pen_role: PenRole::Surface, .. })))
    }
}

fn covers(outer: &Rect, inner: &Rect) -> bool { outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1 }
fn grect(r: &Rect) -> GuestRect { GuestRect { left: r.x0, top: r.y0, width: r.x1 - r.x0, height: r.y1 - r.y0 } }

/// What painting a (transfer mode, pattern) pair does, as a theme role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink { Role(PenRole), /** changes nothing */ Nothing, /** flips what is there */ Invert, /** no role: pixels */ Raw }

/// The pen table: transfer mode (8..15 pattern modes, 0..7 the same for source modes; `mode & 7` selects) x pattern class
/// (all-clear = white, all-set = black, anything else = half-tone) -> [`Ink`].
pub fn ink(mode: u16, pat: &[u8; 8]) -> Ink {
    let class = if *pat == [0xFF; 8] { 2 } else if *pat == [0; 8] { 0 } else { 1 };
    match (mode & 7, class) {
        (0, 2) | (1, 2) => Ink::Role(PenRole::Text),
        (0, 0) | (3, 2) | (4, 2) | (7, 0) => Ink::Role(PenRole::Surface),
        (0, 1) | (4, 1) => Ink::Role(PenRole::TextMuted),
        (4, 0) | (5, 0) => Ink::Role(PenRole::Text),
        (1, 0) | (3, 0) | (2, 0) | (5, 2) | (6, 2) | (7, 2) => Ink::Nothing,
        (2, 2) | (6, 0) => Ink::Invert,
        _ => Ink::Raw,
    }
}

/// The pixels of `r` (clamped to the canvas) as a raw op.
fn snapshot(t: &Target, r: Rect) -> Option<(DrawOp, Rect)> {
    let r = r.intersect(&t.bm.rect());
    if r.is_empty() { return None; }
    let mut rgb = Vec::with_capacity(((r.x1 - r.x0) * (r.y1 - r.y0)) as usize);
    for y in r.y0..r.y1 { for x in r.x0..r.x1 { rgb.push(t.host_color(t.bm.get(x, y).unwrap_or(0))); } }
    Some((DrawOp::Pixels { rect: grect(&r), rgb }, r))
}

fn recording(planes: &Planes, t: &Target) -> bool { !planes.unsupported && matches!(t.home, Home::Canvas) }

/// Record drawing nobody named: its pixels. Called by `finish` for every operation that did not record itself.
pub fn record_pixels(planes: &mut Planes, t: &Target, touched: Rect) {
    if !recording(planes, t) { return; }
    if let Some((op, r)) = snapshot(t, touched) { planes.push(t.port, op, r, true); }
}

/// A fill of `region` (PIXEL coordinates, already clipped) with (`mode`, `pat`), after the bitmap was drawn.
pub fn record_fill(planes: &mut Planes, t: &mut Target, region: &Region, mode: u16, pat: &[u8; 8]) {
    if !recording(planes, t) { return; }
    t.recorded = true;
    let win = t.port;
    // a colour window's pens are RGB pixels, not theme roles: only the raw route can say them exactly
    match if t.bm.depth() > 1 { Ink::Raw } else { ink(mode, pat) } {
        Ink::Nothing => {}
        Ink::Role(pen_role) => for r in region.rects() { planes.push(win, DrawOp::Fill { rect: grect(r), pen_role }, *r, true); },
        Ink::Invert => for r in region.rects() {
            if planes.remove_selection(win, *r) { continue; }
            if planes.only_surface_under(win, r) { planes.push(win, DrawOp::Fill { rect: grect(r), pen_role: PenRole::Selection }, *r, false); }
            else if let Some((op, b)) = snapshot(t, *r) { planes.push(win, op, b, true); }
        },
        Ink::Raw => if let Some((op, b)) = snapshot(t, region.bounds()) { planes.push(win, op, b, true); },
    }
}

/// The pen's 1x1 line from `a` to `b` (PIXEL coordinates) when nothing of it was clipped; `false` = the caller records the region.
pub fn record_line(planes: &mut Planes, t: &mut Target, a: (i32, i32), b: (i32, i32), mode: u16, pat: &[u8; 8]) -> bool {
    if !recording(planes, t) { return true; }
    if t.bm.depth() > 1 { return false; }
    let Ink::Role(pen_role) = ink(mode, pat) else { return false };
    t.recorded = true;
    let bounds = Rect::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0) + 1, a.1.max(b.1) + 1);
    planes.push(t.port, DrawOp::Line { a: GuestPoint { x: a.0, y: a.1 }, b: GuestPoint { x: b.0, y: b.1 }, pen_role }, bounds, false);
    true
}

/// A drawn text run, in pixel coordinates of the canvas.
pub struct Run<'a> {
    pub text: &'a [u8],
    /// The top-left of the glyph box the bitmap used (for bounds and the raw fallback).
    pub box_px: Rect,
    pub font: u16,
    pub face: u8,
    pub size: u16,
    pub mode: u16,
    pub back: &'a [u8; 8],
}

/// Mac Roman above 0x7F is shown as its Latin-1 neighbour; the glyph shapes come from the host's face either way.
fn to_string(b: &[u8]) -> String { b.iter().map(|&c| char::from(c)).collect() }

/// A text run drawn into the canvas. `clip` is the pixel clip the bitmap used.
pub fn record_text(planes: &mut Planes, t: &mut Target, run: &Run, clip: &Region) {
    if !recording(planes, t) { return; }
    t.recorded = true;
    let (win, b) = (t.port, run.box_px);
    let inside = clip.rects().iter().any(|r| covers(r, &b)) && t.bm.depth() == 1; // colour text goes raw (its pen is an RGB, not a role)
    let back = match ink(8, run.back) { Ink::Role(r) => Some(r), _ => None };
    // (ink role, background role) by text mode: srcCopy paints the box, srcOr only the glyphs, srcBic clears the glyphs,
    // notSrcCopy is white glyphs on black; every other mode has no role.
    let roles = match (run.mode & 7, back) { (0, Some(bg)) => Some((PenRole::Text, Some(bg))), (1, _) => Some((PenRole::Text, None)), (3, _) => Some((PenRole::Surface, None)), (4, _) => Some((PenRole::Surface, Some(PenRole::Text))), _ => None };
    let (Some((pen_role, bg_role)), true) = (roles, inside) else {
        if let Some((op, r)) = snapshot(t, b) { planes.push(win, op, r, true); }
        return;
    };
    let px = if run.size == 0 { DEFAULT_PX } else { run.size };
    let font_role = if run.face & 1 != 0 { FontRole::Title } else if MONO_FONTS.contains(&run.font) { FontRole::Mono } else { FontRole::Ui };
    let op = DrawOp::Text { at: GuestPoint { x: b.x0, y: b.y0 }, string: to_string(run.text), font_role, px, width: b.x1 - b.x0, pen_role, bg_role };
    planes.push(win, op, b, bg_role.is_some());
}

/// A colour window's colour table changed (`SetEntries`, an activated palette): what was recorded as RGB pixels is stale, so
/// the list becomes one raw frame of the canvas through the new table. The first call for a window only remembers its table.
pub fn palette_changed(planes: &mut Planes, win: u32, canvas: &retro_raster::Bitmap, palette: &[u32]) {
    planes.reset(win);
    let r = canvas.rect();
    let rgb = canvas.pixels().iter().map(|&p| palette.get(usize::from(p)).copied().unwrap_or(0)).collect();
    planes.push(win, DrawOp::Pixels { rect: grect(&r), rgb }, r, true);
}

/// Present every window's list: one `present_plane` per changed window, and the window's canvas damage is dropped (the
/// plane IS its presentation). Windows without a native window keep their list until they have one.
pub fn flush<U: GuestUi>(m: &mut Mac<U>) {
    if m.qd.plane.unsupported { return; }
    let wins: Vec<u32> = m.qd.canvases.keys().copied().collect();
    for win in wins {
        let Some(native) = m.windows.get(&win).and_then(|w| w.native) else { continue };
        let first = !m.qd.plane.presented.contains(&win);
        let dirty = m.qd.plane.dirty.remove(&win);
        if !first && dirty.is_none() { m.qd.present.forget(win); continue; }
        let rect = if first { m.qd.canvases[&win].rect() } else { dirty.unwrap_or_default() };
        match m.ui.present_plane(native, Plane { id: PLANE_ID, rect: grect(&rect), ops: m.qd.plane.ops(win) }) {
            Ok(()) => {
                m.qd.plane.presented.insert(win);
                m.qd.present.forget(win);
                let n = m.qd.plane.lists.get(&win).map_or(0, Vec::len);
                m.log.push(format!("[quickdraw] plane window {win:#x}: {n} op(s), repaint {}x{} at {},{}", rect.x1 - rect.x0, rect.y1 - rect.y0, rect.x0, rect.y0));
            }
            Err(UiRefusal::Unsupported(why)) => {
                m.log.push(format!("[quickdraw] planes not shown: {why}; the canvas route is used"));
                m.qd.plane.unsupported = true;
                m.qd.plane.lists.clear();
                m.qd.plane.dirty.clear();
                break;
            }
            Err(e) => m.refused.push(format!("[quickdraw] present_plane: {e}")),
        }
    }
}
