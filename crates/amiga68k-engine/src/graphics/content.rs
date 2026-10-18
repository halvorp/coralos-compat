//! Publishing guest-drawn content: once per engine step, each window with damage gets ONE `present_content` carrying
//! the accumulated damage rectangle as resolved `0x00RRGGBB` pixels (the first frame of a window is its whole canvas).
use super::layout::*;
use crate::machine::Amiga;
use alloc::format;
use alloc::vec::Vec;
use guestui_abi::{GuestRect, GuestUi, UiRefusal};

/// Flush every window's accumulated damage to the host. Called by `Amiga::step` after the guest ran.
pub fn flush<U: GuestUi>(a: &mut Amiga<U>) {
    let pending: Vec<(u32, retro_raster::Rect)> = core::mem::take(&mut a.gfx.damage).into_iter().collect();
    for (win, mut r) in pending {
        let Some(&native) = a.windows.get(&win) else { continue };
        let Ok(layer) = a.mem.read_u32(win + WD_WLAYER) else { continue };
        let Some(canvas) = a.gfx.layers.get(&layer).map(|l| l.canvas) else { continue };
        let Some(g) = a.gfx.bitmaps.get(&canvas) else { continue };
        let (w, h) = (g.bm.width(), g.bm.height());
        if !a.gfx.presented.contains(&win) { r = g.bm.rect(); }
        let r = r.intersect(&g.bm.rect());
        if r.is_empty() { continue; }
        let palette = a.gfx.palette().rgb;
        let g = &a.gfx.bitmaps[&canvas];
        let mut px = Vec::with_capacity((r.width() * r.height()) as usize);
        for y in r.y0..r.y1 { px.extend(g.bm.row(y)[r.x0 as usize..r.x1 as usize].iter().map(|&p| palette[p as usize])); }
        let damage = GuestRect { left: r.x0, top: r.y0, width: r.width(), height: r.height() };
        match a.ui.present_content(native, damage, w, h, &px) {
            Ok(()) => { a.gfx.presented.insert(win); a.log.push(format!("[graphics] content window {win:#x}: {}x{} at {},{} of {w}x{h}", r.width(), r.height(), r.x0, r.y0)); }
            Err(UiRefusal::Unsupported(why)) => {
                if !a.gfx.content_refused { a.gfx.content_refused = true; a.log.push(format!("[graphics] content window {win:#x} not shown: {why}")); }
            }
            Err(e) => a.refused.push(format!("graphics/present_content: {e}")),
        }
    }
}
