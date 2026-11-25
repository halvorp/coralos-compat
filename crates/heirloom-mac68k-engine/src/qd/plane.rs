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
use heirloom_ui_abi::{DrawOp, FontRole, GuestPoint, GuestRect, GuestUi, PenRole, Plane, UiRefusal};
use heirloom_raster::{Rect, Region};

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

/// Classify an RGB colour from a `CGrafPort` into a semantic theme role when it matches standard Mac monochrome or
/// Platinum UI chrome (black ink, neutral grey bevel/text, Platinum dialog surface, white shine). Chromatic colours
/// return `None` so artwork and custom-coloured graphics stay on the exact `DrawOp::Pixels` path.
fn is_neutral(c: crate::cqd::color::Rgb) -> bool {
    let (r, g, b) = (i32::from(c.r), i32::from(c.g), i32::from(c.b));
    (r - g).abs() <= 0x0808 && (g - b).abs() <= 0x0808 && (r - b).abs() <= 0x0808
}

fn color_text_role(fg: crate::cqd::color::Rgb) -> Option<PenRole> {
    if !is_neutral(fg) { return None; }
    let v = fg.r.max(fg.g).max(fg.b);
    if v <= 0x3333 { Some(PenRole::Text) }
    else if v <= 0xAAAA { Some(PenRole::TextMuted) }
    else if v >= 0xF000 { Some(PenRole::Surface) }
    else { None }
}

fn color_line_role(fg: crate::cqd::color::Rgb) -> Option<PenRole> {
    if !is_neutral(fg) { return None; }
    let v = fg.r.max(fg.g).max(fg.b);
    if v >= 0xF000 { Some(PenRole::Shine) }
    else if v <= 0x2FFF { Some(PenRole::Text) }
    else if v <= 0xCCCC { Some(PenRole::Shadow) }
    else { Some(PenRole::Surface) }
}

/// A fill of `region` (PIXEL coordinates, already clipped) with (`mode`, `pat`), after the bitmap was drawn.
pub fn record_fill(planes: &mut Planes, t: &mut Target, region: &Region, mode: u16, pat: &[u8; 8], fg_bg: (crate::cqd::color::Rgb, crate::cqd::color::Rgb)) {
    if !recording(planes, t) { return; }
    t.recorded = true;
    let win = t.port;
    let chosen = if t.bm.depth() > 1 {
        let solid_fg = *pat == [0xFF; 8];
        let solid_bg = *pat == [0; 8];
        if (mode & 7 == 0 || mode & 7 == 1) && (solid_fg || (mode & 7 == 0 && solid_bg)) {
            let c = if solid_fg { fg_bg.0 } else { fg_bg.1 };
            let b = region.bounds();
            let hairline = (b.x1 - b.x0) <= 1 || (b.y1 - b.y0) <= 1;
            let v = c.r.max(c.g).max(c.b);
            if is_neutral(c) && (0xC800..=0xF200).contains(&v) {
                Ink::Role(PenRole::Surface)
            } else if solid_bg && is_neutral(c) && v >= 0xF000 && (b.y1 - b.y0) <= 12 {
                // Single-line text/label background erase (e.g. TEUpdate on a dialog group-box title) paints dialog surface,
                // never a raw white strip in dark mode; taller white document canvases stay exact Pixels.
                Ink::Role(PenRole::Surface)
            } else if hairline && is_neutral(c) {
                if v >= 0xF000 { Ink::Role(PenRole::Shine) }
                else if v <= 0xB800 { Ink::Role(PenRole::Shadow) }
                else { Ink::Role(PenRole::Surface) }
            } else {
                Ink::Raw
            }
        } else {
            Ink::Raw
        }
    } else {
        ink(mode, pat)
    };
    match chosen {
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
pub fn record_line(planes: &mut Planes, t: &mut Target, a: (i32, i32), b: (i32, i32), mode: u16, pat: &[u8; 8], fg_bg: (crate::cqd::color::Rgb, crate::cqd::color::Rgb)) -> bool {
    if !recording(planes, t) { return true; }
    let pen_role = if t.bm.depth() > 1 {
        if *pat == [0xFF; 8] && (mode & 7 == 0 || mode & 7 == 1) {
            let Some(r) = color_line_role(fg_bg.0) else { return false };
            r
        } else {
            return false;
        }
    } else {
        let Ink::Role(r) = ink(mode, pat) else { return false };
        r
    };
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
    pub fg_rgb: crate::cqd::color::Rgb,
    pub bg_rgb: crate::cqd::color::Rgb,
}

/// Mac Roman text through [`crate::macroman`] (the one table); the glyph shapes come from the host's face.
fn to_string(b: &[u8]) -> String { b.iter().map(|&c| crate::macroman::to_char(c)).collect() }

/// A text run drawn into the canvas. `clip` is the pixel clip the bitmap used.
pub fn record_text(planes: &mut Planes, t: &mut Target, run: &Run, clip: &Region) {
    if !recording(planes, t) { return; }
    t.recorded = true;
    let (win, b) = (t.port, run.box_px);
    let inside = clip.rects().iter().any(|r| covers(r, &b));
    let roles = if t.bm.depth() > 1 {
        let fg_role = color_text_role(run.fg_rgb);
        let bg_light = is_neutral(run.bg_rgb) && run.bg_rgb.r.min(run.bg_rgb.g).min(run.bg_rgb.b) >= 0xBBBB;
        match (run.mode & 7, fg_role, bg_light) {
            (0, Some(r), true) => Some((r, Some(PenRole::Surface))),
            (1, Some(r), _) => Some((r, None)),
            _ => None,
        }
    } else {
        let back = match ink(8, run.back) { Ink::Role(r) => Some(r), _ => None };
        // (ink role, background role) by text mode: srcCopy paints the box, srcOr only the glyphs, srcBic clears the glyphs,
        // notSrcCopy is white glyphs on black; every other mode has no role.
        match (run.mode & 7, back) { (0, Some(bg)) => Some((PenRole::Text, Some(bg))), (1, _) => Some((PenRole::Text, None)), (3, _) => Some((PenRole::Surface, None)), (4, _) => Some((PenRole::Surface, Some(PenRole::Text))), _ => None }
    };
    let (Some((pen_role, bg_role)), true) = (roles, inside) else {
        if let Some((op, r)) = snapshot(t, b) { planes.push(win, op, r, true); }
        return;
    };
    let px = if run.size == 0 { DEFAULT_PX } else { run.size };
    let font_role = if run.face & 1 != 0 { FontRole::Title } else if MONO_FONTS.contains(&run.font) { FontRole::Mono } else { FontRole::Ui };
    let op = DrawOp::Text { at: GuestPoint { x: b.x0, y: b.y0 }, string: to_string(run.text), font_role, px, width: b.x1 - b.x0, pen_role, bg_role };
    planes.push(win, op, b, bg_role.is_some());
}

/// Record a small double-buffered single-line label blitted via `CopyBits` from an offscreen BitMap/PixMap onto a
/// window canvas (e.g. MacApp `TStaticText`) as a theme `Surface` fill + vector `Text` run.
#[allow(clippy::too_many_arguments)]
pub fn record_offscreen_label(planes: &mut Planes, t: &mut Target, dst: Rect, x: i32, y: i32, width: i32, font: u16, face: u8, size: u16, string: String) {
    if !recording(planes, t) { return; }
    t.recorded = true;
    let win = t.port;
    planes.push(win, DrawOp::Fill { rect: grect(&dst), pen_role: PenRole::Surface }, dst, true);
    let px = if size == 0 { DEFAULT_PX } else { size };
    let font_role = if face & 1 != 0 { FontRole::Title } else if MONO_FONTS.contains(&font) { FontRole::Mono } else { FontRole::Ui };
    let tb = Rect::new(x, y, x + width.max(1), y + i32::from(px));
    planes.push(win, DrawOp::Text { at: GuestPoint { x, y }, string, font_role, px, width, pen_role: PenRole::Text, bg_role: None }, tb, false);
}

/// A colour window's colour table changed (`SetEntries`, an activated palette): what was recorded as RGB pixels is stale, so
/// the list becomes one raw frame of the canvas through the new table. The first call for a window only remembers its table.
pub fn palette_changed(planes: &mut Planes, win: u32, canvas: &heirloom_raster::Bitmap, palette: &[u32]) {
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
        let mut ops = m.qd.plane.ops(win);
        if !m.tr.plane_chrome_bindings.is_empty() {
            let canvas_r = m.qd.canvases[&win].rect();
            let win_size = (canvas_r.x1 - canvas_r.x0, canvas_r.y1 - canvas_r.y0);
            let mut bound_edits: Vec<(u32, GuestRect, String, bool)> = Vec::new();
            for (bi, b) in m.tr.plane_chrome_bindings.iter().enumerate() {
                let heirloom_bind::PlaneChromeIdentity::Resource { res_type, res_id, length, fnv1a64 } = b.identity;
                let matched = m.rm.files.iter().rev().find_map(|f| f.fork.find(res_type, res_id).and_then(|e| f.fork.data(e)))
                    .is_some_and(|data| data.len() as u32 == length && heirloom_bind::wdef::fnv1a64(data) == fnv1a64);
                if matched {
                    let size_ok = b.window_range.is_none_or(|r| r.matches(win_size.0, win_size.1));
                    if size_ok {
                        let guards_ok = b.pixel_guards.iter().all(|g| {
                            heirloom_bind::plane_chrome::dark_mask_hash(&ops, g.rect, g.dark_luma) == Some(g.fnv1a64)
                        });
                        if guards_ok {
                            for (ri, rule) in b.palette_rules.iter().enumerate() {
                                if let heirloom_bind::plane_chrome::PaletteRuleKind::EditField { value, enabled } = &rule.kind {
                                    let tok = 0xBF00_0000u32 | (((bi as u32) & 0xFF) << 8) | ((ri as u32) & 0xFF);
                                    bound_edits.push((tok, rule.rect, value.clone(), *enabled));
                                }
                            }
                        }
                    }
                    ops = b.apply(ops, win_size);
                }
            }
            for (tok, rect, default_val, enabled) in bound_edits {
                let text = m.tr.bound_fields.entry(tok).or_insert((win, rect, default_val)).2.clone();
                if let Some(entry) = m.tr.bound_fields.get_mut(&tok) {
                    entry.0 = win;
                    entry.1 = rect;
                }
                let model = heirloom_ui_abi::ControlModel {
                    kind: heirloom_ui_abi::ControlKind::Field { text, max_len: 32, editable: true, integer: false },
                    label: String::new(),
                    rect,
                    enabled,
                    token: heirloom_ui_abi::CallbackToken(u64::from(tok)),
                };
                let _ = m.sync_projection(win, crate::translate::Obj::BoundField(tok), Some(model));
            }
        }
        match m.ui.present_plane(native, Plane { id: PLANE_ID, rect: grect(&rect), ops }) {
            Ok(()) => {
                m.qd.plane.presented.insert(win);
                m.qd.present.forget(win);
                let n = m.qd.plane.lists.get(&win).map_or(0, Vec::len);
                m.log.push(format!("[quickdraw] plane window {win:#x}: {n} op(s), repaint {}x{} at {},{}", rect.x1 - rect.x0, rect.y1 - rect.y0, rect.x0, rect.y0));
            }
            Err(UiRefusal::Unsupported(why)) => {
                if why.contains("frame limit") {
                    // One oversized pixel-heavy window (e.g. a 526x517 splash > MAX_FRAME) falls back to the
                    // shared-memory canvas route WITHOUT disabling planes + plane_chrome bindings for other windows.
                    m.log.push(format!("[quickdraw] plane window {win:#x} exceeded wire frame limit; falling back to canvas for this window"));
                } else {
                    m.log.push(format!("[quickdraw] planes not shown: {why}; the canvas route is used"));
                    m.qd.plane.unsupported = true;
                    m.qd.plane.lists.clear();
                    m.qd.plane.dirty.clear();
                    break;
                }
            }
            Err(e) => m.refused.push(format!("[quickdraw] present_plane: {e}")),
        }
    }
}

/// A theme part a program asked the Appearance Manager for inside its own content (`DrawThemePrimaryGroup`, `DrawThemeEditTextFrame`
/// ...): a toolkit contract (HEIRLOOM-TRANSLATION-BY-TOOLKIT-NOT-APP), so the plane gets the KIT's part (`op_of` makes it from the part's
/// pixel rectangle), never a hand-drawn Platinum look. `local` is in the current port's local coordinates. Nothing is drawn into the
/// canvas (the part is the kit's to draw); a port that is not a recorded window plane (an offscreen port, planes refused) draws nothing
/// and answers false, named by the caller.
pub fn record_theme_part<U: GuestUi>(m: &mut Mac<U>, local: (i32, i32, i32, i32), op_of: impl FnOnce(GuestRect) -> DrawOp) -> Result<bool, crate::Refusal> {
    let port = super::target::current_port(m)?;
    let mut done = false;
    super::target::with(m, port, |m, t| {
        if !recording(&m.qd.plane, t) { return Ok(Rect::default()); }
        t.recorded = true;
        let r = t.to_pixels(Rect::new(local.0, local.1, local.2, local.3));
        m.qd.plane.push(port, op_of(grect(&r)), r, false);
        done = true;
        Ok(Rect::default())
    })?;
    Ok(done)
}
