//! The drawing of the built-in classes DOpus's gadgets stand on: buttongclass GM_RENDER, propgclass GM_RENDER, strgclass GM_RENDER
//! (background and text) and sysiclass IM_DRAW (the four scroll arrows). Facts and geometry from AROS rom/intuition
//! (buttongclass.c, propgadgets.c, strgadgets.c UpdateStrGadget, sysiclass.c renderimageframe/arrow loops; APL, re-expressed here).
//!
//! KNOWN DIFFERENCES from AROS, all because the engine has no window decoration class: gadgets flagged as BORDER gadgets
//! (GACT_RIGHTBORDER ...) are drawn by AROS's WinDecorObj (its own arrow and knob artwork); here they get the "standard look"
//! the same sources use for gadgets inside the window (arrows: the frame + triangle drawing; knob: black edge lines + SHINEPEN
//! interior). The default size of a sysiclass arrow (AROS asks the decor) is 14x14. Disabled gadgets are not ghosted,
//! GM_HITTEST / input are not implemented here, and strgclass draws no cursor or scrolled text (DispPos 0).
use alloc::format;
use crate::boopsi_ids::*;
use crate::graphics::layout::SCREEN_DRAWINFO;
use crate::graphics::{fonts, rastport, target};
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_raster::{font::draw_text, paint::{self, Brush}, Rect};

const LEFTIMAGE: u32 = 0x0A;
const UPIMAGE: u32 = 0x0B;
const RIGHTIMAGE: u32 = 0x0C;
const DOWNIMAGE: u32 = 0x0D;
const IDS_SELECTED: u32 = 1;
const IDS_INACTIVESELECTED: u32 = 6;
const IDS_NORMAL: u32 = 0;
const SYSIA_WHICH: u32 = IA + 0x0D;
/// DrawInfo pen indices (intuition/screens.h).
const SHINEPEN: usize = 3;
const SHADOWPEN: usize = 4;
const FILLPEN: usize = 5;
const BACKGROUNDPEN: usize = 7;
const TEXTPEN: usize = 2;
const DEFAULT_ARROW: i32 = 14;

fn pens<U: GuestUi>(a: &Amiga<U>) -> Result<[u8; 12], Refusal> {
    let p = a.mem.read_u32(SCREEN_DRAWINFO + 4).map_err(Refusal::mem("DrawInfo.dri_Pens"))?;
    let mut out = [0u8; 12];
    for (i, o) in out.iter_mut().enumerate() { *o = a.mem.read_u16(p + 2 * i as u32).map_err(Refusal::mem("dri_Pens"))? as u8; }
    Ok(out)
}

/// RectFill with one pen (JAM1), inclusive corners, RastPort coordinates.
fn fill<U: GuestUi>(a: &mut Amiga<U>, rp: u32, (x0, y0, x1, y1): (i32, i32, i32, i32), pen: u8) -> Result<(), Refusal> {
    if x1 < x0 || y1 < y0 { return Ok(()); }
    let b = Brush::new(pen, 0, 0);
    let r = Rect::from_inclusive(x0, y0, x1, y1);
    target::paint(a, rp, "native class render", |bm, clip, (ox, oy)| paint::fill_rect(bm, r.translate(-ox, -oy), &b, None, clip))?;
    Ok(())
}

/// GetGadgetIBox: the gadget's box in window coordinates, resolving the Rel* flags against the window (`gi`'s domain).
fn gadget_box<U: GuestUi>(a: &Amiga<U>, g: u32, gi: u32) -> Result<(i32, i32, i32, i32), Refusal> {
    let m = Refusal::mem("Gadget");
    let (ww, wh) = (a.mem.read_u16(gi + GI_DOMAIN + 4).map_err(&m)? as i32, a.mem.read_u16(gi + GI_DOMAIN + 6).map_err(&m)? as i32);
    let fl = a.mem.read_u16(g + GAD_FLAGS).map_err(&m)? as u32;
    let r = |o| a.mem.read_u16(g + o).map(|v| v as i16 as i32).map_err(Refusal::mem("Gadget"));
    let (mut l, mut t, mut w, mut h) = (r(GAD_LEFT)?, r(GAD_TOP)?, r(GAD_WIDTH)?, r(GAD_HEIGHT)?);
    if fl & GFLG_RELRIGHT != 0 { l += ww - 1; }
    if fl & GFLG_RELBOTTOM != 0 { t += wh - 1; }
    if fl & GFLG_RELWIDTH != 0 { w += ww; }
    if fl & GFLG_RELHEIGHT != 0 { h += wh; }
    Ok((l, t, w, h))
}

/// The state value `DrawImageState` is given (buttongclass: IDS_NORMAL, or IDS_SELECTED when the gadget is selected).
fn arrow_frame<U: GuestUi>(a: &mut Amiga<U>, rp: u32, which: u32, state: u32, p: &[u8; 12], (left, top, width, height): (i32, i32, i32, i32)) -> Result<(), Refusal> {
    let (right, bottom) = (left + width - 1, top + height - 1);
    let sel = state == IDS_SELECTED || state == IDS_INACTIVESELECTED;
    let (leftgodown, topgoright) = (which == UPIMAGE || which == DOWNIMAGE || left == 0, which == LEFTIMAGE || which == RIGHTIMAGE || top == 0);
    let (lo, hi) = if sel { (p[SHADOWPEN], p[SHINEPEN]) } else { (p[SHINEPEN], p[SHADOWPEN]) };
    fill(a, rp, (left, top, left, bottom - if leftgodown { 0 } else { 1 }), lo)?;
    fill(a, rp, (left + 1, top, right - if topgoright { 0 } else { 1 }, top), lo)?;
    fill(a, rp, (right, top + if topgoright { 1 } else { 0 }, right, bottom), hi)?;
    fill(a, rp, (left + if leftgodown { 1 } else { 0 }, bottom, right - 1, bottom), hi)
}

/// sysiclass IM_DRAW for an arrow image of `which` in the box (left, top, width, height) (sysiclass.c LEFTIMAGE...DOWNIMAGE).
pub fn arrow<U: GuestUi>(a: &mut Amiga<U>, rp: u32, which: u32, state: u32, (mut left, mut top, mut width, mut height): (i32, i32, i32, i32)) -> Result<(), Refusal> {
    let p = pens(a)?;
    let (mut hs, mut vs) = (3, 3);
    if width <= 12 { hs = 2; } if width <= 10 { hs = 1; }
    if height <= 12 { vs = 2; } if height <= 10 { vs = 1; }
    arrow_frame(a, rp, which, state, &p, (left, top, width, height))?;
    left += 1; top += 1; width -= 2; height -= 2;
    let bg = if state == IDS_NORMAL || state == IDS_SELECTED { p[FILLPEN] } else { p[BACKGROUNDPEN] };
    fill(a, rp, (left, top, left + width - 1, top + height - 1), bg)?;
    left += hs; top += vs; width -= hs * 2; height -= vs * 2;
    if width <= 0 || height <= 0 { return Ok(()); }
    let (right, bottom) = (left + width - 1, top + height - 1);
    let ink = p[SHADOWPEN];
    match which {
        LEFTIMAGE => { let cy = (height + 1) / 2; for i in 0..cy {
            fill(a, rp, (left + (cy - i - 1) * width / cy, top + i, right - i * width / cy / 2, top + i), ink)?;
            fill(a, rp, (left + (cy - i - 1) * width / cy, bottom - i, right - i * width / cy / 2, bottom - i), ink)?; } }
        RIGHTIMAGE => { let cy = (height + 1) / 2; for i in 0..cy {
            fill(a, rp, (left + i * width / cy / 2, top + i, right - (cy - i - 1) * width / cy, top + i), ink)?;
            fill(a, rp, (left + i * width / cy / 2, bottom - i, right - (cy - i - 1) * width / cy, bottom - i), ink)?; } }
        UPIMAGE => { let cx = (width + 1) / 2; for i in 0..cx {
            fill(a, rp, (left + i, top + (cx - i - 1) * height / cx, left + i, bottom - i * height / cx / 2), ink)?;
            fill(a, rp, (right - i, top + (cx - i - 1) * height / cx, right - i, bottom - i * height / cx / 2), ink)?; } }
        DOWNIMAGE => { let cx = (width + 1) / 2; for i in 0..cx {
            fill(a, rp, (left + i, top + i * height / cx / 2, left + i, bottom - (cx - i - 1) * height / cx), ink)?;
            fill(a, rp, (right - i, top + i * height / cx / 2, right - i, bottom - (cx - i - 1) * height / cx), ink)?; } }
        _ => {}
    }
    Ok(())
}

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

/// IM_DRAW on a native image object `im` at (x, y) (DrawImageState / impDraw): the arrows; other sysiclass images draw nothing
/// yet (logged).
pub fn draw_native_image<U: GuestUi>(a: &mut Amiga<U>, im: u32, rp: u32, x: i32, y: i32, state: u32) -> Result<(), Refusal> {
    let Some(which) = sysi_which(a, im) else { return Err(Refusal(format!("DrawImageState: image object {im:#x} is of a class without native drawing"))) };
    let m = Refusal::mem("Image");
    let r = |o| a.mem.read_u16(im + o).map(|v| v as i16 as i32).map_err(&m);
    let (l, t, w, h) = (x + r(IMG_LEFT)?, y + r(IMG_TOP)?, r(IMG_WIDTH)?, r(IMG_HEIGHT)?);
    if matches!(which, LEFTIMAGE | UPIMAGE | RIGHTIMAGE | DOWNIMAGE) { arrow(a, rp, which, state, (l, t, w, h)) }
    else { a.log.push(format!("[intuition] sysiclass image type {which:#x} at {l},{t}: its drawing is not implemented (nothing drawn)")); Ok(()) }
}

/// buttongclass GM_RENDER (msg = gpRender): a borderless image button draws its image; a plain one its Border.
pub fn button_render<U: GuestUi>(a: &mut Amiga<U>, g: u32, msg: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("gpRender");
    let (gi, rp) = (a.mem.read_u32(msg + 4).map_err(&m)?, a.mem.read_u32(msg + GPR_RPORT).map_err(&m)?);
    if gi == 0 || rp == 0 { return Ok(0); }
    let (l, t, w, h) = gadget_box(a, g, gi)?;
    if w <= 1 || h <= 1 { return Ok(0); }
    let fl = a.mem.read_u16(g + GAD_FLAGS).map_err(&m)? as u32;
    let selected = fl & GFLG_SELECTED != 0;
    if fl & GFLG_GADGIMAGE == 0 { return Ok(0); } // GadgetRender borders: DrawBorder of the classic path is not routed from here yet
    let (render, select) = (a.mem.read_u32(g + GAD_RENDER).map_err(&m)?, a.mem.read_u32(g + GAD_SELECT).map_err(&m)?);
    let (img, state) = if selected && select != 0 { (select, IDS_SELECTED) } else { (render, if selected { IDS_SELECTED } else { IDS_NORMAL }) };
    if img != 0 { crate::graphics::draw_image_at(a, rp, img, l, t, state)?; }
    Ok(0)
}

/// propgclass GM_RENDER: the outline (unless PROPBORDERLESS), the background, and the knob (AUTOKNOB) per CalcKnobSize.
pub fn prop_render<U: GuestUi>(a: &mut Amiga<U>, g: u32, msg: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("PropInfo");
    let (gi, rp) = (a.mem.read_u32(msg + 4).map_err(&m)?, a.mem.read_u32(msg + GPR_RPORT).map_err(&m)?);
    let pi = a.mem.read_u32(g + GAD_SPECIAL).map_err(&m)?;
    if gi == 0 || rp == 0 || pi == 0 { return Ok(0); }
    let (l, t, w, h) = gadget_box(a, g, gi)?;
    if w <= 0 || h <= 0 { return Ok(0); }
    let p = pens(a)?;
    let r = |o| a.mem.read_u16(pi + o).map(u32::from).map_err(Refusal::mem("PropInfo"));
    let (flags, hpot, vpot, hbody, vbody) = (r(0)?, r(2)?, r(4)?, r(6)?, r(8)?);
    const AUTOKNOB: u32 = 1; const FREEHORIZ: u32 = 2; const FREEVERT: u32 = 4; const BORDERLESS: u32 = 8; const NEWLOOK: u32 = 0x10;
    let (mut bx, mut by, mut bw, mut bh) = (l, t, w, h);
    if flags & BORDERLESS == 0 {
        let o = p[SHADOWPEN];
        fill(a, rp, (bx, by, bx + bw - 1, by), o)?; fill(a, rp, (bx, by + bh - 1, bx + bw - 1, by + bh - 1), o)?;
        fill(a, rp, (bx, by, bx, by + bh - 1), o)?; fill(a, rp, (bx + bw - 1, by, bx + bw - 1, by + bh - 1), o)?;
        bx += 1; by += 1; bw -= 2; bh -= 2;
    }
    // Background: the checkerboard of PROPNEWLOOK (pattern 0x5555/0xAAAA: SHADOWPEN where x+y is odd) or the background pen.
    if flags & NEWLOOK != 0 {
        let (fg, bg) = (p[SHADOWPEN], p[BACKGROUNDPEN]);
        let rect = Rect::from_inclusive(bx, by, bx + bw - 1, by + bh - 1);
        target::paint(a, rp, "propgclass background", |bm, clip, (ox, oy)| {
            let mut hit = Rect::default();
            for y in rect.y0..rect.y1 { for x in rect.x0..rect.x1 {
                let (cx, cy) = (x - ox, y - oy);
                if clip.allows(cx, cy) { bm.set(cx, cy, if (x + y) & 1 == 1 { fg } else { bg }); hit = hit.union(&Rect::new(cx, cy, cx + 1, cy + 1)); }
            } }
            hit
        })?;
    } else { fill(a, rp, (bx, by, bx + bw - 1, by + bh - 1), p[BACKGROUNDPEN])?; }
    if flags & AUTOKNOB == 0 { return Ok(0); }
    // CalcKnobSize (borderless: no inner border).
    let (mut kl, mut kt, mut kw, mut kh) = (bx, by, bw, bh);
    if flags & FREEHORIZ != 0 { kw = (bw as u32 * hbody / 0xFFFF) as i32; if kw < 6 { kw = 6; } kl = bx + ((bw - kw) as u32 * hpot / 0xFFFF) as i32; }
    if flags & FREEVERT != 0 { kh = (bh as u32 * vbody / 0xFFFF) as i32; if kh < 4 { kh = 4; } kt = by + ((bh - kh) as u32 * vpot / 0xFFFF) as i32; }
    kw = kw.min(bw); kh = kh.min(bh);
    // The standard knob look.
    if flags & BORDERLESS != 0 {
        fill(a, rp, (kl + kw - 1, kt, kl + kw - 1, kt + kh - 1), p[SHADOWPEN])?;
        fill(a, rp, (kl, kt + kh - 1, kl + kw - 2, kt + kh - 1), p[SHADOWPEN])?;
        kw -= 1; kh -= 1;
    } else {
        if flags & FREEHORIZ != 0 { fill(a, rp, (kl, kt, kl, kt + kh - 1), p[SHADOWPEN])?; fill(a, rp, (kl + kw - 1, kt, kl + kw - 1, kt + kh - 1), p[SHADOWPEN])?; kl += 1; kw -= 2; }
        if flags & FREEVERT != 0 { fill(a, rp, (kl, kt, kl + kw - 1, kt), p[SHADOWPEN])?; fill(a, rp, (kl, kt + kh - 1, kl + kw - 1, kt + kh - 1), p[SHADOWPEN])?; kt += 1; kh -= 2; }
    }
    fill(a, rp, (kl, kt, kl + kw - 1, kt + kh - 1), p[SHINEPEN])?;
    Ok(0)
}

/// strgclass GM_RENDER: the background in the background pen, the text of the StringInfo buffer in the text pen.
pub fn strg_render<U: GuestUi>(a: &mut Amiga<U>, g: u32, msg: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("StringInfo");
    let (gi, rp) = (a.mem.read_u32(msg + 4).map_err(&m)?, a.mem.read_u32(msg + GPR_RPORT).map_err(&m)?);
    let si = a.mem.read_u32(g + GAD_SPECIAL).map_err(&m)?;
    if gi == 0 || rp == 0 || si == 0 { return Ok(0); }
    let (l, t, w, h) = gadget_box(a, g, gi)?;
    if w <= 0 || h <= 0 { return Ok(0); }
    let p = pens(a)?;
    fill(a, rp, (l, t, l + w - 1, t + h - 1), p[BACKGROUNDPEN])?;
    let buf = a.mem.read_u32(si).map_err(&m)?;
    if buf == 0 { return Ok(0); }
    let text = a.mem.read_cstr(buf).map_err(&m)?.into_bytes();
    let (_, font) = fonts::rp_font(a, rp)?;
    let st = rastport::read(a, rp)?;
    let ytop = t + ((h - font.y_size as i32) >> 1) + font.baseline as i32;
    let brush = Brush { fg: p[TEXTPEN], bg: p[BACKGROUNDPEN], mode: 1, mask: st.mask };
    target::paint(a, rp, "strgclass text", |bm, clip, (ox, oy)| draw_text(bm, &font, &text, l - ox, ytop - oy, &brush, 0, clip).1)?;
    Ok(0)
}
