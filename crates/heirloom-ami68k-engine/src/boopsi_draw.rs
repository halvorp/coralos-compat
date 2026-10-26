//! sysiclass objects (arrows, window-frame gadgets) and DrawImageState on native images.
//!
//! The engine draws NO control look any more (Gute Form spike, Peter 2026-10-18): buttongclass / propgclass / strgclass gadgets and
//! the sysiclass arrows inside them are NATIVE controls the host draws with its own kit (native_gadgets.rs), and the sysiclass
//! images of a window frame (close / depth / size / the border arrows) belong to the native window frame. What is left here is the
//! bookkeeping a guest can observe: SYSIA_Which recorded at OM_NEW, and the default size of an arrow image (AROS asks the window
//! decoration object; 14x14, rom/intuition sysiclass.c). A guest that calls DrawImageState on a sysiclass image ITSELF (its own
//! gadget class painting into the canvas) gets no pixels: that look is the host's, and the call is logged by name. frameiclass is
//! the one image class still drawn into the canvas, because a guest-defined class that frames its own contents asks for exactly
//! that (boopsi_frame.rs).
use alloc::format;
use crate::boopsi_ids::*;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;

pub const LEFTIMAGE: u32 = 0x0A;
pub const UPIMAGE: u32 = 0x0B;
pub const RIGHTIMAGE: u32 = 0x0C;
pub const DOWNIMAGE: u32 = 0x0D;
const SYSIA_WHICH: u32 = IA + 0x0D;
const DEFAULT_ARROW: i32 = 14;

/// The SYSIA_Which of a sysiclass object (recorded at OM_NEW), if it is one.
pub fn sysi_which<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> { a.classes.sysi_type.get(&obj).copied() }

/// Record SYSIA_Which / default size for a new sysiclass object whose attribute list is `tag_list`.
pub fn sysi_new<U: GuestUi>(a: &mut Amiga<U>, im: u32, tag_list: u32) -> Result<(), Refusal> {
    let t = crate::tags::walk(&a.mem, tag_list)?;
    let which = crate::tags::get(&t, SYSIA_WHICH).unwrap_or(0);
    a.classes.sysi_type.insert(im, which);
    if matches!(which, LEFTIMAGE | UPIMAGE | RIGHTIMAGE | DOWNIMAGE) {
        let m = Refusal::mem("Image");
        if crate::tags::get(&t, IA_WIDTH).is_none() { a.mem.write_u16(im + IMG_WIDTH, DEFAULT_ARROW as u16).map_err(&m)?; }
        if crate::tags::get(&t, IA_HEIGHT).is_none() { a.mem.write_u16(im + IMG_HEIGHT, DEFAULT_ARROW as u16).map_err(&m)?; }
    }
    Ok(())
}

/// IM_DRAW on a native image object `im` at (x, y) (DrawImageState / impDraw): frameiclass draws into the canvas; a sysiclass image
/// draws nothing (it is a native control or part of the native window frame).
pub fn draw_native_image<U: GuestUi>(a: &mut Amiga<U>, im: u32, rp: u32, x: i32, y: i32, state: u32) -> Result<(), Refusal> {
    if a.classes.class_of(im).and_then(|c| a.classes.recs.get(&c)).and_then(|r| r.native) == Some(crate::iclass::Native::Frame) {
        let (w, h) = (a.mem.read_u16(im + IMG_WIDTH).map_err(Refusal::mem("Image"))? as i16 as i32, a.mem.read_u16(im + IMG_HEIGHT).map_err(Refusal::mem("Image"))? as i16 as i32);
        crate::boopsi_frame::draw(a, im, rp, x, y, state, (w, h))?;
        return Ok(());
    }
    let Some(which) = sysi_which(a, im) else { return Err(Refusal(format!("DrawImageState: image object {im:#x} is of a class without native drawing"))) };
    a.log.push(format!("[intuition] DrawImageState of sysiclass image {im:#x} (type {which:#x}) at {x},{y}: the host draws system images natively; no canvas pixels"));
    Ok(())
}
