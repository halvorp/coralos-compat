//! Publishing guest-drawn content: once per engine step, each window with damage gets ONE `present_content` carrying
//! the accumulated damage rectangle as resolved `0x00RRGGBB` pixels (the first frame of a window is its whole canvas).
//! The accumulation and the flush are `retro-present`'s, shared with the Mac engine; this file only resolves a window
//! to its canvas.
use super::layout::*;
use crate::machine::Amiga;
use guestui_abi::GuestUi;
use retro_present::Source;

/// Flush every window's accumulated damage to the host. Called by `Amiga::step` after the guest ran.
pub fn flush<U: GuestUi>(a: &mut Amiga<U>) {
    let palette = a.gfx.palette().rgb;
    let mut presenter = core::mem::take(&mut a.gfx.present);
    let (windows, mem, layers, bitmaps) = (&a.windows, &a.mem, &a.gfx.layers, &a.gfx.bitmaps);
    presenter.flush(&mut a.ui, "[graphics]", &mut a.log, &mut a.refused, |win| {
        let native = *windows.get(&win)?;
        let layer = mem.read_u32(win + WD_WLAYER).ok()?;
        let canvas = layers.get(&layer)?.canvas;
        Some(Source { native, canvas: &bitmaps.get(&canvas)?.bm, palette: &palette })
    });
    a.gfx.present = presenter;
}
