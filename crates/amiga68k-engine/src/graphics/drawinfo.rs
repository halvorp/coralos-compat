//! intuition.library's drawing helpers over a RastPort: GetScreenDrawInfo, DrawImage/DrawImageState/EraseImage/PointInImage
//! for classic planar `struct Image`s, DrawBorder, PrintIText/IntuiTextLength, and the refresh/pointer/beep calls that
//! have no effect on a host-composited window (named in the log, never silent).
use super::layout::*;
use super::{fonts, rastport, target};
use crate::{machine::Amiga, screen, Refusal};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::{paint::{self, Brush, JAM2}, Rect};

const CUSTOMIMAGEDEPTH: i32 = -1;

pub fn get_screen_draw_info<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = cpu.areg(0);
    if s != screen::SCREEN && s != 0 { return Err(Refusal(format!("GetScreenDrawInfo {s:#x}: not the public screen"))); }
    cpu.set_d0(SCREEN_DRAWINFO);
    Ok(format!("[intuition] GetScreenDrawInfo -> {SCREEN_DRAWINFO:#x}"))
}

pub fn free_screen_draw_info<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let d = cpu.areg(1);
    if d != 0 && d != SCREEN_DRAWINFO { return Err(Refusal(format!("FreeScreenDrawInfo {d:#x}: not the DrawInfo GetScreenDrawInfo returned"))); }
    Ok("[intuition] FreeScreenDrawInfo".into())
}

struct Im { left: i32, top: i32, w: i32, h: i32, depth: i32, data: u32, pick: u8, onoff: u8, next: u32 }

fn read_image<U: GuestUi>(a: &Amiga<U>, im: u32) -> Result<Im, Refusal> {
    let m = Refusal::mem("Image");
    let w = |o| a.mem.read_i16(im + o).map(i32::from).map_err(&m);
    Ok(Im { left: w(0)?, top: w(2)?, w: w(4)?, h: w(6)?, depth: w(IM_DEPTH)?, data: a.mem.read_u32(im + IM_DATA).map_err(&m)?,
        pick: a.mem.read_u8(im + IM_PLANE_PICK).map_err(&m)?, onoff: a.mem.read_u8(im + IM_PLANE_ON_OFF).map_err(&m)?, next: a.mem.read_u32(im + IM_NEXT).map_err(&m)? })
}

/// The pen of every pixel of a classic Image, `dest_depth` planes deep: planes in PlanePick come from the image data in
/// order, the others from PlaneOnOff.
fn image_pens<U: GuestUi>(a: &Amiga<U>, i: &Im, dest_depth: u8) -> Result<Vec<u8>, Refusal> {
    let words = ((i.w + 15) >> 4) as u32;
    let mut out = Vec::with_capacity((i.w * i.h).max(0) as usize);
    for y in 0..i.h as u32 {
        for x in 0..i.w as u32 {
            let (mut pen, mut k) = (0u8, 0u32);
            for p in 0..dest_depth {
                let on = if i.pick >> p & 1 != 0 {
                    let at = i.data + 2 * ((k * i.h as u32 + y) * words + (x >> 4));
                    k += 1;
                    if k as i32 > i.depth { false } else { a.mem.read_u16(at).map_err(Refusal::mem("Image data"))? >> (15 - (x & 15)) & 1 != 0 }
                } else { i.onoff >> p & 1 != 0 };
                if on { pen |= 1 << p; }
            }
            out.push(pen);
        }
    }
    Ok(out)
}

fn draw_chain<U: GuestUi>(a: &mut Amiga<U>, rp: u32, mut im: u32, left: i32, top: i32, state: u32, what: &str) -> Result<usize, Refusal> {
    let mask = a.mem.read_u8(rp + RP_MASK).map_err(Refusal::mem("Mask"))?;
    let mut n = 0;
    while im != 0 && n < 64 {
        let i = read_image(a, im)?;
        if i.depth == CUSTOMIMAGEDEPTH {
            // A BOOPSI image object: IM_DRAW. Native classes draw host-side (boopsi_draw.rs); a guest class would need a guest call.
            if a.classes.class_of(im).and_then(|c| a.classes.recs.get(&c)).map_or(true, |r| r.native.is_none()) {
                return Err(Refusal(format!("{what}: image {im:#x} is an image object of a guest class (IM_DRAW through a guest dispatcher is not implemented here)")));
            }
            crate::boopsi_draw::draw_native_image(a, im, rp, left, top, state)?;
            im = i.next; n += 1;
            continue;
        }
        if i.data == 0 { return Err(Refusal(format!("{what}: image {im:#x} has no ImageData"))); }
        let dd = match target::resolve(a, rp)? { target::Dest::Bitmap { bm, .. } => a.gfx.bitmaps[&bm].bm.depth(), target::Dest::Dropped(_) => 8 };
        let pens = image_pens(a, &i, dd)?;
        let (x0, y0) = (left + i.left, top + i.top);
        let prim = a.gfx.icon_images.get(&im).map(|&role| super::record::Prim::Icon { rect: Rect::from_size(x0, y0, i.w, i.h), role });
        target::paint_as(a, rp, what, prim, |bm, clip, (ox, oy)| {
            let mut hit = Rect::default();
            for y in 0..i.h { for x in 0..i.w {
                let (bx, by) = (x0 + x - ox, y0 + y - oy);
                if clip.allows(bx, by) {
                    let old = bm.get(bx, by).unwrap_or(0);
                    bm.set(bx, by, (old & !mask) | (pens[(y * i.w + x) as usize] & mask));
                    hit = hit.union(&Rect::new(bx, by, bx + 1, by + 1));
                }
            } }
            hit
        })?;
        im = i.next; n += 1;
    }
    Ok(n)
}

/// DrawImage(a0 = rp, a1 = image, d0 = left, d1 = top).
pub fn draw_image<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, im, l, t) = (cpu.areg(0), cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let n = draw_chain(a, rp, im, l, t, 0, "DrawImage")?;
    Ok(format!("[intuition] DrawImage {im:#x} ({n} images) at {l},{t}"))
}

/// DrawImageState(a0 = rp, a1 = image, d0 = left, d1 = top, d2 = state, a2 = DrawInfo): classic images draw the same in
/// every state (the ghosting of IDS_DISABLED is a BOOPSI sysiclass job).
pub fn draw_image_state<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, im, l, t, st) = (cpu.areg(0), cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)), cpu.dreg(2));
    let n = draw_chain(a, rp, im, l, t, st, "DrawImageState")?;
    Ok(format!("[intuition] DrawImageState {im:#x} ({n} images) at {l},{t} state {st}"))
}

/// EraseImage(a0 = rp, a1 = image, d0 = left, d1 = top): the image's box in the background pen.
pub fn erase_image<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, mut im, l, t) = (cpu.areg(0), cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let st = rastport::read(a, rp)?;
    let b = Brush { fg: st.bg, bg: st.bg, mode: JAM2, mask: st.mask };
    let mut n = 0;
    while im != 0 && n < 64 {
        let i = read_image(a, im)?;
        let r = Rect::from_size(l + i.left, t + i.top, i.w, i.h);
        target::paint(a, rp, "EraseImage", |bm, clip, (ox, oy)| paint::fill_rect(bm, r.translate(-ox, -oy), &b, None, clip))?;
        im = i.next; n += 1;
    }
    Ok(format!("[intuition] EraseImage {n} images"))
}

/// PointInImage(d0 = packed x<<16|y, a0 = image).
pub fn point_in_image<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (p, im) = (cpu.dreg(0), cpu.areg(0));
    let i = read_image(a, im)?;
    let (x, y) = ((p >> 16) as i16 as i32, p as u16 as i16 as i32);
    let hit = x >= 0 && y >= 0 && x < i.w && y < i.h;
    cpu.set_d0(hit as u32);
    Ok(format!("[intuition] PointInImage {x},{y} in {}x{} -> {hit}", i.w, i.h))
}

/// DrawBorder(a0 = rp, a1 = border, d0 = left, d1 = top): each Border chains lines in its own pens and mode.
pub fn draw_border<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, bd, l, t) = (cpu.areg(0), cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    draw_borders(a, rp, bd, l, t)
}

/// DrawBorder's body: the Border chain `bd` into `rp` at (l, t). Shared with the Requester renderer (request_region.rs).
pub(crate) fn draw_borders<U: GuestUi>(a: &mut Amiga<U>, rp: u32, mut bd: u32, l: i32, t: i32) -> Result<String, Refusal> {
    let m = Refusal::mem("Border");
    let st = rastport::read(a, rp)?;
    let mut n = 0;
    while bd != 0 && n < 64 {
        let (bl, bt) = (a.mem.read_i16(bd).map_err(&m)? as i32, a.mem.read_i16(bd + 2).map_err(&m)? as i32);
        let (fg, bg, mode) = (a.mem.read_u8(bd + BD_FRONT_PEN).map_err(&m)?, a.mem.read_u8(bd + BD_BACK_PEN).map_err(&m)?, a.mem.read_u8(bd + BD_DRAW_MODE).map_err(&m)?);
        let (count, xy, next) = (a.mem.read_u8(bd + BD_COUNT).map_err(&m)? as i8, a.mem.read_u32(bd + BD_XY).map_err(&m)?, a.mem.read_u32(bd + BD_NEXT).map_err(&m)?);
        let pts: Vec<(i32, i32)> = (0..count.max(0) as u32).map(|i| Ok((l + bl + a.mem.read_i16(xy + 4 * i).map_err(&m)? as i32, t + bt + a.mem.read_i16(xy + 4 * i + 2).map_err(&m)? as i32))).collect::<Result<_, Refusal>>()?;
        let brush = Brush { fg, bg, mode: mode & 7, mask: st.mask };
        let mut phase = 15u8;
        let prim = (mode & 7 <= 1 && st.mask == 0xFF && !pts.is_empty()).then(|| super::record::Prim::Lines { pts: pts.clone(), pen: fg });
        target::paint_as(a, rp, "DrawBorder", prim, |bm, clip, (ox, oy)| {
            let mut hit = Rect::default();
            for w in pts.windows(2) { hit = hit.union(&paint::line(bm, (w[0].0 - ox, w[0].1 - oy), (w[1].0 - ox, w[1].1 - oy), &brush, 0xFFFF, &mut phase, clip)); }
            if pts.len() == 1 { hit = paint::plot(bm, pts[0].0 - ox, pts[0].1 - oy, &brush, clip); }
            hit
        })?;
        bd = next; n += 1;
    }
    Ok(format!("[intuition] DrawBorder {n} borders at {l},{t}"))
}

/// The font an IntuiText names (a resident TextAttr) or the RastPort's.
fn itext_font<U: GuestUi>(a: &mut Amiga<U>, it: u32, rp_font: u32) -> Result<u32, Refusal> {
    let ta = a.mem.read_u32(it + IT_FONT).map_err(Refusal::mem("IntuiText.ITextFont"))?;
    if ta == 0 { return Ok(rp_font); }
    let m = Refusal::mem("TextAttr");
    let name = a.mem.read_cstr(a.mem.read_u32(ta + TA_NAME).map_err(&m)?).map_err(&m)?;
    let y = a.mem.read_u16(ta + TA_Y_SIZE).map_err(&m)?;
    Ok(fonts::find_resident(a, &name, y)?.unwrap_or(rp_font))
}

/// PrintIText(a0 = rp, a1 = IntuiText, d0 = left, d1 = top).
pub fn print_itext<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, it, l, t) = (cpu.areg(0), cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    print_itexts(a, rp, it, l, t)
}

/// PrintIText's body: the IntuiText chain `it` into `rp` at (l, t). Shared with the Requester renderer (request_region.rs).
pub(crate) fn print_itexts<U: GuestUi>(a: &mut Amiga<U>, rp: u32, mut it: u32, l: i32, t: i32) -> Result<String, Refusal> {
    let m = Refusal::mem("IntuiText");
    let mask = a.mem.read_u8(rp + RP_MASK).map_err(&m)?;
    let (rp_font, mut n) = (a.mem.read_u32(rp + RP_FONT).map_err(&m)?, 0);
    let saved = rastport::read(a, rp)?;
    while it != 0 && n < 64 {
        let (fg, bg, mode) = (a.mem.read_u8(it + IT_FRONT_PEN).map_err(&m)?, a.mem.read_u8(it + IT_BACK_PEN).map_err(&m)?, a.mem.read_u8(it + IT_DRAW_MODE).map_err(&m)?);
        let (x, y) = (l + a.mem.read_i16(it + IT_LEFT).map_err(&m)? as i32, t + a.mem.read_i16(it + IT_TOP).map_err(&m)? as i32);
        let (tp, next) = (a.mem.read_u32(it + IT_TEXT).map_err(&m)?, a.mem.read_u32(it + IT_NEXT).map_err(&m)?);
        let tf = itext_font(a, it, if rp_font == 0 { fonts::default_font(a)? } else { rp_font })?;
        let font = fonts::font_at(a, tf)?;
        let text = if tp == 0 { Vec::new() } else { a.mem.read_cstr(tp).map_err(&m)?.into_bytes() };
        let brush = Brush { fg, bg, mode: mode & 7, mask };
        let base = font.baseline as i32;
        let prim = super::record::text(&rastport::Rp { fg, bg, mode: mode & 7, ..saved }, &font, &text, (x, y + base));
        target::paint_as(a, rp, "PrintIText", prim, |bm, clip, (ox, oy)| retro_raster::font::draw_text(bm, &font, &text, x - ox, y + base - oy, &brush, saved.algo, clip).1)?;
        it = next; n += 1;
    }
    Ok(format!("[intuition] PrintIText {n} texts at {l},{t}"))
}

/// IntuiTextLength(a0 = IntuiText): the pixel width of the first text (and chained ones are separate).
pub fn intui_text_length<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let it = cpu.areg(0);
    let m = Refusal::mem("IntuiText");
    let tf = itext_font(a, it, fonts::default_font(a)?)?;
    let font = fonts::font_at(a, tf)?;
    let tp = a.mem.read_u32(it + IT_TEXT).map_err(&m)?;
    let w = if tp == 0 { 0 } else { font.text_length(a.mem.read_cstr(tp).map_err(&m)?.as_bytes()) };
    cpu.set_d0(w as u32);
    Ok(format!("[intuition] IntuiTextLength -> {w}"))
}

/// ObtainGIRPort(a0 = GadgetInfo) -> the gadget's window RastPort (shared, not cloned; tasks switch only after a library call, so no lock is needed).
pub fn obtain_gi_rport<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let gi = cpu.areg(0);
    let win = a.mem.read_u32(gi + 4).map_err(Refusal::mem("GadgetInfo.gi_Window"))?;
    let rp = if win == 0 { 0 } else { a.mem.read_u32(win + WD_RPORT).map_err(Refusal::mem("Window.RPort"))? };
    cpu.set_d0(rp);
    Ok(format!("[intuition] ObtainGIRPort window {win:#x} -> {rp:#x}"))
}

/// Calls with nothing to do on a host-composited window, accepted and named in the log.
pub fn log_only<U: GuestUi>(_a: &mut Amiga<U>, _cpu: &mut Cpu) -> Result<String, Refusal> {
    Ok("[intuition] refresh/pointer/beep call: no effect on a host-composited window".into())
}

/// BeginRefresh / BeginUpdate answer TRUE: the canvas is always whole, so a redraw may proceed.
pub fn true_noop<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(1);
    Ok("[intuition] BeginRefresh/BeginUpdate: canvas is always whole".into())
}

/// ScrollWindowRaster(a1 = window, d0..d5): ScrollRaster on the window's RPort.
pub fn scroll_window_raster<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, dx, dy) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let r = Rect::from_inclusive(sx(cpu.dreg(2)), sx(cpu.dreg(3)), sx(cpu.dreg(4)), sx(cpu.dreg(5)));
    let rp = a.mem.read_u32(win + WD_RPORT).map_err(Refusal::mem("Window.RPort"))?;
    super::draw::scroll(a, rp, dx, dy, r)?;
    Ok(format!("[intuition] ScrollWindowRaster {win:#x} by ({dx},{dy})"))
}

/// GetScreenData(a0 = buffer, d0 = size, d1 = type, a1 = screen): the public screen's struct, copied.
pub fn get_screen_data<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (buf, size, ty) = (cpu.areg(0), cpu.dreg(0) as u16 as u32, cpu.dreg(1) as u16);
    if ty != 1 && ty != 2 { return Err(Refusal(format!("GetScreenData type {ty}: only the Workbench/custom screen is implemented"))); }
    let n = size.min(346);
    let bytes = a.mem.bytes(a.mem.span(screen::SCREEN, n).map_err(Refusal::mem("Screen"))?).map_err(Refusal::mem("Screen"))?.to_vec();
    a.mem.write_bytes(buf, &bytes).map_err(Refusal::mem("GetScreenData buffer"))?;
    cpu.set_d0(1);
    Ok(format!("[intuition] GetScreenData {n} bytes"))
}

/// DrawImageState for `img` (classic Image chain or a BOOPSI image object) at (x, y): the entry the native gadget classes use.
pub fn draw_image_at<U: GuestUi>(a: &mut Amiga<U>, rp: u32, img: u32, x: i32, y: i32, state: u32) -> Result<(), Refusal> { draw_chain(a, rp, img, x, y, state, "DrawImageState").map(|_| ()) }

/// SetRast-style fill of the box (x, y, w, h) of `rp` in pen `pen` (a Requester's BackFill, requesters.c:91).
pub(crate) fn fill_pen<U: GuestUi>(a: &mut Amiga<U>, rp: u32, (x, y, w, h): (i32, i32, i32, i32), pen: u8) -> Result<(), Refusal> {
    let st = rastport::read(a, rp)?;
    let b = Brush { fg: pen, bg: pen, mode: JAM2, mask: st.mask };
    let r = Rect::from_size(x, y, w, h);
    target::paint(a, rp, "Requester BackFill", |bm, clip, (ox, oy)| paint::fill_rect(bm, r.translate(-ox, -oy), &b, None, clip))?;
    Ok(())
}
