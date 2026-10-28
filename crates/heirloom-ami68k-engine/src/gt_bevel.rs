//! gadtools.library DrawBevelBoxA: SYSTEM UI, so it never becomes canvas pixels. The call is a display-list `Bevel` op the host's kit
//! draws (raised, or recessed with GTBB_Recessed), recorded through the graphics layer's plane route.
//! Facts: AROS workbench/libs/gadtools/drawbevelboxa.c:69-90 (APL, re-expressed): no GT_VisualInfo = no drawing at all; the box is
//! a frameiclass image with IA_EdgesOnly (gadtools_init.c:66-80, so nothing inside is filled) of IA_FrameType = GTBB_FrameType (default
//! BBFT_BUTTON) and IA_Recessed = GTBB_Recessed, drawn in IDS_NORMAL; frameiclass.c:290-358: BUTTON is one bevel, shine and shadow
//! swapped when recessed. BBFT_RIDGE and BBFT_ICONDROPBOX are two nested inverted bevels (frameiclass.c:333-358), which the kit has no
//! op for, and any other frame type draws nothing in AROS: all three are refused by name.
use alloc::format;
use alloc::string::String;
use crate::graphics::{layout::RP_MASK, record::Prim, target};
use crate::{machine::Amiga, tags, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::Rect;

pub const LVO_DRAW_BEVEL_BOX_A: i32 = -120;
const GT_TAGBASE: u32 = tags::TAG_USER + 0x0008_0000;
pub const GTBB_RECESSED: u32 = GT_TAGBASE + 51;
pub const GT_VISUAL_INFO: u32 = GT_TAGBASE + 52;
pub const GTBB_FRAME_TYPE: u32 = GT_TAGBASE + 77;
const BBFT_BUTTON: u32 = 1;
const BBFT_RIDGE: u32 = 2;
const BBFT_ICONDROPBOX: u32 = 3;

/// DrawBevelBoxA(a0 = rport, d0 = left, d1 = top, d2 = width, d3 = height, a1 = taglist).
pub fn draw_bevel_box<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, tag_list) = (cpu.areg(0), cpu.areg(1));
    let (left, top, w, h) = (cpu.dreg(0) as i16 as i32, cpu.dreg(1) as i16 as i32, cpu.dreg(2) as i16 as i32, cpu.dreg(3) as i16 as i32);
    let t = tags::walk(&a.mem, tag_list)?;
    if tags::get(&t, GT_VISUAL_INFO).unwrap_or(0) == 0 { return Ok("[gadtools] DrawBevelBoxA without GT_VisualInfo: draws nothing".into()); }
    let recessed = tags::get(&t, GTBB_RECESSED).unwrap_or(0) != 0;
    match tags::get(&t, GTBB_FRAME_TYPE).unwrap_or(BBFT_BUTTON) {
        BBFT_BUTTON => {}
        BBFT_RIDGE => return Err(Refusal("DrawBevelBoxA: GTBB_FrameType BBFT_RIDGE is two nested inverted bevels: the kit has no op for it".into())),
        BBFT_ICONDROPBOX => return Err(Refusal("DrawBevelBoxA: GTBB_FrameType BBFT_ICONDROPBOX is two nested inverted bevels: the kit has no op for it".into())),
        other => return Err(Refusal(format!("DrawBevelBoxA: GTBB_FrameType {other} is not a frame type frameiclass draws"))),
    }
    if a.mem.read_u8(rp + RP_MASK).map_err(Refusal::mem("RastPort.Mask"))? != 0xFF { return Err(Refusal("DrawBevelBoxA: a write mask on the RastPort changes what the bevel pens draw".into())); }
    if w <= 0 || h <= 0 { return Ok(format!("[gadtools] DrawBevelBoxA {w}x{h} at {left},{top}: no area")); }
    target::note_native(a, rp, "DrawBevelBoxA", Prim::Bevel { rect: Rect::from_size(left, top, w, h), raised: !recessed }, Rect::from_size(left, top, w, h))?;
    Ok(format!("[gadtools] DrawBevelBoxA {w}x{h} at {left},{top} {}: a native bevel", if recessed { "recessed" } else { "raised" }))
}
