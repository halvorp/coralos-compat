//! Publishing guest-drawn content, once, for every heirloom engine: drawing accumulates a damage rectangle per window;
//! once per engine step each damaged window gets ONE `GuestUi::present_content` carrying that rectangle as resolved
//! `0x00RRGGBB` pixels. The first frame of a window is its whole canvas. Both `heirloom-ami68k-engine` and `heirloom-mac68k-engine`
//! call this; neither has its own copy.
//!
//! The plane route ([`plane`], ruling 17 piece 5): an engine that records a display list per window publishes
//! `present_plane` instead; a host that answers `Unsupported` falls back to the canvas route, so the fallback is the design.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod plane;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestRect, GuestUi, UiObjectRef, UiRefusal};
use heirloom_raster::{Bitmap, Rect};

/// Where a window's pixels come from at flush time: its native window, the canvas, and the pen -> RGB table.
pub struct Source<'a> { pub native: UiObjectRef, pub canvas: &'a Bitmap, pub palette: &'a [u32] }

/// Per-window damage and first-frame bookkeeping. The key is whatever the engine calls a window (a guest address).
#[derive(Default)]
pub struct Presenter {
    damage: BTreeMap<u32, Rect>,
    presented: BTreeSet<u32>,
    /// A host without a canvas part refused a frame; logged once.
    refused_once: bool,
    /// The display list of each window (plane route; empty and unused by an engine that does not record).
    pub(crate) recs: BTreeMap<u32, Vec<plane::Rec>>,
    /// Plane route on: damaged windows go out as `present_plane` (canvas pixels only where a `Rec::Hole` says so).
    pub(crate) plane_route: bool,
    pub(crate) plane_refused_once: bool,
}

impl Presenter {
    /// Union `r` into the window's pending damage (an empty rectangle is no damage).
    pub fn add_damage(&mut self, win: u32, r: Rect) {
        if r.is_empty() { return; }
        let e = self.damage.entry(win).or_default();
        *e = e.union(&r);
    }
    /// The window is gone: forget its damage and its first-frame state.
    pub fn forget(&mut self, win: u32) { self.damage.remove(&win); self.presented.remove(&win); self.recs.remove(&win); }
    pub fn pending(&self, win: u32) -> Option<Rect> { self.damage.get(&win).copied() }
    pub fn has_presented(&self, win: u32) -> bool { self.presented.contains(&win) }

    /// Flush every damaged window. `tag` prefixes log lines ("[graphics]"); `source` resolves a window key (None: the
    /// window no longer has a canvas, its damage is dropped).
    pub fn flush<'a, U: GuestUi>(&mut self, ui: &mut U, tag: &str, log: &mut Vec<String>, refused: &mut Vec<String>, mut source: impl FnMut(u32) -> Option<Source<'a>>) {
        for (win, mut r) in core::mem::take(&mut self.damage) {
            let Some(s) = source(win) else { continue };
            let (w, h) = (s.canvas.width(), s.canvas.height());
            if !self.presented.contains(&win) { r = s.canvas.rect(); }
            let r = r.intersect(&s.canvas.rect());
            if r.is_empty() { continue; }
            let mut px = Vec::with_capacity((r.width() * r.height()) as usize);
            for y in r.y0..r.y1 { px.extend(s.canvas.row(y)[r.x0 as usize..r.x1 as usize].iter().map(|&p| s.palette[p as usize])); }
            let damage = GuestRect { left: r.x0, top: r.y0, width: r.width(), height: r.height() };
            match ui.present_content(s.native, damage, w, h, &px) {
                Ok(()) => { self.presented.insert(win); log.push(format!("{tag} content window {win:#x}: {}x{} at {},{} of {w}x{h}", r.width(), r.height(), r.x0, r.y0)); }
                Err(UiRefusal::Unsupported(why)) => {
                    if !self.refused_once { self.refused_once = true; log.push(format!("{tag} content window {win:#x} not shown: {why}")); }
                }
                Err(e) => refused.push(format!("{tag} present_content: {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::{RecordingGuestUi, UiCall, WindowModel, WindowRole, CallbackToken};

    fn window(ui: &mut RecordingGuestUi) -> UiObjectRef {
        ui.open_window(WindowModel { title: "t".into(), rect: GuestRect { left: 0, top: 0, width: 4, height: 3 }, role: WindowRole::Document, closable: true, resizable: false, token: CallbackToken(1) }).unwrap()
    }
    fn presents(ui: &RecordingGuestUi) -> Vec<GuestRect> { ui.calls.iter().filter_map(|c| if let UiCall::PresentContent { damage, .. } = c { Some(*damage) } else { None }).collect() }

    #[test]
    fn first_frame_is_the_whole_canvas_then_only_accumulated_damage_once_per_flush() {
        let (mut ui, mut p) = (RecordingGuestUi::new(), Presenter::default());
        let (n, mut bm, pal) = (window(&mut ui), Bitmap::new(4, 3, 1).unwrap(), [0xFFFFFF, 0]);
        let (mut log, mut refused) = (Vec::new(), Vec::new());
        p.add_damage(7, Rect::new(0, 0, 1, 1));
        p.flush(&mut ui, "[t]", &mut log, &mut refused, |_| Some(Source { native: n, canvas: &bm, palette: &pal }));
        assert_eq!(presents(&ui), vec![GuestRect { left: 0, top: 0, width: 4, height: 3 }], "first frame covers the canvas");
        bm.set(3, 2, 1);
        p.add_damage(7, Rect::new(3, 2, 4, 3)); p.add_damage(7, Rect::new(2, 1, 3, 2));
        p.flush(&mut ui, "[t]", &mut log, &mut refused, |_| Some(Source { native: n, canvas: &bm, palette: &pal }));
        assert_eq!(presents(&ui)[1], GuestRect { left: 2, top: 1, width: 2, height: 2 }, "two primitives, ONE union rectangle");
        assert_eq!(ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. })).count(), 2);
        // negative control: nothing damaged, nothing presented
        p.flush(&mut ui, "[t]", &mut log, &mut refused, |_| Some(Source { native: n, canvas: &bm, palette: &pal }));
        assert_eq!(presents(&ui).len(), 2);
        assert!(refused.is_empty());
    }

    #[test]
    fn a_window_without_a_canvas_drops_its_damage_and_forget_restarts_the_first_frame() {
        let (mut ui, mut p) = (RecordingGuestUi::new(), Presenter::default());
        let (n, bm, pal) = (window(&mut ui), Bitmap::new(4, 3, 1).unwrap(), [0xFFFFFF, 0]);
        let (mut log, mut refused) = (Vec::new(), Vec::new());
        p.add_damage(1, Rect::new(0, 0, 2, 2));
        p.flush(&mut ui, "[t]", &mut log, &mut refused, |_| None);
        assert!(presents(&ui).is_empty() && p.pending(1).is_none());
        p.add_damage(1, Rect::new(0, 0, 1, 1));
        p.flush(&mut ui, "[t]", &mut log, &mut refused, |_| Some(Source { native: n, canvas: &bm, palette: &pal }));
        assert!(p.has_presented(1));
        p.forget(1);
        assert!(!p.has_presented(1));
        p.add_damage(1, Rect::new(0, 0, 1, 1));
        p.flush(&mut ui, "[t]", &mut log, &mut refused, |_| Some(Source { native: n, canvas: &bm, palette: &pal }));
        assert_eq!(presents(&ui)[1].width, 4, "after forget the next frame is a full first frame again");
    }
}
