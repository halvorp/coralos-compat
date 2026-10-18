//! frameiclass: an imageclass whose instance data holds the frame type, the contents offsets and the Recessed / EdgesOnly flags;
//! IM_DRAW / IM_DRAWFRAME draw the bevel (and fill the inside), IM_FRAMEBOX computes the box around some contents.
//! Facts and geometry: AROS rom/intuition frameiclass.c (DrawFrame, set_frameiclass, draw_frameiclass, FrameIClass__IM_FRAMEBOX; APL,
//! re-expressed here). Differences: FRAME_SIZE is FRAMESIZE_THIN (AROS's default, intuition_init.c:259), so the "thicken" passes of
//! DrawFrame are no-ops and the offsets are not doubled; the pens come from the engine screen's one DrawInfo (an imp_DrInfo of another
//! screen is ignored); a disabled state is not ghosted (RenderDisabledPattern is not implemented), logged by name.
use crate::boopsi_ids::*;
use crate::boopsi_native::{dispatch_super, image};
use crate::{machine::Amiga, tags, Refusal};
use alloc::format;
use guestui_abi::{GroupStyle, GuestUi};
use retro_raster::{paint::{self, Brush}, Rect};

const FRAME_DEFAULT: u32 = 0;
const FRAME_BUTTON: u32 = 1;
const FRAME_RIDGE: u32 = 2;
const FRAME_ICONDROPBOX: u32 = 3;
pub const IM_DRAWFRAME: u32 = 0x206;
pub const IM_FRAMEBOX: u32 = 0x207;
pub const IA_RECESSED: u32 = IA + 0x15;
pub const IA_EDGES_ONLY: u32 = IA + 0x17;
pub const IA_FRAME_TYPE: u32 = IA + 0x1b;
const FRAMEF_SPECIFY: u32 = 1;
const IDS_SELECTED: u32 = 1;
const IDS_DISABLED: u32 = 2;
const IDS_INACTIVESELECTED: u32 = 6;
const IDS_INACTIVEDISABLED: u32 = 7;
const IDS_SELECTEDDISABLED: u32 = 8;
const SHINEPEN: usize = 3;
const SHADOWPEN: usize = 4;
const FILLPEN: usize = 5;
const BACKGROUNDPEN: usize = 7;
// Instance data: type, HOffset, VOffset, Recessed, EdgesOnly (UWORDs).
const FD_TYPE: u32 = 0;
const FD_HOFF: u32 = 2;
const FD_VOFF: u32 = 4;
const FD_RECESSED: u32 = 6;
const FD_EDGES: u32 = 8;

fn data<U: GuestUi>(a: &Amiga<U>, cl: u32, o: u32) -> u32 { o + a.classes.recs.get(&cl).map(|r| r.inst_offset).unwrap_or(0) }
fn r16<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<i32, Refusal> { a.mem.read_u16(at).map(|v| v as i16 as i32).map_err(Refusal::mem("frameiclass")) }
fn w16<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: i32) -> Result<(), Refusal> { a.mem.write_u16(at, v as u16).map_err(Refusal::mem("frameiclass")) }

/// set_frameiclass: IA_Recessed, IA_EdgesOnly, IA_FrameType (which also sets the offsets: DEFAULT/BUTTON 1, RIDGE 2, ICONDROPBOX 3).
fn set<U: GuestUi>(a: &mut Amiga<U>, fd: u32, tag_list: u32) -> Result<(), Refusal> {
    for (tag, d) in tags::walk(&a.mem, tag_list)? {
        match tag {
            IA_RECESSED => w16(a, fd + FD_RECESSED, (d != 0) as i32)?,
            IA_EDGES_ONLY => w16(a, fd + FD_EDGES, (d != 0) as i32)?,
            IA_FRAME_TYPE => {
                w16(a, fd + FD_TYPE, d as i32)?;
                let off = match d { FRAME_DEFAULT | FRAME_BUTTON => 1, FRAME_RIDGE => 2, FRAME_ICONDROPBOX => 3, _ => r16(a, fd + FD_HOFF)? };
                w16(a, fd + FD_HOFF, off)?; w16(a, fd + FD_VOFF, off)?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn frame<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("frameiclass message");
    let id = a.mem.read_u32(msg).map_err(&m)?;
    match id {
        OM_NEW => {
            let im = image(a, cl, o, msg)?;
            if im == 0 { return Ok(0); }
            let fd = data(a, cl, im);
            for (off, v) in [(FD_TYPE, FRAME_DEFAULT as i32), (FD_HOFF, 1), (FD_VOFF, 1), (FD_RECESSED, 0), (FD_EDGES, 0)] { w16(a, fd + off, v)?; }
            let t = a.mem.read_u32(msg + 4).map_err(&m)?;
            set(a, fd, t)?;
            Ok(im)
        }
        OM_SET | OM_UPDATE => {
            let sup = image(a, cl, o, msg)?;
            let t = a.mem.read_u32(msg + 4).map_err(&m)?;
            let fd = data(a, cl, o);
            set(a, fd, t)?;
            Ok(sup)
        }
        IM_DRAW => {
            let (rp, x, y, st) = (a.mem.read_u32(msg + 4).map_err(&m)?, r16(a, msg + 8)?, r16(a, msg + 10)?, a.mem.read_u32(msg + 12).map_err(&m)?);
            let (w, h) = (r16(a, o + IMG_WIDTH)?, r16(a, o + IMG_HEIGHT)?);
            draw(a, o, rp, x, y, st, (w, h))
        }
        IM_DRAWFRAME => {
            let (rp, x, y, st) = (a.mem.read_u32(msg + 4).map_err(&m)?, r16(a, msg + 8)?, r16(a, msg + 10)?, a.mem.read_u32(msg + 12).map_err(&m)?);
            let (w, h) = (r16(a, msg + 20)?, r16(a, msg + 22)?);
            draw(a, o, rp, x, y, st, (w, h))
        }
        IM_FRAMEBOX => framebox(a, cl, o, msg),
        _ => dispatch_super(a, cl, o, msg),
    }
}

/// FrameIClass__IM_FRAMEBOX: the box of the frame around the contents box (or, FRAMEF_SPECIFY, the given-size frame centred on it).
fn framebox<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("impFrameBox");
    let (contents, out, flags) = (a.mem.read_u32(msg + 4).map_err(&m)?, a.mem.read_u32(msg + 8).map_err(&m)?, a.mem.read_u32(msg + 16).map_err(&m)?);
    let fd = data(a, cl, o);
    let (ho, vo) = (r16(a, fd + FD_HOFF)?, r16(a, fd + FD_VOFF)?);
    let (cl_, ct, cw, ch) = (r16(a, contents)?, r16(a, contents + 2)?, r16(a, contents + 4)?, r16(a, contents + 6)?);
    if flags & FRAMEF_SPECIFY != 0 {
        let (fw, fh) = (r16(a, out + 4)?, r16(a, out + 6)?);
        w16(a, out, cl_ + (cw - fw) / 2)?; w16(a, out + 2, ct + (ch - fh) / 2)?;
    } else {
        for (off, v) in [(0, cl_ - ho), (2, ct - vo), (4, cw + ho * 2), (6, ch + vo * 2)] { w16(a, out + off, v)?; }
    }
    Ok(1)
}

/// RectFill with one pen (JAM1), inclusive corners, RastPort coordinates.
fn rect<U: GuestUi>(a: &mut Amiga<U>, rp: u32, (x0, y0, x1, y1): (i32, i32, i32, i32), pen: u8) -> Result<(), Refusal> {
    if x1 < x0 || y1 < y0 { return Ok(()); }
    let (b, r) = (Brush::new(pen, 0, 0), Rect::from_inclusive(x0, y0, x1, y1));
    crate::graphics::target::paint(a, rp, "frameiclass draw", |bm, clip, (ox, oy)| paint::fill_rect(bm, r.translate(-ox, -oy), &b, None, clip))?;
    Ok(())
}

/// The DrawInfo pens (dri_Pens, 12 entries) of the engine screen.
fn pens<U: GuestUi>(a: &Amiga<U>) -> Result<[u8; 12], Refusal> {
    let p = a.mem.read_u32(crate::graphics::layout::SCREEN_DRAWINFO + 4).map_err(Refusal::mem("DrawInfo.dri_Pens"))?;
    let mut out = [0u8; 12];
    for (i, o) in out.iter_mut().enumerate() { *o = a.mem.read_u16(p + 2 * i as u32).map_err(Refusal::mem("dri_Pens"))? as u8; }
    Ok(out)
}

/// DrawFrame with FRAME_SIZE thin: top/left in `shine`, bottom/right in `shadow` (the "thicken" passes do nothing at that size).
fn bevel<U: GuestUi>(a: &mut Amiga<U>, rp: u32, shine: u8, shadow: u8, (l, t, w, h): (i32, i32, i32, i32)) -> Result<(), Refusal> {
    let (w, h) = (w - 1, h - 1);
    rect(a, rp, (l, t, l, t + h), shine)?; rect(a, rp, (l, t, l + w, t), shine)?;
    rect(a, rp, (l + w, t, l + w, t + h), shadow)?; rect(a, rp, (l + 1, t + h, l + w, t + h), shadow)
}

/// draw_frameiclass for object `im` at offset (x, y) with the given dimensions and IDS_ state.
pub fn draw<U: GuestUi>(a: &mut Amiga<U>, im: u32, rp: u32, x: i32, y: i32, state: u32, (width, height): (i32, i32)) -> Result<u32, Refusal> {
    let Some(cl) = a.classes.class_of(im) else { return Err(Refusal(format!("frameiclass draw: {im:#x} is not an object"))) };
    if rp == 0 { return Ok(0); }
    let fd = data(a, cl, im);
    let (ty, ho, vo) = (r16(a, fd + FD_TYPE)? as u32, r16(a, fd + FD_HOFF)?, r16(a, fd + FD_VOFF)?);
    let (recessed, edges) = (r16(a, fd + FD_RECESSED)? != 0, r16(a, fd + FD_EDGES)? != 0);
    let pens = pens(a)?;
    let selected = matches!(state, IDS_SELECTED | IDS_INACTIVESELECTED | IDS_SELECTEDDISABLED);
    let (mut shine, mut shadow) = (pens[SHINEPEN], pens[SHADOWPEN]);
    if recessed ^ selected { core::mem::swap(&mut shine, &mut shadow); }
    let (left, top) = (x + r16(a, im + IMG_LEFT)?, y + r16(a, im + IMG_TOP)?);
    match ty {
        FRAME_DEFAULT | FRAME_BUTTON => bevel(a, rp, shine, shadow, (left, top, width, height))?,
        FRAME_RIDGE => {
            bevel(a, rp, shine, shadow, (left, top, width, height))?;
            bevel(a, rp, shadow, shine, (left + ho / 2, top + vo / 2, width - ho, height - vo))?;
        }
        FRAME_ICONDROPBOX => {
            let (h2, v2) = (ho * 2 / 3, vo * 2 / 3);
            bevel(a, rp, shine, shadow, (left, top, width, height))?;
            bevel(a, rp, shadow, shine, (left + h2, top + v2, width - h2 * 2, height - v2 * 2))?;
        }
        _ => {}
    }
    if !edges { rect(a, rp, (left + ho, top + vo, left + width - ho - 1, top + height - vo - 1), if selected { pens[FILLPEN] } else { pens[BACKGROUNDPEN] })?; }
    if matches!(state, IDS_DISABLED | IDS_INACTIVEDISABLED | IDS_SELECTEDDISABLED) {
        a.log.push(format!("[intuition] frameiclass {im:#x}: disabled state at {left},{top}: RenderDisabledPattern is not implemented (drawn as enabled)"));
    }
    Ok(1)
}

/// The bevel role a frameiclass object stands for (what a gadget framed by it becomes natively), `None` if `im` is not one.
pub fn bevel_of<U: GuestUi>(a: &Amiga<U>, im: u32) -> Option<GroupStyle> {
    let cl = a.classes.class_of(im)?;
    if a.classes.recs.get(&cl)?.native != Some(crate::iclass::Native::Frame) { return None; }
    let fd = data(a, cl, im);
    let recessed = r16(a, fd + FD_RECESSED).ok()? != 0;
    Some(match r16(a, fd + FD_TYPE).ok()? as u32 {
        FRAME_RIDGE | FRAME_ICONDROPBOX => GroupStyle::Plain,
        _ if recessed => GroupStyle::Recessed,
        _ => GroupStyle::Raised,
    })
}
