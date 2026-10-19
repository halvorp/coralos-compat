//! Where a RastPort draws: its Layer's canvas (a window), or the bitmap it points at.
use super::layout::*;
use super::record::Prim;
use super::state::GBitmap;
use crate::{machine::Amiga, Refusal};
use alloc::format;
use guestui_abi::GuestUi;
use retro_raster::{paint::Clip, Bitmap, Rect, Region};

/// A resolved drawing destination.
pub enum Dest {
    /// Draw into bitmap `bm` (a guest BitMap address) at (x - origin.0, y - origin.1), clipped to `region` (already in
    /// bitmap coordinates). `window` receives the damage.
    Bitmap { bm: u32, origin: (i32, i32), window: Option<u32>, region: Option<Region> },
    /// Drawing is dropped by design; the reason is logged.
    Dropped(&'static str),
}

/// Resolve `rp`. A RastPort with a Layer must be one of the engine's layers (a guest-built Layer is a named refusal).
pub fn resolve<U: GuestUi>(a: &mut Amiga<U>, rp: u32) -> Result<Dest, Refusal> {
    if let Some(why) = a.gfx.dropped_rps.get(&rp) { return Ok(Dest::Dropped(why)); }
    let m = Refusal::mem("RastPort");
    let layer = a.mem.read_u32(rp + RP_LAYER).map_err(&m)?;
    if layer != 0 {
        let Some(l) = a.gfx.layers.get(&layer) else { return Err(Refusal(format!("RastPort {rp:#x}: Layer {layer:#x} was not made by this engine"))) };
        let (bm, origin, window) = (l.canvas, l.origin, l.window);
        let installed = a.mem.read_u32(layer + LY_CLIP_REGION).map_err(&m)?;
        let region = a.gfx.regions.get(&installed).filter(|_| installed != 0).map(|g| { let mut r = g.region.clone(); r.translate(-origin.0, -origin.1); r });
        return Ok(Dest::Bitmap { bm, origin, window: Some(window), region });
    }
    let bm = a.mem.read_u32(rp + RP_BITMAP).map_err(&m)?;
    if bm == 0 { return Err(Refusal(format!("RastPort {rp:#x} has neither a Layer nor a BitMap"))); }
    super::bitmaps::ensure(a, bm)?;
    let window = a.gfx.bitmaps.get(&bm).and_then(|g| g.window);
    Ok(Dest::Bitmap { bm, origin: (0, 0), window, region: None })
}

/// Run `f` on the destination bitmap with its clip and origin; the rectangle it returns (bitmap coordinates) becomes
/// damage. A dropped destination logs and does nothing.
pub fn paint<U: GuestUi>(a: &mut Amiga<U>, rp: u32, what: &str, f: impl FnOnce(&mut Bitmap, &Clip, (i32, i32)) -> Rect) -> Result<Rect, Refusal> {
    paint_as(a, rp, what, None, f)
}

/// [`paint`] for a call the plane route can NAME (`prim`: what was drawn, in RastPort coordinates and guest pens); `None`, a pen
/// without a role, or an installed clip region record the damage as a hole of canvas pixels instead (record.rs).
pub fn paint_as<U: GuestUi>(a: &mut Amiga<U>, rp: u32, what: &str, prim: Option<Prim>, f: impl FnOnce(&mut Bitmap, &Clip, (i32, i32)) -> Rect) -> Result<Rect, Refusal> {
    match resolve(a, rp)? {
        Dest::Dropped(why) => { a.log.push(format!("[graphics] {what} on {rp:#x} dropped: {why}")); Ok(Rect::default()) }
        Dest::Bitmap { bm, origin, window, region } => {
            super::bitmaps::ensure_source(a, bm)?;
            let g: &mut GBitmap = a.gfx.bitmaps.get_mut(&bm).ok_or_else(|| Refusal(format!("{what}: BitMap {bm:#x} vanished")))?;
            let clip = Clip { rect: g.bm.rect(), region: region.as_ref() };
            let r = f(&mut g.bm, &clip, origin);
            if let Some(w) = window {
                a.gfx.add_damage(w, r);
                super::record::note(a, w, if region.is_some() { None } else { prim }, r, origin);
            }
            super::bitmaps::export_planes(a, bm)?;
            Ok(r)
        }
    }
}

/// A call that is SYSTEM UI and so puts no pixels in the canvas (`prim`, in RastPort coordinates: a bevel box): only the display-list
/// op is recorded, for the host's kit to draw. Refused by name where that cannot be done exactly: a destination that is not a window
/// canvas, an installed clip region, or a host without the plane route (it would show nothing, and the guest's drawing is not faked).
pub fn note_native<U: GuestUi>(a: &mut Amiga<U>, rp: u32, what: &str, prim: Prim, rect: Rect) -> Result<(), Refusal> {
    match resolve(a, rp)? {
        Dest::Dropped(why) => { a.log.push(format!("[graphics] {what} on {rp:#x} dropped: {why}")); Ok(()) }
        Dest::Bitmap { window: None, .. } => Err(Refusal(format!("{what}: RastPort {rp:#x} draws into an off-screen bitmap: system UI has no display list there"))),
        Dest::Bitmap { region: Some(_), .. } => Err(Refusal(format!("{what}: a clip region is installed on the layer: a native op cannot be clipped to it"))),
        Dest::Bitmap { bm, origin, window: Some(w), .. } => {
            if !a.gfx.present.plane_route() { return Err(Refusal(format!("{what}: the host has no plane route, so a native op would show nothing"))); }
            super::bitmaps::ensure_source(a, bm)?;
            let hit = rect.translate(-origin.0, -origin.1).intersect(&a.gfx.bitmaps[&bm].bm.rect());
            a.gfx.add_damage(w, hit);
            super::record::note(a, w, Some(prim), hit, origin);
            Ok(())
        }
    }
}

/// Read access to the destination bitmap (ReadPixel, ClipBlit sources): the pixel at RastPort coordinates, honouring
/// the layer clip. `None` = outside.
pub fn read_pixel<U: GuestUi>(a: &mut Amiga<U>, rp: u32, x: i32, y: i32) -> Result<Option<u8>, Refusal> {
    Ok(match resolve(a, rp)? {
        Dest::Dropped(_) => None,
        Dest::Bitmap { bm, origin, region, .. } => {
            let (bx, by) = (x - origin.0, y - origin.1);
            super::bitmaps::ensure_source(a, bm)?;
            let g = a.gfx.bitmaps.get(&bm).ok_or_else(|| Refusal(format!("ReadPixel: BitMap {bm:#x} vanished")))?;
            let clip = Clip { rect: g.bm.rect(), region: region.as_ref() };
            if clip.allows(bx, by) { g.bm.get(bx, by) } else { None }
        }
    })
}
