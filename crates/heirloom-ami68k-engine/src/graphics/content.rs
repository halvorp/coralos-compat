//! Publishing guest-drawn content: once per engine step, each window with damage gets ONE `present_plane` carrying the display
//! list recorded for the damage rectangle (record.rs; the `heirloom-present` plane route), with the guest's unnamed drawing as
//! `Pixels` holes read from the canvas. A host without a plane part gets the same damage as ONE `present_content` of resolved
//! `0x00RRGGBB` pixels instead (the first frame of a window is its whole canvas). The accumulation and both flushes are
//! `heirloom-present`'s, shared with the Mac engine; this file only resolves a window to its canvas and its native controls.
use super::layout::*;
use crate::machine::Amiga;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_present::{plane::Layout, Source};
use heirloom_raster::Rect;

/// Where `win`'s canvas sits in the window (the border offset) and which canvas rectangles native controls cover: a control
/// is drawn by the host, so nothing the guest drew under it is published.
fn layout_of<U: GuestUi>(a: &Amiga<U>, win: u32) -> Layout {
    let border = |i: u32| a.mem.read_u8(win + 54 + i).map_or(0, i32::from);
    let (bl, bt) = (border(0), border(1));
    let rects = crate::native_gadgets::of_window(a, win).into_iter().map(|(_, m)| m.rect)
        .chain(a.gt_ctls.iter().filter(|(_, p)| p.meta == win).map(|(_, p)| p.model.rect));
    let exclude: Vec<Rect> = rects.map(|r| Rect::from_size(r.left - bl, r.top - bt, r.width, r.height)).collect();
    Layout { offset: (bl, bt), exclude }
}

/// Flush every window's accumulated damage to the host. Called by `Amiga::step` after the guest ran.
pub fn flush<U: GuestUi>(a: &mut Amiga<U>) {
    let palette = a.gfx.palette().rgb;
    let layouts: Vec<(u32, Layout)> = a.windows.keys().map(|&w| (w, layout_of(a, w))).collect();
    let mut presenter = core::mem::take(&mut a.gfx.present);
    let (windows, mem, layers, bitmaps) = (&a.windows, &a.mem, &a.gfx.layers, &a.gfx.bitmaps);
    let resolve = |win: u32| {
        let native = *windows.get(&win)?;
        let layer = mem.read_u32(win + WD_WLAYER).ok()?;
        let canvas = layers.get(&layer)?.canvas;
        Some(Source { native, canvas: &bitmaps.get(&canvas)?.bm, palette: &palette })
    };
    if presenter.plane_route() {
        presenter.flush_planes(&mut a.ui, "[graphics]", &mut a.log, &mut a.refused, |win| {
            let layout = layouts.iter().find(|(w, _)| *w == win)?.1.clone();
            Some((resolve(win)?, layout))
        });
    } else {
        presenter.flush(&mut a.ui, "[graphics]", &mut a.log, &mut a.refused, resolve);
    }
    a.gfx.present = presenter;
}
