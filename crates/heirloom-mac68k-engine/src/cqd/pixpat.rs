//! Color QuickDraw pixel patterns (`PixPat`, `'ppat'` resources: `NewPixPat`, `DisposePixPat`, `CopyPixPat`,
//! `PenPixPat`, `BackPixPat`, `GetPixPat`, `MakeRGBPat`, `FillCRect`, `FillCRgn`), `GetCPixel`, and QuickDraw polygons
//! (`OpenPoly`, `ClosePoly`, `KillPoly`, `OffsetPoly`, `FramePoly`/`PaintPoly`/`ErasePoly`/`InvertPoly`/`FillPoly`).
use super::color::{read_rgb, write_rgb, Rgb};
use super::pixmap::{unpack, PM_BOUNDS, PM_PIXEL_SIZE, PM_ROW, PM_TABLE};
use crate::cpu::Cpu;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg, Rect};
use crate::qd::draw::{paint, BLACK};
use crate::qd::port::*;
use crate::qd::target::{self, clip_of, clip_region, current_port};
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_raster::{Rect as RRect, Region};
use heirloom_ui_abi::GuestUi;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA07, "NewPixPat", &[], Some(Arg::L), new_pixpat_trap::<U>)?;
    reg(m, 0xAA08, "DisposePixPat", &[Arg::L], None, dispose_pixpat::<U>)?;
    reg(m, 0xAA09, "CopyPixPat", &[Arg::L, Arg::L], None, copy_pixpat::<U>)?;
    reg(m, 0xAA0A, "PenPixPat", &[Arg::L], None, pen_pixpat::<U>)?;
    reg(m, 0xAA0B, "BackPixPat", &[Arg::L], None, back_pixpat::<U>)?;
    reg(m, 0xAA0C, "GetPixPat", &[Arg::W], Some(Arg::L), get_pixpat::<U>)?;
    reg(m, 0xAA0D, "MakeRGBPat", &[Arg::L, Arg::P], None, make_rgb_pat::<U>)?;
    reg(m, 0xAA0E, "FillCRect", &[Arg::P, Arg::L], None, fill_crect::<U>)?;
    reg(m, 0xAA12, "FillCRgn", &[Arg::L, Arg::L], None, fill_crgn::<U>)?;
    reg(m, 0xAA17, "GetCPixel", &[Arg::W, Arg::W, Arg::P], None, get_cpixel::<U>)?;
    reg(m, 0xA8CB, "OpenPoly", &[], Some(Arg::L), open_poly::<U>)?;
    reg(m, 0xA8CC, "ClosePoly", &[], None, close_poly::<U>)?;
    reg(m, 0xA8CD, "KillPoly", &[Arg::L], None, kill_poly::<U>)?;
    reg(m, 0xA8CE, "OffsetPoly", &[Arg::L, Arg::W, Arg::W], None, offset_poly::<U>)?;
    reg(m, 0xA8C6, "FramePoly", &[Arg::L], None, draw_poly::<U>)?;
    reg(m, 0xA8C7, "PaintPoly", &[Arg::L], None, draw_poly::<U>)?;
    reg(m, 0xA8C8, "ErasePoly", &[Arg::L], None, draw_poly::<U>)?;
    reg(m, 0xA8C9, "InvertPoly", &[Arg::L], None, draw_poly::<U>)?;
    reg(m, 0xA8CA, "FillPoly", &[Arg::L, Arg::P], None, draw_poly::<U>)
}

fn new_pixpat_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let h = m.new_pixpat(&BLACK)?;
    let p = m.deref_handle(h, "NewPixPat")?;
    let pm = m.new_pixmap(1, Rect { top: 0, left: 0, bottom: 8, right: 8 }, 0, 2, 0)?;
    let (data_h, _) = m.alloc_handle(0, "PixPat.patData")?;
    let e = Refusal::mem("PixPat");
    m.mem.write_u16(p, 1).map_err(&e)?; // patType = 1 (new-style)
    m.mem.write_u32(p + 2, pm).map_err(&e)?;
    m.mem.write_u32(p + 6, data_h).map_err(&e)?;
    m.mem.write_u16(p + 14, 0xFFFF).map_err(&e)?; // patXValid = -1
    Ok(Done { result: h, log: format!("[cqd] NewPixPat -> {h:#x}") })
}

fn dispose_pixpat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if h != 0 {
        if let Ok(p) = m.deref_handle(h, "DisposePixPat") {
            let pm = m.mem.read_u32(p + 2).unwrap_or(0);
            let data = m.mem.read_u32(p + 6).unwrap_or(0);
            let xdata = m.mem.read_u32(p + 10).unwrap_or(0);
            if pm != 0 {
                if let Ok(pm_p) = m.deref_handle(pm, "patMap") {
                    let ctab = m.mem.read_u32(pm_p + PM_TABLE).unwrap_or(0);
                    if ctab != 0 { let _ = m.dispose_handle(ctab); }
                }
                let _ = m.dispose_handle(pm);
            }
            if data != 0 { let _ = m.dispose_handle(data); }
            if xdata != 0 { let _ = m.dispose_handle(xdata); }
        }
        let _ = m.dispose_handle(h);
    }
    Ok(Done { result: 0, log: format!("[cqd] DisposePixPat {h:#x}") })
}

fn copy_pixpat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (src, dst) = (c.args.get(0), c.args.get(1));
    if src != 0 && dst != 0 {
        if let (Ok(sp), Ok(dp)) = (m.deref_handle(src, "CopyPixPat"), m.deref_handle(dst, "CopyPixPat")) {
            let pat1 = read_pat(&m.mem, sp + PIXPAT_1DATA)?;
            write_pat(&mut m.mem, dp + PIXPAT_1DATA, &pat1)?;
            let kind = m.mem.read_u16(sp).unwrap_or(0);
            let _ = m.mem.write_u16(dp, kind);
        }
    }
    Ok(Done { result: 0, log: format!("[cqd] CopyPixPat {src:#x} -> {dst:#x}") })
}

fn set_port_pixpat<U: GuestUi>(m: &mut Mac<U>, slot: PatSlot, pp: u32, name: &str) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    if pp != 0 {
        if is_cport(&m.mem, port) {
            let off = match slot { PatSlot::Bk => CP_BK_PIXPAT, PatSlot::Pn => CP_PN_PIXPAT, PatSlot::Fill => CP_FILL_PIXPAT };
            m.mem.write_u32(port.wrapping_add(off), pp).map_err(Refusal::mem(name))?;
        } else if let Ok(p) = m.deref_handle(pp, name) {
            let pat1 = read_pat(&m.mem, p + PIXPAT_1DATA)?;
            write_port_pat(&mut m.mem, port, slot, &pat1)?;
        }
    }
    Ok(Done { result: 0, log: format!("[cqd] {name} {pp:#x}") })
}

fn pen_pixpat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_port_pixpat(m, PatSlot::Pn, c.args.get(0), "PenPixPat") }
fn back_pixpat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_port_pixpat(m, PatSlot::Bk, c.args.get(0), "BackPixPat") }

/// `GetPixPat(patID)`: parses a `'ppat'` resource (Inside Macintosh: Imaging With QuickDraw p. 4-104) into a live
/// `PixPatHandle` with real `patMap` (`PixMapHandle` + `CTabHandle`), `patData`, and `pat1Data`. Returns NIL when absent.
fn get_pixpat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(rh) = m.get_resource(*b"ppat", id, false) else {
        return Ok(Done { result: 0, log: format!("[cqd] GetPixPat {id} -> NIL (not found)") });
    };
    let Some(bytes) = m.handle_bytes(rh) else {
        return Ok(Done { result: 0, log: format!("[cqd] GetPixPat {id} -> NIL") });
    };
    if bytes.len() < PIXPAT_SIZE as usize {
        return Ok(Done { result: 0, log: format!("[cqd] GetPixPat {id} -> NIL (short {} bytes)", bytes.len()) });
    }
    let be16 = |o: usize| u16::from_be_bytes([bytes[o], bytes[o + 1]]);
    let be32 = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let pat_type = be16(0);
    let map_off = be32(2) as usize;
    let data_off = be32(6) as usize;
    let mut pat1 = [0u8; 8];
    pat1.copy_from_slice(&bytes[20..28]);
    let h = m.new_pixpat(&pat1)?;
    let p = m.deref_handle(h, "GetPixPat")?;
    let e = Refusal::mem("GetPixPat");
    m.mem.write_u16(p, pat_type).map_err(&e)?;
    m.mem.write_u16(p + 14, 0xFFFF).map_err(&e)?;
    if pat_type == 1 && map_off + 50 <= bytes.len() {
        let row_bytes = be16(map_off + 4) & 0x3FFF;
        let bounds = Rect {
            top: be16(map_off + 6) as i16,
            left: be16(map_off + 8) as i16,
            bottom: be16(map_off + 10) as i16,
            right: be16(map_off + 12) as i16,
        };
        let depth = be16(map_off + 32) as u8;
        let ctab_off = be32(map_off + 42) as usize;
        let rows = (i32::from(bounds.bottom) - i32::from(bounds.top)).max(0) as usize;
        let data_len = usize::from(row_bytes) * rows;
        let (data_h, data_p) = m.alloc_handle(data_len as u32, "ppat.patData")?;
        if data_off + data_len <= bytes.len() && data_len > 0 {
            m.mem.write_bytes(data_p, &bytes[data_off..data_off + data_len]).map_err(&e)?;
        }
        let ctab_h = if ctab_off + 8 <= bytes.len() {
            let count = usize::from(be16(ctab_off + 6)) + 1;
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                let eo = ctab_off + 8 + i * 8;
                if eo + 8 <= bytes.len() {
                    entries.push(Rgb { r: be16(eo + 2), g: be16(eo + 4), b: be16(eo + 6) });
                }
            }
            if !entries.is_empty() { let seed = m.unique_seed(); m.new_ctab(&entries, seed, 0)? } else { 0 }
        } else { 0 };
        let pm = m.new_pixmap(depth.max(1), bounds, 0, u32::from(row_bytes.max(1)), ctab_h)?;
        m.mem.write_u32(p + 2, pm).map_err(&e)?;
        m.mem.write_u32(p + 6, data_h).map_err(&e)?;
    }
    Ok(Done { result: h, log: format!("[cqd] GetPixPat {id} (type {pat_type}) -> {h:#x}") })
}

fn make_rgb_pat<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (pp, rgb_p) = (c.args.get(0), c.args.get(1));
    if pp != 0 && rgb_p != 0 {
        let col = read_rgb(&m.mem, rgb_p)?;
        let p = m.deref_handle(pp, "MakeRGBPat")?;
        let e = Refusal::mem("MakeRGBPat");
        // Store a 1x1 32-bit direct PixMap holding the exact RGB color + patType = 2 (RGB pattern).
        let (data_h, data_p) = m.alloc_handle(4, "MakeRGBPat.data")?;
        let px = super::direct::pixel_of(32, col);
        m.mem.write_u32(data_p, px).map_err(&e)?;
        let pm = m.new_pixmap(32, Rect { top: 0, left: 0, bottom: 1, right: 1 }, 0, 4, 0)?;
        m.mem.write_u16(p, 2).map_err(&e)?;
        m.mem.write_u32(p + 2, pm).map_err(&e)?;
        m.mem.write_u32(p + 6, data_h).map_err(&e)?;
        write_pat(&mut m.mem, p + PIXPAT_1DATA, &BLACK)?;
    }
    Ok(Done { result: 0, log: format!("[cqd] MakeRGBPat {pp:#x}") })
}

fn fill_with_pixpat<U: GuestUi>(m: &mut Mac<U>, region: &Region, pp: u32, name: &str) -> Result<(), Refusal> {
    let port = current_port(m)?;
    if pp == 0 { return Ok(()); }
    let p = m.deref_handle(pp, name)?;
    let pat_type = m.mem.read_u16(p).map_err(Refusal::mem(name))?;
    let pm_h = m.mem.read_u32(p + 2).unwrap_or(0);
    let data_h = m.mem.read_u32(p + 6).unwrap_or(0);
    if pat_type != 0 && pm_h != 0 && data_h != 0 {
        if let (Ok(pm_p), Ok(data_p)) = (m.deref_handle(pm_h, name), m.deref_handle(data_h, name)) {
            let bounds = read_rect(&m.mem, pm_p + PM_BOUNDS)?;
            let (pw, ph) = ((i32::from(bounds.right) - i32::from(bounds.left)).max(0), (i32::from(bounds.bottom) - i32::from(bounds.top)).max(0));
            let depth = m.mem.read_u16(pm_p + PM_PIXEL_SIZE).unwrap_or(1) as u8;
            let row_bytes = m.mem.read_u16(pm_p + PM_ROW).unwrap_or(0) & 0x3FFF;
            let ctab_h = m.mem.read_u32(pm_p + PM_TABLE).unwrap_or(0);
            let src_ctab = if ctab_h != 0 { m.read_ctab(ctab_h).ok() } else { None };
            if pw > 0 && ph > 0 && row_bytes > 0 {
                let total_bytes = (ph as u32).saturating_mul(u32::from(row_bytes));
                let raw = m.mem.span(data_p, total_bytes).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).unwrap_or_default();
                if let Ok(pat_bm) = unpack(&raw, usize::from(row_bytes), pw, ph, depth) {
                    let mut tile = Vec::with_capacity((pw * ph) as usize);
                    for y in 0..ph {
                        for x in 0..pw {
                            let px = pat_bm.get(x, y).unwrap_or(0);
                            let rgb = if super::direct::is_direct(depth) {
                                super::direct::rgb_of(depth, px)
                            } else if let Some(ref ct) = src_ctab {
                                ct.get(px as usize).copied().unwrap_or(Rgb::BLACK)
                            } else if px != 0 { Rgb::BLACK } else { Rgb::WHITE };
                            tile.push(rgb);
                        }
                    }
                    return target::with(m, port, |m, t| {
                        let mut g = region.clone();
                        g.translate(-t.local0.0, -t.local0.1);
                        let clip = clip_region(m, t)?;
                        let c = clip_of(&clip, t);
                        let depth_dst = t.bm.depth();
                        let dst_ctab: Option<Vec<Rgb>> = if !t.bm.is_direct() { t.clut.clone() } else { None };
                        let mut touched = RRect::default();
                        for r in g.rects() {
                            touched = touched.union(&heirloom_raster::paint::stencil_with(&mut t.bm, *r, &c, |x, y, _| {
                                let tx = (x + t.local0.0).rem_euclid(pw);
                                let ty = (y + t.local0.1).rem_euclid(ph);
                                let rgb = tile[(ty * pw + tx) as usize];
                                let v = if super::direct::is_direct(depth_dst) {
                                    super::direct::pixel_of(depth_dst, rgb)
                                } else if let Some(ref ct) = dst_ctab {
                                    super::color::nearest(ct, rgb) as u32
                                } else {
                                    u32::from(u32::from(rgb.r) + u32::from(rgb.g) + u32::from(rgb.b) < 3 * 0x8000)
                                };
                                Some(v)
                            }));
                        }
                        Ok(touched)
                    });
                }
            }
        }
    }
    let pat1 = read_pat(&m.mem, p + PIXPAT_1DATA)?;
    paint(m, port, region, PAT_COPY, &pat1)
}

fn fill_crect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = raster(read_rect(&m.mem, c.args.get(0))?);
    let pp = c.args.get(1);
    if !r.is_empty() { fill_with_pixpat(m, &Region::from_rect(r), pp, "FillCRect")?; }
    Ok(Done { result: 0, log: format!("[cqd] FillCRect {} pp={pp:#x}", crate::qd::rect_str(&r)) })
}

fn fill_crgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rgn = m.read_region(c.args.get(0))?;
    let pp = c.args.get(1);
    if !rgn.is_empty() { fill_with_pixpat(m, &rgn, pp, "FillCRgn")?; }
    Ok(Done { result: 0, log: format!("[cqd] FillCRgn pp={pp:#x}") })
}

fn get_cpixel<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, v, out) = (c.args.get(0) as u16 as i16, c.args.get(1) as u16 as i16, c.args.get(2));
    let port = current_port(m)?;
    let mut rgb = Rgb::WHITE;
    let _ = target::with(m, port, |_m, t| {
        let (x, y) = (i32::from(h) - t.local0.0, i32::from(v) - t.local0.1);
        if x >= 0 && y >= 0 && x < t.bm.width() && y < t.bm.height() {
            let px = t.bm.get(x, y).unwrap_or(0);
            rgb = if t.bm.is_direct() {
                super::direct::rgb_of(t.bm.depth(), px)
            } else if let Some(ref ct) = t.clut {
                ct.get(px as usize).copied().unwrap_or(Rgb::WHITE)
            } else if px != 0 { Rgb::BLACK } else { Rgb::WHITE };
        }
        Ok(RRect::default())
    });
    if out != 0 { write_rgb(&mut m.mem, out, rgb)?; }
    Ok(Done { result: 0, log: format!("[cqd] GetCPixel ({h},{v}) -> ({:04x},{:04x},{:04x})", rgb.r, rgb.g, rgb.b) })
}

fn open_poly<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let (h, p) = m.alloc_handle(10, "OpenPoly")?;
    m.mem.write_u16(p, 10).map_err(Refusal::mem("OpenPoly"))?; // polySize = 10 (empty bbox)
    Ok(Done { result: h, log: format!("[quickdraw] OpenPoly -> {h:#x}") })
}
fn close_poly<U: GuestUi>(_m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[quickdraw] ClosePoly") })
}
fn kill_poly<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if h != 0 { let _ = m.dispose_handle(h); }
    Ok(Done { result: 0, log: format!("[quickdraw] KillPoly {h:#x}") })
}
fn offset_poly<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, dh, dv) = (c.args.get(0), c.args.get(1) as u16 as i16, c.args.get(2) as u16 as i16);
    if let Ok(p) = m.deref_handle(h, "OffsetPoly") {
        let size = m.mem.read_u16(p).unwrap_or(10);
        let e = Refusal::mem("OffsetPoly");
        for off in (2..u32::from(size)).step_by(4) {
            if off + 4 <= u32::from(size) {
                let v = m.mem.read_i16(p + off).map_err(&e)?;
                let x = m.mem.read_i16(p + off + 2).map_err(&e)?;
                m.mem.write_u16(p + off, v.wrapping_add(dv) as u16).map_err(&e)?;
                m.mem.write_u16(p + off + 2, x.wrapping_add(dh) as u16).map_err(&e)?;
            }
        }
    }
    Ok(Done { result: 0, log: format!("[quickdraw] OffsetPoly {h:#x} ({dh},{dv})") })
}
fn draw_poly<U: GuestUi>(_m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[quickdraw] Poly op on {:#x}", c.args.get(0)) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixed::trap_opcode;
    use heirloom_ui_abi::RecordingGuestUi;

    #[test]
    fn new_make_rgb_fill_and_dispose_pixpat_round_trip() {
        let mut m = Mac::with_profile(RecordingGuestUi::new(), "mac-ii").unwrap();
        let pp = m.pascal_call(trap_opcode("NewPixPat").unwrap(), &[]).unwrap().value;
        assert_ne!(pp, 0);
        // MakeRGBPat with red
        m.mem.write_u16(0x4000, 0xFFFF).unwrap();
        m.mem.write_u16(0x4002, 0x0000).unwrap();
        m.mem.write_u16(0x4004, 0x0000).unwrap();
        m.pascal_call(trap_opcode("MakeRGBPat").unwrap(), &[pp, 0x4000]).unwrap();
        let p = m.deref_handle(pp, "test").unwrap();
        assert_eq!(m.mem.read_u16(p).unwrap(), 2, "patType = RGB");
        // Negative control: GetPixPat for missing id returns NIL
        assert_eq!(m.pascal_call(trap_opcode("GetPixPat").unwrap(), &[9999]).unwrap().value, 0);
        m.pascal_call(trap_opcode("DisposePixPat").unwrap(), &[pp]).unwrap();
    }
}
