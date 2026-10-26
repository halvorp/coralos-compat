//! Intuition's side of BOOPSI gadgets: the GadgetInfo handed to every GM_* method.
use alloc::format;
use crate::boopsi_ids::*;
use crate::graphics::layout::{WD_RPORT, WD_WLAYER};
use crate::{machine::Amiga, memory, screen, Refusal};
use heirloom_ui_abi::GuestUi;

/// Build a `struct GadgetInfo` for `win` (gi_Window, gi_RastPort, gi_Layer, gi_Screen, gi_Domain = the window's box).
pub fn make_ginfo<U: GuestUi>(a: &mut Amiga<U>, win: u32, req: u32, _gadget: u32) -> Result<u32, Refusal> {
    let gi = memory::alloc_zeroed(a, GI_SIZE, "GadgetInfo")?;
    let e = Refusal::mem("GadgetInfo");
    a.mem.write_u32(gi + GI_SCREEN, screen::SCREEN).map_err(&e)?;
    a.mem.write_u32(gi + GI_WINDOW, win).map_err(&e)?;
    a.mem.write_u32(gi + 8, req).map_err(&e)?;
    a.mem.write_u32(gi + GI_DRINFO, crate::graphics::layout::SCREEN_DRAWINFO).map_err(&e)?;
    if win != 0 {
        let rp = a.mem.read_u32(win + WD_RPORT).map_err(&e)?;
        let layer = a.mem.read_u32(win + WD_WLAYER).map_err(&e)?;
        a.mem.write_u32(gi + GI_RASTPORT, rp).map_err(&e)?;
        a.mem.write_u32(gi + GI_LAYER, layer).map_err(&e)?;
        let (w, h) = (a.mem.read_u16(win + 8).map_err(&e)?, a.mem.read_u16(win + 10).map_err(&e)?);
        a.mem.write_u16(gi + GI_DOMAIN + 4, w).map_err(&e)?;
        a.mem.write_u16(gi + GI_DOMAIN + 6, h).map_err(&e)?;
    }
    Ok(gi)
}

pub fn free_ginfo<U: GuestUi>(a: &mut Amiga<U>, gi: u32) -> Result<(), Refusal> {
    if gi == 0 { return Ok(()); }
    a.heap.free(gi, GI_SIZE).map_err(|e| Refusal(format!("GadgetInfo: {e}")))
}
