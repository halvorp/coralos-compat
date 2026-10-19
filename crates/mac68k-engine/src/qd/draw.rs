//! The pen and the shape-drawing traps: PenNormal/PenSize/PenMode/PenPat/BackPat/HidePen/ShowPen/Get-SetPenState,
//! MoveTo/Move/LineTo/Line/GetPen, and Frame/Paint/Erase/Invert/Fill for Rect, Oval, RoundRect and Rgn.
//!
//! Transfer modes. A pattern mode (8..15) or a source mode (0..7) means the same thing for a pattern fill, so `mode & 7`
//! selects the operation: 0 copy, 1 or, 2 xor, 3 bic (clear), 4..7 the same with the pattern/source inverted
//! ([`brush_for`] maps them onto `retro-raster`'s stencil modes). Black is 1.
//!
//! Pen hidden (`pnVis < 0`) draws nothing but still moves; with a region open, Frame* records the shape instead.
use super::port::*;
use super::shapes::Shape;
use super::target::{self, clip_of, clip_region, current_port};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::paint::{stencil, Brush, COMPLEMENT, INVERSVID, JAM1, JAM2};
use retro_raster::{Rect, Region};

pub const BLACK: [u8; 8] = [0xFF; 8];

/// Transfer mode -> the raster stencil brush (fg 1 = black).
pub fn brush_for(mode: u16) -> Brush {
    match mode & 7 {
        0 => Brush::new(1, 0, JAM2), 1 => Brush::new(1, 0, JAM1), 2 => Brush::new(1, 0, COMPLEMENT), 3 => Brush::new(0, 0, JAM1),
        4 => Brush::new(1, 0, JAM2 | INVERSVID), 5 => Brush::new(1, 0, JAM1 | INVERSVID), 6 => Brush::new(1, 0, COMPLEMENT | INVERSVID),
        _ => Brush::new(0, 0, JAM1 | INVERSVID),
    }
}

/// Draw `region` (LOCAL coordinates) into `port` with transfer `mode` and pattern `pat`, through the port's clip.
pub fn paint<U: GuestUi>(m: &mut Mac<U>, port: u32, region: &Region, mode: u16, pat: &[u8; 8]) -> Result<(), Refusal> { paint_with(m, port, region, mode, pat, None) }

/// [`paint`], recording a pen line as ONE line op (`line`: its end points, LOCAL coordinates) when nothing of it is clipped.
fn paint_with<U: GuestUi>(m: &mut Mac<U>, port: u32, region: &Region, mode: u16, pat: &[u8; 8], line: Option<((i32, i32), (i32, i32))>) -> Result<(), Refusal> {
    super::picture::note_drawing(m, port);
    if m.mem.read_i16(port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis"))? < 0 { return Ok(()); }
    let (pt, brush) = (pattern(pat), brush_for(mode));
    target::with(m, port, |m, t| {
        let (mut g, ph) = (region.clone(), t.local0);
        g.translate(-t.local0.0, -t.local0.1);
        let clip = clip_region(m, t)?;
        let c = clip_of(&clip, t);
        let mut touched = Rect::default();
        for r in g.rects() { touched = touched.union(&stencil(&mut t.bm, *r, &brush, &c, |x, y| pt.bit(x + ph.0, y + ph.1))); }
        let mut seen = g.clone();
        seen.and_region(&clip);
        let l0 = t.local0;
        let as_line = line.is_some_and(|(a, b)| seen.rects() == g.rects() && super::plane::record_line(&mut m.qd.plane, t, (a.0 - l0.0, a.1 - l0.1), (b.0 - l0.0, b.1 - l0.1), mode, pat));
        if !as_line { super::plane::record_fill(&mut m.qd.plane, t, &seen, mode, pat); }
        Ok(touched)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op { Frame, Paint, Erase, Invert, Fill }

/// One Frame/Paint/Erase/Invert/Fill of `shape` bounded by `r` (LOCAL coordinates) on the current port.
fn shape_op<U: GuestUi>(m: &mut Mac<U>, shape: Shape, r: Rect, op: Op, fill: Option<u32>, name: &str) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let pen = read_pen(&m.mem, port)?;
    let log = format!("[quickdraw] {name} {}", super::rect_str(&r));
    if op == Op::Frame {
        if let Some(open) = m.qd.rgn_open.get_mut(&port) { open.or_region(&shape.region(r)); return Ok(Done { result: 0, log: format!("{log} (recorded into the open region)") }); }
        if pen.width <= 0 || pen.height <= 0 { return Ok(Done { result: 0, log: format!("{log} (pen size is empty: nothing drawn)") }); }
    }
    if r.is_empty() { return Ok(Done { result: 0, log: format!("{log} (empty)") }); }
    let (region, mode, pat) = match op {
        Op::Frame => (shape.frame(r, i32::from(pen.width), i32::from(pen.height)), pen.mode, pen.pat),
        Op::Paint => (shape.region(r), pen.mode, pen.pat),
        Op::Erase => (shape.region(r), PAT_COPY, read_pat(&m.mem, port.wrapping_add(P_BK_PAT))?),
        Op::Invert => (shape.region(r), 10, BLACK),
        Op::Fill => (shape.region(r), PAT_COPY, read_pat(&m.mem, fill.unwrap_or(0))?),
    };
    paint(m, port, &region, mode, &pat)?;
    Ok(Done { result: 0, log })
}

fn rect_arg<U: GuestUi>(m: &Mac<U>, a: u32) -> Result<Rect, Refusal> { Ok(raster(read_rect(&m.mem, a)?)) }

macro_rules! rect_ops {
    ($($f:ident, $shape:expr, $op:expr, $name:literal;)*) => {$(
        fn $f<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
            let r = rect_arg(m, c.args.get(0))?;
            let fill = if $op == Op::Fill { Some(c.args.get(c.args.len() - 1)) } else { None };
            shape_op(m, $shape, r, $op, fill, $name)
        }
    )*};
}
rect_ops! {
    frame_rect, Shape::Rect, Op::Frame, "FrameRect"; paint_rect, Shape::Rect, Op::Paint, "PaintRect"; erase_rect, Shape::Rect, Op::Erase, "EraseRect";
    invert_rect, Shape::Rect, Op::Invert, "InvertRect"; fill_rect, Shape::Rect, Op::Fill, "FillRect";
    frame_oval, Shape::Oval, Op::Frame, "FrameOval"; paint_oval, Shape::Oval, Op::Paint, "PaintOval"; erase_oval, Shape::Oval, Op::Erase, "EraseOval";
    invert_oval, Shape::Oval, Op::Invert, "InvertOval"; fill_oval, Shape::Oval, Op::Fill, "FillOval";
}

macro_rules! round_ops {
    ($($f:ident, $op:expr, $name:literal;)*) => {$(
        fn $f<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
            let r = rect_arg(m, c.args.get(0))?;
            let shape = Shape::RoundRect { ow: i32::from(c.args.get(1) as i16), oh: i32::from(c.args.get(2) as i16) };
            let fill = if $op == Op::Fill { Some(c.args.get(3)) } else { None };
            shape_op(m, shape, r, $op, fill, $name)
        }
    )*};
}
round_ops! {
    frame_round, Op::Frame, "FrameRoundRect"; paint_round, Op::Paint, "PaintRoundRect"; erase_round, Op::Erase, "EraseRoundRect";
    invert_round, Op::Invert, "InvertRoundRect"; fill_round, Op::Fill, "FillRoundRect";
}

/// Frame/Paint/Erase/Invert/FillRgn: the region's own pixels (a frame is the region minus itself inset by the pen).
fn rgn_op<U: GuestUi>(m: &mut Mac<U>, c: &Call, op: Op, name: &str) -> Result<Done, Refusal> {
    let g = m.read_region(c.args.get(0))?;
    let port = current_port(m)?;
    let pen = read_pen(&m.mem, port)?;
    let (region, mode, pat) = match op {
        Op::Frame => {
            let (pw, ph) = (i32::from(pen.width), i32::from(pen.height));
            // Regions.a FrRgn: a rectangular region is FrRect; any other is DiffRgn(rgn, InsetRgn(copy, pnSize))
            if g.rects().len() <= 1 { (Shape::Rect.frame(g.bounds(), pw, ph), pen.mode, pen.pat) } else { (super::inset::frame_region(&g, pw, ph), pen.mode, pen.pat) }
        }
        Op::Paint => (g, pen.mode, pen.pat),
        Op::Erase => (g, PAT_COPY, read_pat(&m.mem, port.wrapping_add(P_BK_PAT))?),
        Op::Invert => (g, 10, BLACK),
        Op::Fill => (g, PAT_COPY, read_pat(&m.mem, c.args.get(1))?),
    };
    paint(m, port, &region, mode, &pat)?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name}") })
}
fn frame_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { rgn_op(m, c, Op::Frame, "FrameRgn") }
fn paint_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { rgn_op(m, c, Op::Paint, "PaintRgn") }
fn erase_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { rgn_op(m, c, Op::Erase, "EraseRgn") }
fn invert_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { rgn_op(m, c, Op::Invert, "InvertRgn") }
fn fill_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { rgn_op(m, c, Op::Fill, "FillRgn") }

// ---- the pen ----

fn done(log: &str) -> Result<Done, Refusal> { Ok(Done { result: 0, log: format!("[quickdraw] {log}") }) }
fn w16(m: &mut Mac<impl GuestUi>, a: u32, v: i16) -> Result<(), Refusal> { m.mem.write_u16(a, v as u16).map_err(Refusal::mem("GrafPort")) }

fn pen_normal<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    w16(m, p.wrapping_add(P_PN_SIZE), 1)?; w16(m, p.wrapping_add(P_PN_SIZE + 2), 1)?; w16(m, p.wrapping_add(P_PN_MODE), PAT_COPY as i16)?;
    write_pat(&mut m.mem, p.wrapping_add(P_PN_PAT), &BLACK)?;
    done("PenNormal")
}
fn pen_size<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    w16(m, p.wrapping_add(P_PN_SIZE + 2), c.args.get(0) as i16)?; w16(m, p.wrapping_add(P_PN_SIZE), c.args.get(1) as i16)?;
    done(&format!("PenSize {}x{}", c.args.get(0) as i16, c.args.get(1) as i16))
}
fn pen_mode<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    w16(m, p.wrapping_add(P_PN_MODE), c.args.get(0) as i16)?;
    done(&format!("PenMode {}", c.args.get(0) as u16))
}
fn pen_pat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (p, pat) = (current_port(m)?, read_pat(&m.mem, c.args.get(0))?);
    write_pat(&mut m.mem, p.wrapping_add(P_PN_PAT), &pat)?;
    done("PenPat")
}
fn back_pat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (p, pat) = (current_port(m)?, read_pat(&m.mem, c.args.get(0))?);
    write_pat(&mut m.mem, p.wrapping_add(P_BK_PAT), &pat)?;
    done("BackPat")
}
fn hide_show<U: GuestUi>(m: &mut Mac<U>, delta: i16, name: &str) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    let v = m.mem.read_i16(p.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis"))?;
    w16(m, p.wrapping_add(P_PN_VIS), v.wrapping_add(delta))?;
    done(name)
}
fn hide_pen<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> { hide_show(m, -1, "HidePen") }
fn show_pen<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> { hide_show(m, 1, "ShowPen") }

/// PenState: pnLoc (4), pnSize (4), pnMode (2), pnPat (8) = the port's bytes 48..66.
fn get_pen_state<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    let b = m.mem.bytes(m.mem.span(p.wrapping_add(P_PN_LOC), 18).map_err(Refusal::mem("pnState"))?).map_err(Refusal::mem("pnState"))?.to_vec();
    m.mem.write_bytes(c.args.get(0), &b).map_err(Refusal::mem("PenState"))?;
    done("GetPenState")
}
fn set_pen_state<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    let b = m.mem.bytes(m.mem.span(c.args.get(0), 18).map_err(Refusal::mem("PenState"))?).map_err(Refusal::mem("PenState"))?.to_vec();
    m.mem.write_bytes(p.wrapping_add(P_PN_LOC), &b).map_err(Refusal::mem("pnState"))?;
    done("SetPenState")
}
fn get_pen<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let pen = read_pen(&m.mem, current_port(m)?)?;
    w16(m, c.args.get(0), pen.v)?; w16(m, c.args.get(0) + 2, pen.h)?;
    done("GetPen")
}

/// The pixels of the pen dragged from `from` to `to`, the way DrawLine.a (Atkinson's QuickDraw) scan-converts them: the
/// line is cut into one horizontal slab per scanline, top to bottom; the slab's edges are 16.16 fixed-point values that
/// start at the top point + 1/2 (+ slope/2, minus the pen height times the slope, then one more slope step when the line is
/// shallower than 45 degrees, else the other edge loses a whole pixel) and advance by slope = FixRatio(dh, dv) per row.
/// Every slab is clipped to LINERECT, the rectangle bounding the pen's travel (DrawLine.a: MINRECT is drawn into the mask
/// buffer first), so a shallow line's first and last runs never poke past its end points. A horizontal line is that
/// rectangle itself, a vertical one likewise (HVLINE).
fn line_region(from: (i32, i32), to: (i32, i32), pw: i32, ph: i32) -> Region {
    let mut g = Region::new();
    let (p1, p2) = if from.1 <= to.1 { (from, to) } else { (to, from) }; // sort by vertical (VSWAP)
    let (dh, dv) = (i64::from(p2.0 - p1.0), i64::from(p2.1 - p1.1));
    if dv == 0 {
        g.or_rect(Rect::new(from.0.min(to.0), p1.1, from.0.max(to.0) + pw, p1.1 + ph));
        return g;
    }
    if dh == 0 {
        g.or_rect(Rect::new(p1.0, p1.1, p1.0 + pw, p2.1 + ph));
        return g;
    }
    const ONE: i64 = 1 << 16;
    let slope = (dh << 16) / dv; // FixRatio truncates toward zero (executor toolutil.cpp C_FixRatio)
    let adjust = i64::from(ph) * slope; // FixMul(ph << 16, slope), exact for an integer factor
    let mut left = (i64::from(p1.0) << 16) + 0x8000 + (slope >> 1);
    let mut right = left + (i64::from(pw) << 16);
    if slope >= 0 {
        left -= adjust;
        if slope < ONE { left += slope } else { right -= ONE }
    } else {
        right -= adjust;
        if slope >= -ONE { right += slope } else { left += ONE }
    }
    let (bound_l, bound_r) = (p1.0.min(p2.0), p1.0.max(p2.0) + pw);
    for row in 0..i64::from(ph) + dv {
        let (l, r) = (((left >> 16) as i32).max(bound_l), ((right >> 16) as i32).min(bound_r)); // integer words of the edges: floor
        if r > l { g.or_rect(Rect::new(l, p1.1 + row as i32, r, p1.1 + row as i32 + 1)); }
        left += slope;
        right += slope;
    }
    g
}

/// Draw the pen's line from pnLoc to (h, v) and move there.
fn line_to<U: GuestUi>(m: &mut Mac<U>, h: i16, v: i16, name: &str) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    if m.qd.rgn_open.contains_key(&port) { return Err(Refusal(format!("{name} while a region is open: recording polygon outlines into a region is not implemented"))); }
    let pen = read_pen(&m.mem, port)?;
    if pen.width > 0 && pen.height > 0 {
        let g = line_region((i32::from(pen.h), i32::from(pen.v)), (i32::from(h), i32::from(v)), i32::from(pen.width), i32::from(pen.height));
        let line = (pen.width == 1 && pen.height == 1).then_some(((i32::from(pen.h), i32::from(pen.v)), (i32::from(h), i32::from(v))));
        paint_with(m, port, &g, pen.mode, &pen.pat, line)?;
    }
    set_pen_loc(&mut m.mem, port, v, h)?;
    done(&format!("{name} ({},{}) -> ({v},{h})", pen.v, pen.h))
}
fn move_to<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    set_pen_loc(&mut m.mem, p, c.args.get(1) as i16, c.args.get(0) as i16)?;
    done(&format!("MoveTo {},{}", c.args.get(0) as i16, c.args.get(1) as i16))
}
fn move_by<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    let pen = read_pen(&m.mem, p)?;
    set_pen_loc(&mut m.mem, p, pen.v.wrapping_add(c.args.get(1) as i16), pen.h.wrapping_add(c.args.get(0) as i16))?;
    done("Move")
}
fn line_to_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { line_to(m, c.args.get(0) as i16, c.args.get(1) as i16, "LineTo") }
fn line_by<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let pen = read_pen(&m.mem, current_port(m)?)?;
    line_to(m, pen.h.wrapping_add(c.args.get(0) as i16), pen.v.wrapping_add(c.args.get(1) as i16), "Line")
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use super::reg;
    const R: &[Arg] = &[Arg::L];
    const RF: &[Arg] = &[Arg::L, Arg::L];
    const RR: &[Arg] = &[Arg::L, Arg::W, Arg::W];
    const RRF: &[Arg] = &[Arg::L, Arg::W, Arg::W, Arg::L];
    reg(m, 0xA8A1, "FrameRect", R, None, frame_rect::<U>)?; reg(m, 0xA8A2, "PaintRect", R, None, paint_rect::<U>)?;
    reg(m, 0xA8A3, "EraseRect", R, None, erase_rect::<U>)?; reg(m, 0xA8A4, "InverRect", R, None, invert_rect::<U>)?;
    reg(m, 0xA8A5, "FillRect", RF, None, fill_rect::<U>)?;
    reg(m, 0xA8B7, "FrameOval", R, None, frame_oval::<U>)?; reg(m, 0xA8B8, "PaintOval", R, None, paint_oval::<U>)?;
    reg(m, 0xA8B9, "EraseOval", R, None, erase_oval::<U>)?; reg(m, 0xA8BA, "InvertOval", R, None, invert_oval::<U>)?;
    reg(m, 0xA8BB, "FillOval", RF, None, fill_oval::<U>)?;
    reg(m, 0xA8B0, "FrameRoundRect", RR, None, frame_round::<U>)?; reg(m, 0xA8B1, "PaintRoundRect", RR, None, paint_round::<U>)?;
    reg(m, 0xA8B2, "EraseRoundRect", RR, None, erase_round::<U>)?; reg(m, 0xA8B3, "InverRoundRect", RR, None, invert_round::<U>)?;
    reg(m, 0xA8B4, "FillRoundRect", RRF, None, fill_round::<U>)?;
    reg(m, 0xA8D2, "FrameRgn", R, None, frame_rgn::<U>)?; reg(m, 0xA8D3, "PaintRgn", R, None, paint_rgn::<U>)?;
    reg(m, 0xA8D4, "EraseRgn", R, None, erase_rgn::<U>)?; reg(m, 0xA8D5, "InverRgn", R, None, invert_rgn::<U>)?;
    reg(m, 0xA8D6, "FillRgn", RF, None, fill_rgn::<U>)?;
    reg(m, 0xA89E, "PenNormal", &[], None, pen_normal::<U>)?;
    reg(m, 0xA89B, "PenSize", &[Arg::W, Arg::W], None, pen_size::<U>)?;
    reg(m, 0xA89C, "PenMode", &[Arg::W], None, pen_mode::<U>)?;
    reg(m, 0xA89D, "PenPat", R, None, pen_pat::<U>)?;
    reg(m, 0xA87C, "BackPat", R, None, back_pat::<U>)?;
    reg(m, 0xA896, "HidePen", &[], None, hide_pen::<U>)?;
    reg(m, 0xA897, "ShowPen", &[], None, show_pen::<U>)?;
    reg(m, 0xA898, "GetPenState", R, None, get_pen_state::<U>)?;
    reg(m, 0xA899, "SetPenState", R, None, set_pen_state::<U>)?;
    reg(m, 0xA89A, "GetPen", R, None, get_pen::<U>)?;
    reg(m, 0xA893, "MoveTo", &[Arg::W, Arg::W], None, move_to::<U>)?;
    reg(m, 0xA894, "Move", &[Arg::W, Arg::W], None, move_by::<U>)?;
    reg(m, 0xA891, "LineTo", &[Arg::W, Arg::W], None, line_to_trap::<U>)?;
    reg(m, 0xA892, "Line", &[Arg::W, Arg::W], None, line_by::<U>)
}


#[cfg(test)]
mod tests {
    use super::*;
    fn pixels(g: &Region, (x0, y0, x1, y1): (i32, i32, i32, i32)) -> Vec<(i32, i32)> {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| g.contains(x, y)).collect()
    }
    /// DrawLine.a: `CMP.L #$00010000,D7 / BGE LESSV1` sends slope == 1.0 to the branch that trims the right edge, which makes a
    /// 45-degree 1x1 line hit (x+i, y+i) from its first point to its last. (systemless, run as the oracle, puts it one pixel
    /// right of that: it treats slope == 1.0 like slope < 1.0.)
    #[test]
    fn a_45_degree_line_hits_both_end_points_and_one_pixel_per_row() {
        for (from, to) in [((4, 4), (36, 36)), ((36, 4), (4, 36)), ((10, 10), (11, 11))] {
            let g = line_region(from, to, 1, 1);
            let n = (to.0 - from.0).abs();
            let (top, bot) = (if from.1 < to.1 { from } else { to }, if from.1 < to.1 { to } else { from });
            let step = (bot.0 - top.0).signum();
            let want: Vec<(i32, i32)> = (0..=n).map(|i| (top.0 + step * i, top.1 + i)).collect();
            let mut got = pixels(&g, (0, 0, 50, 50));
            got.sort_by_key(|&(x, y)| (y, x));
            assert_eq!(got, want, "{from:?} -> {to:?}");
        }
    }
    /// DrawLine.a: every slab is clipped to LINERECT (MINRECT goes into the mask buffer first), so no pixel lies outside the
    /// rectangle bounding the pen's travel, however shallow the slope (the unclipped first run of a slope-1.27 line starts a pixel early).
    #[test]
    fn slabs_never_leave_the_rectangle_bounding_the_pens_travel() {
        for (from, to, pw, ph) in [((8, 14), (36, 36), 1, 1), ((3, 4), (37, 12), 1, 1), ((4, 4), (35, 16), 3, 2), ((20, 3), (17, 37), 2, 3)] {
            let g = line_region(from, to, pw, ph);
            let b = g.bounds();
            assert_eq!(b, Rect::new(from.0.min(to.0), from.1.min(to.1), from.0.max(to.0) + pw, from.1.max(to.1) + ph), "{from:?} {to:?} pen {pw}x{ph}");
        }
        // negative control: the end points really are inside, so the bound is tight, not vacuous
        assert!(line_region((8, 14), (36, 36), 1, 1).contains(8, 14) && line_region((8, 14), (36, 36), 1, 1).contains(36, 36));
    }
    /// DrawLine.a HVLINE: a horizontal or vertical line is the rectangle the pen sweeps, whichever way it is drawn.
    #[test]
    fn horizontal_and_vertical_lines_are_the_swept_rectangle_in_either_direction() {
        assert_eq!(line_region((4, 10), (36, 10), 4, 1), line_region((36, 10), (4, 10), 4, 1));
        assert_eq!(line_region((4, 10), (36, 10), 4, 1).bounds(), Rect::new(4, 10, 40, 11));
        assert_eq!(line_region((7, 3), (7, 30), 2, 3), line_region((7, 30), (7, 3), 2, 3));
        assert_eq!(line_region((7, 3), (7, 30), 2, 3).bounds(), Rect::new(7, 3, 9, 33));
        assert_ne!(line_region((4, 10), (36, 10), 4, 1), line_region((4, 10), (36, 12), 4, 1), "negative control: a slanted line differs");
    }
}
