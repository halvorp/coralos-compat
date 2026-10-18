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
use guestui_abi::GuestUi;

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
