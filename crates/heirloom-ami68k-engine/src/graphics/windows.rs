//! A window's drawing surface: a content canvas (a host bitmap behind a real guest BitMap), a real `struct Layer`, the
//! window's RPort and BorderRPort, wired into `struct Window` (RPort @50, BorderRPort @58, WLayer @124, IFont @128).
//!
//! The canvas is the window's INNER area (what is left after the frame): the native window supplies the title bar,
//! borders and depth/close gadgets, so a guest's drawing on the frame (BorderRPort, or RPort coordinates inside the
//! border) is dropped by design and says so in the log. A normal window's RPort origin is the window's top-left, so
//! inner-area pixel (x, y) is RPort (x + BorderLeft, y + BorderTop); a GIMMEZEROZERO window's RPort origin is the inner
//! area's top-left. Every window canvas starts as pen 0 (the window background) and is presented in full on its first
//! frame.
use super::layout::*;
use super::{bitmaps, fonts, rastport, state::GLayer};
use crate::{machine::Amiga, memory, Refusal};
use heirloom_ui_abi::GuestUi;

/// Build the surface for `win` (a freshly allocated struct Window with geometry and borders already written).
pub fn open<U: GuestUi>(a: &mut Amiga<U>, win: u32, (left, top): (i32, i32), (iw, ih): (i32, i32), (bl, bt): (i32, i32), gzz: bool) -> Result<(), Refusal> {
    let m = Refusal::mem("Window surface");
    let canvas = bitmaps::create(a, iw.max(1), ih.max(1), SCREEN_DEPTH, Some(win))?;
    let origin = if gzz { (0, 0) } else { (bl, bt) };
    let layer = memory::alloc_zeroed(a, LY_SIZE, "Layer")?;
    let (rp, brp) = (memory::alloc_zeroed(a, RP_SIZE, "RPort")?, memory::alloc_zeroed(a, RP_SIZE, "BorderRPort")?);
    rastport::init(a, rp)?;
    rastport::init(a, brp)?;
    // The window RastPort's pens: InitRastPort leaves FgPen all-ones (pen 255); a window starts on the screen's text pen 1.
    a.mem.write_u8(rp + RP_FG_PEN, 1).map_err(&m)?;
    a.mem.write_u32(rp + RP_LAYER, layer).map_err(&m)?;
    a.mem.write_u32(rp + RP_BITMAP, canvas).map_err(&m)?;
    a.mem.write_u32(layer + LY_RP, rp).map_err(&m)?;
    a.mem.write_u32(layer + LY_WINDOW, win).map_err(&m)?;
    for (i, v) in [left + bl, top + bt, left + bl + iw - 1, top + bt + ih - 1].into_iter().enumerate() { a.mem.write_u16(layer + LY_BOUNDS + 2 * i as u32, v as u16).map_err(&m)?; }
    a.mem.write_u32(win + WD_RPORT, rp).map_err(&m)?;
    a.mem.write_u32(win + WD_BORDER_RPORT, brp).map_err(&m)?;
    a.mem.write_u32(win + WD_WLAYER, layer).map_err(&m)?;
    a.mem.write_u32(win + WD_IFONT, fonts::default_font(a)?).map_err(&m)?;
    if gzz {
        a.mem.write_u16(win + WD_GZZ_WIDTH, iw as u16).map_err(&m)?;
        a.mem.write_u16(win + WD_GZZ_HEIGHT, ih as u16).map_err(&m)?;
    }
    a.gfx.layers.insert(layer, GLayer { window: win, canvas, origin });
    a.gfx.dropped_rps.insert(brp, "the window frame is native (BorderRPort)");
    Ok(())
}

/// Tear the surface down with the window.
pub fn close<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("Window surface");
    let (layer, rp, brp) = (a.mem.read_u32(win + WD_WLAYER).map_err(&m)?, a.mem.read_u32(win + WD_RPORT).map_err(&m)?, a.mem.read_u32(win + WD_BORDER_RPORT).map_err(&m)?);
    a.gfx.present.forget(win);
    a.input.buttons.remove(&win);
    if let Some(l) = a.gfx.layers.remove(&layer) {
        a.gfx.bitmaps.remove(&l.canvas);
        a.heap.free(l.canvas, BM_SIZE).map_err(|e| Refusal(alloc::format!("CloseWindow canvas: {e}")))?;
        a.heap.free(layer, LY_SIZE).map_err(|e| Refusal(alloc::format!("CloseWindow layer: {e}")))?;
        a.heap.free(rp, RP_SIZE).map_err(|e| Refusal(alloc::format!("CloseWindow RPort: {e}")))?;
        a.heap.free(brp, RP_SIZE).map_err(|e| Refusal(alloc::format!("CloseWindow BorderRPort: {e}")))?;
    }
    a.gfx.dropped_rps.remove(&brp);
    Ok(())
}

/// Resize `win`'s surface to the new OUTER size (the Window.Width/Height fields are updated by the caller): the canvas becomes
/// the new inner area, keeping the pixels that still fit, the guest BitMap struct and the layer bounds follow, and the whole
/// canvas is damaged so the next present shows the new size. Returns the new inner size.
pub fn resize<U: GuestUi>(a: &mut Amiga<U>, win: u32, (ow, oh): (i32, i32)) -> Result<(i32, i32), Refusal> {
    let m = Refusal::mem("Window resize");
    let layer = a.mem.read_u32(win + WD_WLAYER).map_err(&m)?;
    let (canvas, origin) = a.gfx.layers.get(&layer).map(|l| (l.canvas, l.origin)).ok_or_else(|| Refusal(alloc::format!("window {win:#x} has no surface")))?;
    let b = |o: u32| a.mem.read_u8(win + o).map(|v| v as i32);
    let (bl, bt, br, bb) = (b(54).map_err(&m)?, b(55).map_err(&m)?, b(56).map_err(&m)?, b(57).map_err(&m)?);
    let (iw, ih) = ((ow - bl - br).max(1), (oh - bt - bb).max(1));
    let depth = a.gfx.bitmaps.get(&canvas).map(|g| g.bm.depth()).ok_or_else(|| Refusal("window canvas vanished".into()))?;
    let mut fresh = heirloom_raster::Bitmap::new(iw, ih, depth).ok_or_else(|| Refusal(alloc::format!("window canvas {iw}x{ih} is unsupported")))?;
    let old = &a.gfx.bitmaps[&canvas].bm;
    let keep = old.rect().intersect(&fresh.rect());
    for y in keep.y0..keep.y1 { for x in keep.x0..keep.x1 { fresh.set(x, y, old.get(x, y).unwrap_or(0)); } }
    a.gfx.bitmaps.get_mut(&canvas).expect("canvas").bm = fresh;
    bitmaps::write_struct(a, canvas, iw, ih, depth)?;
    let left = a.mem.read_u16(win + 4).map_err(&m)? as i16 as i32;
    let top = a.mem.read_u16(win + 6).map_err(&m)? as i16 as i32;
    let (ox, oy) = if origin == (0, 0) { (0, 0) } else { (bl, bt) };
    for (i, v) in [left + ox, top + oy, left + ox + iw - 1, top + oy + ih - 1].into_iter().enumerate() { a.mem.write_u16(layer + LY_BOUNDS + 2 * i as u32, v as u16).map_err(&m)?; }
    a.gfx.add_damage(win, heirloom_raster::Rect::new(0, 0, iw, ih));
    Ok((iw, ih))
}
