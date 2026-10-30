//! The plane route (ruling 17, piece 5): drawing is RECORDED as a display list per window, next to the authoritative guest
//! bitmap, and published once per step as `GuestUi::present_plane`. The list holds NAMED ops ([`Rec::Op`]: text runs, fills,
//! lines, bevels, icons, with pen ROLES) and HOLES ([`Rec::Hole`]: a rectangle the engine drew with something it cannot name,
//! a blit, a flood, a pixel array). A hole becomes a `DrawOp::Pixels` read from the canvas at flush time, so the plane is
//! always complete: the raw route is a part of a plane, counted by the route census, never a second channel.
//!
//! Coordinates in here are CANVAS coordinates; `Layout::offset` moves them to window coordinates at flush. Regions that sit
//! under a native control (`Layout::exclude`, canvas coordinates) are not published: the host draws the control there.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{DrawOp, GuestPoint, GuestRect, GuestUi, PenRole, Plane, UiRefusal};
use heirloom_raster::Rect;

use crate::{Presenter, Source};

/// The id of a window's content plane.
pub const CONTENT_PLANE: u32 = 0;
/// A display list longer than this after occlusion pruning has its OLDEST half collapsed into one hole over their union (the honest
/// raw fallback: those pixels are read from the canvas). Newer recs stay named, and an opaque rec that later covers the hole prunes it.
const MAX_RECS: usize = 4096;

/// One recorded drawing step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rec {
    /// A named op (canvas coordinates) and the canvas rectangle it touches.
    Op { op: DrawOp, bbox: Rect },
    /// Unnamed drawing over this canvas rectangle.
    Hole(Rect),
}

impl Rec {
    pub fn bbox(&self) -> Rect { match self { Rec::Op { bbox, .. } => *bbox, Rec::Hole(r) => *r } }
    /// Does this rec paint every pixel of its box opaquely (so anything under it is dead)? A fill (a theme role or a Swatch in the guest's exact colour), a hole, a bound Face (the
    /// kit face covers its box, so the program's own drawing wholly under it never shows at the face's rounded corners), and a text
    /// run that paints its cell first (`bg_role` Some: JAM2) do; a JAM1 run, a line, a bevel edge or an icon leave pixels under them.
    pub fn opaque(&self) -> bool {
        matches!(self, Rec::Hole(_) | Rec::Op { op: DrawOp::Fill { .. } | DrawOp::Swatch { .. } | DrawOp::Face { .. } | DrawOp::Text { bg_role: Some(_), .. }, .. })
    }
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    !a.intersect(&b).is_empty()
}

/// Push a rec, folding a shadow/emboss text pass into its foreground.
///
/// Programs draw embossed labels as two text passes at a 1-2px offset
/// (shadow then foreground). The kit draws each run in its own typography,
/// so replaying both would print the label twice, slightly offset. When `rec` is a transparent (`bg_role: None`)
/// `DrawOp::Text` run, this walks backwards over at most the last 8 recs
/// looking for a matching text run (same `string`, `font_role`, `px`,
/// `bg_role: None`) whose `at` is within 2px on both axes (not both 0).
/// If found, the earlier (shadow) rec is removed and the new (foreground)
/// rec is pushed — the foreground's `pen_role` wins. An exact same-position
/// repeat is left to `prune_recs`. The walk stops at the first rec whose
/// bbox intersects the new rec's bbox and is not a transparent text run
/// (something was drawn in between: do not merge across it).
pub fn push_rec(v: &mut Vec<Rec>, rec: Rec) {
    let merge_index: Option<usize> = {
        let new_bbox = rec.bbox();
        match &rec {
            Rec::Op {
                op: DrawOp::Text {
                    string: new_string,
                    font_role: new_font_role,
                    px: new_px,
                    at: new_at,
                    bg_role: None,
                    ..
                },
                ..
            } => {
                let start = v.len().saturating_sub(8);
                let mut found = None;
                for i in (start..v.len()).rev() {
                    let candidate = &v[i];
                    let is_transp_text = matches!(
                        candidate,
                        Rec::Op {
                            op: DrawOp::Text { bg_role: None, .. },
                            ..
                        }
                    );
                    if !is_transp_text && rects_intersect(candidate.bbox(), new_bbox) {
                        break;
                    }
                    if let Rec::Op {
                        op: DrawOp::Text {
                            string,
                            font_role,
                            px,
                            at,
                            bg_role: None,
                            ..
                        },
                        ..
                    } = candidate
                    {
                        if string == new_string
                            && font_role == new_font_role
                            && px == new_px
                        {
                            let dx = at.x - new_at.x;
                            let dy = at.y - new_at.y;
                            if dx.abs() <= 2 && dy.abs() <= 2 && (dx != 0 || dy != 0) {
                                found = Some(i);
                                break;
                            }
                        }
                    }
                }
                found
            }
            _ => None,
        }
    };
    if let Some(i) = merge_index {
        v.remove(i);
    }
    v.push(rec);
}

/// Drop every rec an opaque later rec fully covers; collapse a list still over [`MAX_RECS`] into one hole over `canvas`. Shared by
/// the window lists here and an engine's off-screen bitmap lists (ami68k graphics/surface.rs).
/// Returns the hole a collapse made, if one did.
pub fn prune_recs(v: &mut Vec<Rec>, canvas: Rect) -> Option<Rect> {
    let mut keep = alloc::vec![true; v.len()];
    let mut covers: Vec<Rect> = Vec::new();
    for (i, r) in v.iter().enumerate().rev() {
        let b = r.bbox();
        if covers.iter().any(|c| c.intersect(&b) == b) { keep[i] = false; continue }
        if r.opaque() { covers.push(b); }
    }
    let mut it = keep.iter();
    v.retain(|_| *it.next().unwrap());
    if v.len() > MAX_RECS {
        let old: Vec<Rec> = v.drain(..v.len() - MAX_RECS / 2).collect();
        let hole = old.iter().fold(Rect::default(), |u, r| u.union(&r.bbox())).intersect(&canvas);
        if !hole.is_empty() { v.insert(0, Rec::Hole(hole)); return Some(hole); }
    }
    None
}

/// How a window's canvas sits in its window: the canvas -> window offset, and the canvas rectangles native controls cover.
#[derive(Debug, Clone, Default)]
pub struct Layout { pub offset: (i32, i32), pub exclude: Vec<Rect> }

fn grect(r: &Rect) -> GuestRect { GuestRect { left: r.x0, top: r.y0, width: r.width(), height: r.height() } }

/// `op` moved by (dx, dy): a display list spliced from one surface into another (ami68k graphics/surface.rs).
pub fn moved(op: &DrawOp, (dx, dy): (i32, i32)) -> DrawOp {
    let r = |g: &GuestRect| GuestRect { left: g.left + dx, top: g.top + dy, ..*g };
    let p = |g: &GuestPoint| GuestPoint { x: g.x + dx, y: g.y + dy };
    match op {
        DrawOp::Fill { rect, pen_role } => DrawOp::Fill { rect: r(rect), pen_role: *pen_role },
        DrawOp::Text { at, string, font_role, px, width, pen_role, bg_role } => DrawOp::Text { at: p(at), string: string.clone(), font_role: *font_role, px: *px, width: *width, pen_role: *pen_role, bg_role: *bg_role },
        DrawOp::Line { a, b, pen_role } => DrawOp::Line { a: p(a), b: p(b), pen_role: *pen_role },
        DrawOp::Bevel { rect, raised } => DrawOp::Bevel { rect: r(rect), raised: *raised },
        DrawOp::Icon { rect, role } => DrawOp::Icon { rect: r(rect), role: *role },
        DrawOp::Pixels { rect, rgb } => DrawOp::Pixels { rect: r(rect), rgb: rgb.clone() },
        DrawOp::Swatch { rect, rgb } => DrawOp::Swatch { rect: r(rect), rgb: *rgb },
        DrawOp::Face { rect, label, pressed, accent } => DrawOp::Face { rect: r(rect), label: label.clone(), pressed: *pressed, accent: *accent },
    }
}

/// Merge a run of SHINE lines followed by a run of SHADOW lines (or the reverse) that together form a box edge into one
/// `Bevel`. The roles come from the guest's DrawInfo pen table, not from the picture; the geometry only decides that the
/// two runs are the lit and the shaded edges of ONE rectangle. `ops` are in the order drawn.
pub fn coalesce_bevels(ops: Vec<Rec>) -> Vec<Rec> {
    fn line(r: &Rec) -> Option<(Rect, PenRole)> {
        match r { Rec::Op { op: DrawOp::Line { pen_role, .. }, bbox } if matches!(pen_role, PenRole::Shine | PenRole::Shadow) => Some((*bbox, *pen_role)), _ => None }
    }
    let (mut out, mut i) = (Vec::with_capacity(ops.len()), 0);
    while i < ops.len() {
        let Some((_, first)) = line(&ops[i]) else { out.push(ops[i].clone()); i += 1; continue };
        let mut j = i;
        let (mut lit, mut dark) = (Rect::default(), Rect::default());
        while j < ops.len() { match line(&ops[j]) { Some((b, role)) if role == first => { lit = lit.union(&b); j += 1 } _ => break } }
        let mut k = j;
        while k < ops.len() { match line(&ops[k]) { Some((b, role)) if role != first => { dark = dark.union(&b); k += 1 } _ => break } }
        let whole = lit.union(&dark);
        // lit edges: left column + top row; shaded: right column + bottom row (inclusive pixel boxes)
        let on_edges = |run: &[Rec], a: &dyn Fn(&Rect) -> bool| run.iter().all(|r| line(r).is_some_and(|(b, _)| a(&b)));
        let (left, top, right, bottom) = (whole.x0, whole.y0, whole.x1 - 1, whole.y1 - 1);
        let lit_edge = |b: &Rect| (b.x0 == left && b.x1 - 1 == left) || (b.y0 == top && b.y1 - 1 == top);
        let dark_edge = |b: &Rect| (b.x0 == right && b.x1 - 1 == right) || (b.y0 == bottom && b.y1 - 1 == bottom);
        // the pair must DRAW the whole outline (the four corner pixels aside): a lone lit row with a lone shaded column is not a box
        let drawn = |x: i32, y: i32| ops[i..k].iter().any(|r| line(r).is_some_and(|(b, _)| b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1));
        let outline = (left + 1..right).all(|x| drawn(x, top) && drawn(x, bottom)) && (top + 1..bottom).all(|y| drawn(left, y) && drawn(right, y));
        if k > j && whole.width() > 2 && whole.height() > 2 && on_edges(&ops[i..j], &lit_edge) && on_edges(&ops[j..k], &dark_edge) && outline {
            out.push(Rec::Op { op: DrawOp::Bevel { rect: grect(&whole), raised: first == PenRole::Shine }, bbox: whole });
            i = k;
        } else { out.push(ops[i].clone()); i += 1; }
    }
    out
}

impl Presenter {
    /// Turn the plane route on or off for this presenter (off = every window goes out as `present_content`, as before).
    pub fn set_plane_route(&mut self, on: bool) { self.plane_route = on; }
    pub fn plane_route(&self) -> bool { self.plane_route }

    /// Record a named op over `bbox` (canvas coordinates) for window `win`.
    pub fn record(&mut self, win: u32, op: DrawOp, bbox: Rect) { if !bbox.is_empty() { push_rec(self.recs.entry(win).or_default(), Rec::Op { op, bbox }); } }
    /// Record drawing the engine cannot name over `r`.
    pub fn record_hole(&mut self, win: u32, r: Rect) { if !r.is_empty() { push_rec(self.recs.entry(win).or_default(), Rec::Hole(r)); } }
    /// The display list of a window (diagnostics, tests).
    pub fn recs(&self, win: u32) -> &[Rec] { self.recs.get(&win).map_or(&[], |v| v.as_slice()) }

    /// Drop every rec an opaque later rec fully covers; collapse a list still over the cap into one hole over `canvas`.
    fn prune(&mut self, win: u32, canvas: Rect) {
        if let Some(hole) = self.recs.get_mut(&win).and_then(|v| prune_recs(v, canvas)) { self.why_hole("window list over its cap: the oldest half collapsed", hole); }
    }

    /// Flush every damaged window as one `present_plane` carrying the display list for the damage rectangle. A host that
    /// answers `Unsupported` gets the same frame as `present_content` (the canvas route) instead. `source` resolves a window
    /// key as in [`Presenter::flush`], plus how its canvas sits in the window.
    pub fn flush_planes<'a, U: GuestUi>(&mut self, ui: &mut U, tag: &str, log: &mut Vec<String>, refused: &mut Vec<String>, mut source: impl FnMut(u32) -> Option<(Source<'a>, Layout)>) {
        for (win, mut r) in core::mem::take(&mut self.damage) {
            let Some((s, layout)) = source(win) else { continue };
            let canvas = s.canvas.rect();
            if !self.presented.contains(&win) { r = canvas; }
            let r = r.intersect(&canvas);
            if r.is_empty() { continue; }
            let hidden = |b: &Rect| layout.exclude.iter().any(|e| e.intersect(b) == *b);
            let mut ops = Vec::new();
            for rec in coalesce_bevels(self.recs.get(&win).cloned().unwrap_or_default()) {
                match rec {
                    Rec::Op { op, bbox } if !bbox.intersect(&r).is_empty() && !hidden(&bbox) => ops.push(moved(&op, layout.offset)),
                    Rec::Hole(h) => {
                        let mut pieces = alloc::vec![h.intersect(&r)];
                        for e in &layout.exclude { pieces = pieces.iter().flat_map(|p| p.subtract(e)).collect(); }
                        for p in pieces.into_iter().filter(|p| !p.is_empty()) {
                            let mut rgb = Vec::with_capacity((p.width() * p.height()) as usize);
                            for y in p.y0..p.y1 { rgb.extend(s.canvas.row(y)[p.x0 as usize..p.x1 as usize].iter().map(|&px| s.palette[px as usize])); }
                            ops.push(DrawOp::Pixels { rect: grect(&p.translate(layout.offset.0, layout.offset.1)), rgb });
                        }
                    }
                    _ => {}
                }
            }
            let window_rect = r.translate(layout.offset.0, layout.offset.1);
            let plane = Plane { id: CONTENT_PLANE, rect: grect(&window_rect), ops };
            match ui.present_plane(s.native, plane) {
                Ok(()) => {
                    self.presented.insert(win);
                    log.push(format!("{tag} plane window {win:#x}: {}x{} at {},{}", r.width(), r.height(), window_rect.x0, window_rect.y0));
                    self.prune(win, canvas);
                }
                Err(UiRefusal::Unsupported(_)) => {
                    // FALLBACK-IS-THE-DESIGN: the host has no plane part; the same damage goes out as canvas pixels.
                    if !self.plane_refused_once { self.plane_refused_once = true; log.push(format!("{tag} plane window {win:#x} not shown by the host: canvas route")); }
                    self.damage.insert(win, r);
                    self.flush_canvas_only(ui, tag, log, refused, win, &mut source);
                }
                Err(e) => refused.push(format!("{tag} present_plane: {e}")),
            }
        }
    }

    fn flush_canvas_only<'a, U: GuestUi>(&mut self, ui: &mut U, tag: &str, log: &mut Vec<String>, refused: &mut Vec<String>, win: u32, source: &mut impl FnMut(u32) -> Option<(Source<'a>, Layout)>) {
        let keep: Vec<(u32, Rect)> = self.damage.iter().filter(|(w, _)| **w != win).map(|(w, r)| (*w, *r)).collect();
        self.damage.retain(|w, _| *w == win);
        self.flush(ui, tag, log, refused, |w| source(w).map(|(s, _)| s));
        for (w, r) in keep { self.damage.insert(w, r); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::{CallbackToken, RecordingGuestUi, UiCall, WindowModel, WindowRole};
    use heirloom_raster::Bitmap;

    fn line(x0: i32, y0: i32, x1: i32, y1: i32, role: PenRole) -> Rec {
        Rec::Op { op: DrawOp::Line { a: GuestPoint { x: x0, y: y0 }, b: GuestPoint { x: x1, y: y1 }, pen_role: role }, bbox: Rect::new(x0.min(x1), y0.min(y1), x0.max(x1) + 1, y0.max(y1) + 1) }
    }
    fn window(ui: &mut RecordingGuestUi) -> heirloom_ui_abi::UiObjectRef {
        ui.open_window(WindowModel { title: "t".into(), rect: GuestRect { left: 0, top: 0, width: 8, height: 6 }, role: WindowRole::Document, closable: true, resizable: false, token: CallbackToken(1) }).unwrap()
    }

    #[test]
    fn a_lit_run_and_a_shaded_run_around_one_box_become_one_bevel_and_a_stray_line_does_not() {
        let raised = alloc::vec![line(2, 9, 2, 2, PenRole::Shine), line(2, 2, 11, 2, PenRole::Shine), line(12, 2, 12, 9, PenRole::Shadow), line(12, 9, 2, 9, PenRole::Shadow)];
        let out = coalesce_bevels(raised);
        assert_eq!(out, alloc::vec![Rec::Op { op: DrawOp::Bevel { rect: GuestRect { left: 2, top: 2, width: 11, height: 8 }, raised: true }, bbox: Rect::new(2, 2, 13, 10) }]);
        let recessed = alloc::vec![line(2, 2, 11, 2, PenRole::Shadow), line(2, 2, 2, 9, PenRole::Shadow), line(12, 2, 12, 9, PenRole::Shine), line(2, 9, 12, 9, PenRole::Shine)];
        assert!(matches!(coalesce_bevels(recessed)[..], [Rec::Op { op: DrawOp::Bevel { raised: false, .. }, .. }]));
        // negative control: two unrelated lines (the shine one is neither the left column nor the top row of the union) stay lines
        let stray = alloc::vec![line(5, 5, 5, 9, PenRole::Shine), line(2, 8, 4, 8, PenRole::Shadow)];
        assert_eq!(coalesce_bevels(stray.clone()), stray);
        assert_eq!(coalesce_bevels(alloc::vec![line(1, 1, 5, 1, PenRole::Text)]).len(), 1);
        // negative control: a lone lit row and a lone shaded column lie on the right edges of their union but draw no box
        let lone = alloc::vec![line(2, 2, 11, 2, PenRole::Shine), line(12, 2, 12, 9, PenRole::Shadow)];
        assert_eq!(coalesce_bevels(lone.clone()), lone, "a pair that does not draw the whole outline is not a bevel");
    }

    #[test]
    fn a_text_row_redrawn_in_place_does_not_grow_the_list_and_an_overflow_keeps_the_newest_named() {
        let text = |s: &str, bg: Option<PenRole>| (DrawOp::Text { at: GuestPoint { x: 0, y: 0 }, string: s.into(), font_role: heirloom_ui_abi::FontRole::Ui, px: 8, width: 40, pen_role: PenRole::Text, bg_role: bg }, Rect::new(0, 0, 40, 8));
        let canvas = Rect::new(0, 0, 100, 100);
        let (mut jam2, mut jam1) = (Presenter::default(), Presenter::default());
        for i in 0..50 {
            let (op, bb) = text(&alloc::format!("{i}"), Some(PenRole::Background));
            jam2.record(1, op, bb);
            let (op, bb) = text(&alloc::format!("{i}"), None);
            jam1.record(1, op, bb);
        }
        jam2.prune(1, canvas); jam1.prune(1, canvas);
        assert_eq!(jam2.recs(1).len(), 1, "a JAM2 run paints its cell, so the run under it is dead");
        assert_eq!(jam1.recs(1).len(), 50, "NEGATIVE CONTROL: a JAM1 run leaves what is under it");
        // Overflow: the oldest half becomes ONE hole over its union, the newest recs stay named, and an opaque fill over the hole clears it.
        let mut p = Presenter::default();
        for i in 0..(MAX_RECS as i32 + 10) { p.record(1, DrawOp::Line { a: GuestPoint { x: 0, y: i % 90 }, b: GuestPoint { x: 9, y: i % 90 }, pen_role: PenRole::Text }, Rect::new(0, i % 90, 10, i % 90 + 1)); }
        p.prune(1, canvas);
        assert!(p.recs(1).len() <= MAX_RECS / 2 + 1);
        assert!(matches!(p.recs(1)[0], Rec::Hole(_)), "the oldest recs became a hole");
        assert!(matches!(p.recs(1)[1], Rec::Op { .. }) && matches!(p.recs(1).last(), Some(Rec::Op { .. })), "the newest stay named");
        p.record(1, DrawOp::Fill { rect: grect(&canvas), pen_role: PenRole::Background }, canvas);
        p.prune(1, canvas);
        assert_eq!(p.recs(1).len(), 1, "the window recovers: one opaque rec over the canvas removes the hole");
    }

    #[test]
    fn a_flush_publishes_one_plane_with_named_ops_holes_as_pixels_and_nothing_under_a_native_control() {
        let (mut ui, mut p) = (RecordingGuestUi::new(), Presenter::default());
        p.set_plane_route(true);
        let (n, mut bm, pal) = (window(&mut ui), Bitmap::new(8, 6, 1).unwrap(), [0xFFFFFF, 0x112233]);
        bm.set(6, 5, 1);
        let fill = |r: Rect, role| (DrawOp::Fill { rect: grect(&r), pen_role: role }, r);
        let (op, bb) = fill(Rect::new(0, 0, 4, 2), PenRole::Background);
        p.record(7, op, bb);
        let (op, bb) = fill(Rect::new(5, 0, 8, 2), PenRole::Fill); // under the native control
        p.record(7, op, bb);
        p.record_hole(7, Rect::new(4, 4, 8, 6));
        p.add_damage(7, Rect::new(0, 0, 8, 6));
        let (mut log, mut refused) = (Vec::new(), Vec::new());
        let layout = Layout { offset: (3, 2), exclude: alloc::vec![Rect::new(5, 0, 8, 2)] };
        p.flush_planes(&mut ui, "[t]", &mut log, &mut refused, |_| Some((Source { native: n, canvas: &bm, palette: &pal }, layout.clone())));
        let planes: Vec<&Plane> = ui.calls.iter().filter_map(|c| if let UiCall::PresentPlane(_, pl) = c { Some(pl) } else { None }).collect();
        assert_eq!(planes.len(), 1, "one present_plane per flush");
        assert_eq!(planes[0].rect, GuestRect { left: 3, top: 2, width: 8, height: 6 }, "window coordinates");
        assert_eq!(planes[0].ops.len(), 2, "the fill under the control is not published: {:?}", planes[0].ops);
        assert!(matches!(&planes[0].ops[0], DrawOp::Fill { rect, pen_role: PenRole::Background } if *rect == GuestRect { left: 3, top: 2, width: 4, height: 2 }));
        let DrawOp::Pixels { rect, rgb } = &planes[0].ops[1] else { panic!("{:?}", planes[0].ops) };
        assert_eq!((rect.left, rect.top, rgb.len()), (7, 6, 8));
        assert_eq!(rgb[6], 0x112233, "the hole's pixels are read from the canvas at flush time");
        assert!(ui.calls.iter().all(|c| !matches!(c, UiCall::PresentContent { .. })));
        // negative control: no damage, no plane
        p.flush_planes(&mut ui, "[t]", &mut log, &mut refused, |_| Some((Source { native: n, canvas: &bm, palette: &pal }, layout.clone())));
        assert_eq!(ui.calls.iter().filter(|c| matches!(c, UiCall::PresentPlane(..))).count(), 1);
        assert!(refused.is_empty(), "{refused:?}");
    }

    #[test]
    fn a_later_opaque_fill_prunes_what_it_covers() {
        let mut p = Presenter::default();
        p.record_hole(1, Rect::new(2, 2, 4, 4));
        p.record(1, DrawOp::Line { a: GuestPoint { x: 0, y: 0 }, b: GuestPoint { x: 9, y: 0 }, pen_role: PenRole::Text }, Rect::new(0, 0, 10, 1));
        p.record(1, DrawOp::Fill { rect: grect(&Rect::new(0, 0, 10, 10)), pen_role: PenRole::Background }, Rect::new(0, 0, 10, 10));
        p.prune(1, Rect::new(0, 0, 10, 10));
        assert_eq!(p.recs(1).len(), 1, "the fill covers both earlier recs");
        // negative control: a fill that covers only part of a rec removes nothing
        let mut q = Presenter::default();
        q.record_hole(1, Rect::new(0, 0, 10, 10));
        q.record(1, DrawOp::Fill { rect: grect(&Rect::new(0, 0, 5, 5)), pen_role: PenRole::Fill }, Rect::new(0, 0, 5, 5));
        q.prune(1, Rect::new(0, 0, 10, 10));
        assert_eq!(q.recs(1).len(), 2);
    }

    fn make_text(s: &str, x: i32, y: i32, pen_role: PenRole, bg_role: Option<PenRole>) -> Rec {
        Rec::Op {
            op: DrawOp::Text {
                at: GuestPoint { x, y },
                string: s.into(),
                font_role: heirloom_ui_abi::FontRole::Ui,
                px: 8,
                width: 24,
                pen_role,
                bg_role,
            },
            bbox: Rect::new(x, y, x + 24, y + 8),
        }
    }

    fn make_fill_covering_text_box() -> Rec {
        let bbox = Rect::new(0, 0, 50, 50);
        Rec::Op {
            op: DrawOp::Fill { rect: grect(&bbox), pen_role: PenRole::Background },
            bbox,
        }
    }

    #[test]
    fn shadow_merge_basic() {
        // Positive: shadow at (11,11) then same string at (10,10) → 1 rec at (10,10) with second pen_role
        let mut v = Vec::new();
        push_rec(&mut v, make_text("RAM", 11, 11, PenRole::Shadow, None));
        push_rec(&mut v, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v.len(), 1);
        if let Rec::Op { op: DrawOp::Text { at, pen_role, .. }, .. } = &v[0] {
            assert_eq!((at.x, at.y), (10, 10));
            assert_eq!(*pen_role, PenRole::Text);
        } else { panic!("expected Text rec"); }

        // Negative control: 3px apart → 2 recs
        let mut v2 = Vec::new();
        push_rec(&mut v2, make_text("RAM", 13, 13, PenRole::Shadow, None));
        push_rec(&mut v2, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v2.len(), 2);
    }

    #[test]
    fn shadow_merge_different_strings() {
        // Positive: different strings at 1px → 2 recs
        let mut v = Vec::new();
        push_rec(&mut v, make_text("RAM", 11, 11, PenRole::Shadow, None));
        push_rec(&mut v, make_text("SYS", 10, 10, PenRole::Text, None));
        assert_eq!(v.len(), 2);

        // Negative control: same string at 1px → 1 rec
        let mut v2 = Vec::new();
        push_rec(&mut v2, make_text("RAM", 11, 11, PenRole::Shadow, None));
        push_rec(&mut v2, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v2.len(), 1);
    }

    #[test]
    fn shadow_merge_opaque_not_merged() {
        // Positive: opaque first run (bg_role: Some) → 2 recs
        let mut v = Vec::new();
        push_rec(&mut v, make_text("RAM", 11, 11, PenRole::Shadow, Some(PenRole::Background)));
        push_rec(&mut v, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v.len(), 2);

        // Negative control: both transparent → 1 rec
        let mut v2 = Vec::new();
        push_rec(&mut v2, make_text("RAM", 11, 11, PenRole::Shadow, None));
        push_rec(&mut v2, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v2.len(), 1);
    }

    #[test]
    fn shadow_merge_fill_blocks() {
        // Positive: Fill between passes → 3 recs
        let mut v = Vec::new();
        push_rec(&mut v, make_text("RAM", 11, 11, PenRole::Shadow, None));
        push_rec(&mut v, make_fill_covering_text_box());
        push_rec(&mut v, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v.len(), 3);

        // Negative control: no fill → 1 rec
        let mut v2 = Vec::new();
        push_rec(&mut v2, make_text("RAM", 11, 11, PenRole::Shadow, None));
        push_rec(&mut v2, make_text("RAM", 10, 10, PenRole::Text, None));
        assert_eq!(v2.len(), 1);
    }
}
