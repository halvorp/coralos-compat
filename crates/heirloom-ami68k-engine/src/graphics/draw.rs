//! Lines, pixels, rectangle fills, erases, scrolling: the drawing calls that read RastPort state and write the
//! destination bitmap through heirloom-raster.
use super::layout::*;
use super::{rastport::{self, Rp}, target};
use crate::{machine::Amiga, Refusal};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::{blit::blit_within, paint::{self, Brush, Clip, Pattern, JAM2}, Rect, Region};

/// The AreaPtrn as rows, if the RastPort has a single-plane one (AreaPtSz > 0: 2^AreaPtSz words).
pub(super) fn pattern<U: GuestUi>(a: &Amiga<U>, st: &Rp) -> Result<Option<Pattern>, Refusal> {
    if st.area_ptrn == 0 || st.area_pt_sz == 0 { return Ok(None); }
    if st.area_pt_sz < 0 { return Err(Refusal(format!("AreaPtrn with AreaPtSz {} (multi-plane pattern) is not implemented", st.area_pt_sz))); }
    if st.area_pt_sz > 8 { return Err(Refusal(format!("AreaPtSz {} is out of range", st.area_pt_sz))); }
    let rows = (0..1u32 << st.area_pt_sz).map(|i| a.mem.read_u16(st.area_ptrn + 2 * i).map_err(Refusal::mem("AreaPtrn"))).collect::<Result<Vec<_>, _>>()?;
    Ok(Some(Pattern { rows }))
}

/// A brush that paints `pen` solidly, whatever the RastPort's mode (the erase calls).
fn solid(pen: u8, mask: u8) -> Brush { Brush { fg: pen, bg: pen, mode: JAM2, mask } }

/// Draw(a1 = rp, d0 = x, d1 = y): a line from the cursor, in the pen/mode/LinePtrn; the cursor moves to (x,y).
pub fn draw<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { draw_to(a, cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1))) }

/// `Draw` with its arguments: a line from the cursor to (x, y).
pub fn draw_to<U: GuestUi>(a: &mut Amiga<U>, rp: u32, x: i32, y: i32) -> Result<String, Refusal> {
    let st = rastport::read(a, rp)?;
    let (brush, mut phase) = (st.brush(), st.linpatcnt);
    let ptrn = if st.mode & INVERSVID != 0 { !st.line_ptrn } else { st.line_ptrn };
    // COMPLEMENT lines that continue another segment skip the shared end pixel, or it would be inverted twice.
    let skip_first = brush.mode & COMPLEMENT != 0 && st.flags & FRST_DOT == 0;
    let from = st.cp;
    target::paint_as(a, rp, "Draw", super::record::line(&st, from, (x, y)).filter(|_| !skip_first), |bm, clip, (ox, oy)| {
        let (p0, p1) = ((from.0 - ox, from.1 - oy), (x - ox, y - oy));
        if skip_first && p0 != p1 {
            let mut g = clip.region.cloned().unwrap_or_else(|| Region::from_rect(clip.rect));
            g.clear_rect(Rect::new(p0.0, p0.1, p0.0 + 1, p0.1 + 1));
            return paint::line(bm, p0, p1, &brush, ptrn, &mut phase, &Clip { rect: clip.rect, region: Some(&g) });
        }
        paint::line(bm, p0, p1, &brush, ptrn, &mut phase, clip)
    })?;
    rastport::set_cp(a, rp, x, y)?;
    a.mem.write_u8(rp + RP_LINPATCNT, phase).map_err(Refusal::mem("linpatcnt"))?;
    a.mem.write_u16(rp + RP_FLAGS, st.flags & !FRST_DOT).map_err(Refusal::mem("Flags"))?;
    Ok(format!("[graphics] Draw {rp:#x} {},{} -> {x},{y}", from.0, from.1))
}

/// RectFill(a1 = rp, d0..d3 = xMin yMin xMax yMax): the pen (or AreaPtrn) over an inclusive rectangle.
pub fn rect_fill<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x0, y0, x1, y1) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)), sx(cpu.dreg(2)), sx(cpu.dreg(3)));
    let st = rastport::read(a, rp)?;
    let (brush, pat) = (st.brush(), pattern(a, &st)?);
    let r = Rect::from_inclusive(x0, y0, x1, y1);
    target::paint_as(a, rp, "RectFill", super::record::fill(&st), |bm, clip, (ox, oy)| paint::fill_rect(bm, r.translate(-ox, -oy), &brush, pat.as_ref(), clip))?;
    Ok(format!("[graphics] RectFill {rp:#x} ({x0},{y0})-({x1},{y1}) pen {} mode {}", st.fg, st.mode))
}

/// The pen the erase calls use: BgPen (the RastPort's background), as AROS's layer-less EraseRect does.
fn erase_pen(st: &Rp) -> u8 { st.bg }

/// EraseRect(a1 = rp, d0..d3 = xMin yMin xMax yMax).
pub fn erase_rect<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x0, y0, x1, y1) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)), sx(cpu.dreg(2)), sx(cpu.dreg(3)));
    let st = rastport::read(a, rp)?;
    let (b, r) = (solid(erase_pen(&st), st.mask), Rect::from_inclusive(x0, y0, x1, y1));
    target::paint_as(a, rp, "EraseRect", super::record::solid(&st, erase_pen(&st)), |bm, clip, (ox, oy)| paint::fill_rect(bm, r.translate(-ox, -oy), &b, None, clip))?;
    Ok(format!("[graphics] EraseRect {rp:#x} ({x0},{y0})-({x1},{y1})"))
}

/// SetRast(a1 = rp, d0 = pen): every pixel of the RastPort's bitmap/layer.
pub fn set_rast<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, pen) = (cpu.areg(1), cpu.dreg(0) as u8);
    let mask = a.mem.read_u8(rp + RP_MASK).map_err(Refusal::mem("Mask"))?;
    let b = solid(pen, mask);
    let prim = (mask == 0xFF).then_some(super::record::Prim::Fill { pen });
    target::paint_as(a, rp, "SetRast", prim, |bm, clip, _| { let all = bm.rect(); paint::fill_rect(bm, all, &b, None, clip) })?;
    Ok(format!("[graphics] SetRast {rp:#x} pen {pen}"))
}

/// ClearEOL(a1 = rp): from the cursor to the right edge, one text line high (TxHeight, top = cp_y - baseline).
pub fn clear_eol<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rp = cpu.areg(1);
    let st = rastport::read(a, rp)?;
    let (h, base) = (a.mem.read_u16(rp + RP_TX_HEIGHT).map_err(Refusal::mem("TxHeight"))? as i32, a.mem.read_u16(rp + RP_TX_BASELINE).map_err(Refusal::mem("TxBaseline"))? as i32);
    let pen = if st.mode & JAM2 != 0 { st.bg } else { 0 };
    let b = solid(pen, st.mask);
    let (x, top) = (st.cp.0, st.cp.1 - base);
    target::paint_as(a, rp, "ClearEOL", super::record::solid(&st, pen), |bm, clip, (ox, oy)| { let w = bm.width(); paint::fill_rect(bm, Rect::new(x - ox, top - oy, w, top - oy + h), &b, None, clip) })?;
    Ok(format!("[graphics] ClearEOL {rp:#x} from x={x}"))
}

/// ClearScreen(a1 = rp): ClearEOL, then everything below that line.
pub fn clear_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rp = cpu.areg(1);
    let st = rastport::read(a, rp)?;
    let (h, base) = (a.mem.read_u16(rp + RP_TX_HEIGHT).map_err(Refusal::mem("TxHeight"))? as i32, a.mem.read_u16(rp + RP_TX_BASELINE).map_err(Refusal::mem("TxBaseline"))? as i32);
    let b = solid(if st.mode & JAM2 != 0 { st.bg } else { 0 }, st.mask);
    let (x, top) = (st.cp.0, st.cp.1 - base);
    target::paint(a, rp, "ClearScreen", |bm, clip, (ox, oy)| {
        let (w, bh) = (bm.width(), bm.height());
        let r1 = paint::fill_rect(bm, Rect::new(x - ox, top - oy, w, top - oy + h), &b, None, clip);
        r1.union(&paint::fill_rect(bm, Rect::new(0, top - oy + h, w, bh), &b, None, clip))
    })?;
    Ok(format!("[graphics] ClearScreen {rp:#x}"))
}

/// ReadPixel(a1 = rp, d0 = x, d1 = y) -> the pen, or -1 outside the clip.
pub fn read_pixel<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x, y) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let v = target::read_pixel(a, rp, x, y)?;
    cpu.set_d0(v.map_or(u32::MAX, |p| p as u32));
    Ok(format!("[graphics] ReadPixel {rp:#x} {x},{y} -> {}", v.map_or(-1, |p| p as i32)))
}

/// WritePixel(a1 = rp, d0 = x, d1 = y) -> 0, or -1 if the pixel is clipped away.
pub fn write_pixel<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x, y) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let st = rastport::read(a, rp)?;
    let b = st.brush();
    let r = target::paint(a, rp, "WritePixel", |bm, clip, (ox, oy)| paint::plot(bm, x - ox, y - oy, &b, clip))?;
    cpu.set_d0(if r.is_empty() { u32::MAX } else { 0 });
    Ok(format!("[graphics] WritePixel {rp:#x} {x},{y} -> {}", if r.is_empty() { -1 } else { 0 }))
}

/// Scroll `r` (RastPort coordinates) of `rp` by (-dx, -dy); the uncovered strip is erased with the background pen.
pub fn scroll<U: GuestUi>(a: &mut Amiga<U>, rp: u32, dx: i32, dy: i32, r: Rect) -> Result<(), Refusal> {
    let st = rastport::read(a, rp)?;
    let b = solid(erase_pen(&st), st.mask);
    let mask = st.mask;
    target::paint(a, rp, "ScrollRaster", |bm, clip, (ox, oy)| {
        let r = r.translate(-ox, -oy).intersect(&bm.rect());
        let inner = Clip { rect: r.intersect(&clip.rect), region: clip.region };
        // destination (x,y) takes the pixel at (x+dx, y+dy): a within-bitmap blit from the shifted source
        let moved = blit_within(bm, r.x0, r.y0, r.x0 + dx, r.y0 + dy, r.width(), r.height(), 0xC0, mask, None, &inner);
        // what was not fed from inside the rectangle: r minus (r shifted by -d)
        let keep = r.intersect(&r.translate(-dx, -dy));
        let mut hit = moved;
        for p in r.subtract(&keep) { hit = hit.union(&paint::fill_rect(bm, p, &b, None, &inner)); }
        hit
    })?;
    Ok(())
}

/// ScrollRaster(a1 = rp, d0 = dx, d1 = dy, d2..d5 = xMin yMin xMax yMax).
pub fn scroll_raster<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, dx, dy) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let r = Rect::from_inclusive(sx(cpu.dreg(2)), sx(cpu.dreg(3)), sx(cpu.dreg(4)), sx(cpu.dreg(5)));
    scroll(a, rp, dx, dy, r)?;
    Ok(format!("[graphics] ScrollRaster {rp:#x} by ({dx},{dy}) in ({},{})-({},{})", r.x0, r.y0, r.x1 - 1, r.y1 - 1))
}

/// PolyDraw(a1 = rp, d0 = count, a0 = array of WORD x, y pairs): Move to the first point, Draw to each of the others.
pub fn poly_draw<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, n, arr) = (cpu.areg(1), cpu.dreg(0) as u16 as u32, cpu.areg(0));
    for i in 0..n {
        let (x, y) = (a.mem.read_u16(arr + 4 * i).map_err(Refusal::mem("PolyDraw array"))? as u32, a.mem.read_u16(arr + 4 * i + 2).map_err(Refusal::mem("PolyDraw array"))? as u32);
        cpu.set_areg(1, rp); cpu.set_d0(x); cpu.set_dreg(1, y);
        if i == 0 { super::rastport::move_(a, cpu)?; } else { draw(a, cpu)?; }
    }
    Ok(format!("[graphics] PolyDraw {rp:#x} {n} points"))
}
