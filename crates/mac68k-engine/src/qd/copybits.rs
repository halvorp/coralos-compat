//! `CopyBits(srcBits, dstBits, srcRect, dstRect, mode, maskRgn)` between any two BitMaps: a window's `portBits` (its
//! canvas), or a real 1-bit BitMap in guest memory (an offscreen port). One `retro_raster::blit` with the transfer mode
//! as a minterm; `maskRgn` (destination LOCAL coordinates) leaves the destination alone outside it. The destination
//! is clipped by `thePort`'s clipRgn and visRgn ONLY when it is `thePort`'s `portBits` (same baseAddr and
//! bounds.topLeft, QuickDraw/Bitmaps.a TOPORT), and not at all when `thePort`'s pen is hidden (StdBits: `pnVis < 0`);
//! any other destination is drawn with a wide-open clip (NOTPORT).
//!
//! Refused by name: scaling (srcRect and dstRect of different size), pattern modes 8..15, color PixMaps.
use super::port::{raster, BM_BASE, BM_BOUNDS, P_BITS, P_PN_VIS};
use super::target::{clip_of, clip_region_via, current_port, finish, open_bits, Target};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::blit::{blit, blit_within};
use retro_raster::Rect;

/// srcCopy..notSrcBic as a minterm over (A = mask, B = source, C = destination), with the mask-off terms keeping C.
pub fn minterm(mode: u16) -> u8 {
    let base = match mode & 7 { 0 => 0xC0, 1 => 0xE0, 2 => 0x60, 3 => 0x20, 4 => 0x30, 5 => 0xB0, 6 => 0x90, _ => 0x80 };
    base | 0x0A
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA8EC, "CopyBits", &[Arg::L, Arg::L, Arg::L, Arg::L, Arg::W, Arg::L], None, copy_bits::<U>)
}

/// `thePort` when `db` is its `portBits` (the struct itself, or a copy with the same baseAddr and bounds.topLeft).
fn port_for_bits<U: GuestUi>(m: &Mac<U>, db: u32) -> Result<u32, Refusal> {
    let Ok(p) = current_port(m) else { return Ok(0) };
    let pb = p.wrapping_add(P_BITS);
    if pb == db { return Ok(p); }
    let e = Refusal::mem("BitMap");
    let key = |bm: u32| -> Result<(u32, u32), Refusal> {
        Ok((m.mem.read_u32(bm.wrapping_add(BM_BASE)).map_err(&e)?, m.mem.read_u32(bm.wrapping_add(BM_BOUNDS)).map_err(&e)?)) // baseAddr, (top, left)
    };
    Ok(match key(pb) { Ok(k) if k == key(db)? => p, _ => 0 })
}

fn blit_them<U: GuestUi>(m: &Mac<U>, clip_port: u32, dt: &mut Target, st: Option<&mut Target>, sr: Rect, dr: Rect, mt: u8, mask: Option<&retro_raster::Region>) -> Result<Rect, Refusal> {
    let clip_region = clip_region_via(m, clip_port, dt)?;
    let c = clip_of(&clip_region, dt);
    let (l0, d) = (dt.local0, dt.to_pixels(dr));
    let maskf = |x: i32, y: i32| mask.map_or(true, |g| g.contains(x + l0.0, y + l0.1));
    let mf: retro_raster::blit::MaskFn = if mask.is_some() { Some(&maskf) } else { None };
    Ok(match st {
        Some(s) => { let sp = s.to_pixels(sr); blit(&mut dt.bm, d.x0, d.y0, &s.bm, sp.x0, sp.y0, sp.width(), sp.height(), mt, 0xFF, mf, &c) }
        None => { let sp = dt.to_pixels(sr); blit_within(&mut dt.bm, d.x0, d.y0, sp.x0, sp.y0, sp.width(), sp.height(), mt, 0xFF, mf, &c) }
    })
}

fn copy_bits<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (sb, db, mode) = (a.get(0), a.get(1), a.get(4) as u16);
    let (sr, dr) = (raster(read_rect(&m.mem, a.get(2))?), raster(read_rect(&m.mem, a.get(3))?));
    if (sr.width(), sr.height()) != (dr.width(), dr.height()) {
        return Err(Refusal(format!("CopyBits: scaling is not implemented (srcRect is {}x{}, dstRect {}x{})", sr.width(), sr.height(), dr.width(), dr.height())));
    }
    let mode_bits = mode & !64; // 64 = ditherCopy: no meaning on a 1-bit destination
    if mode_bits > 7 { return Err(Refusal(format!("CopyBits: transfer mode {mode} is not implemented (source modes 0..7 only; pattern modes 8..15 need a pattern source)"))); }
    let mask = if a.get(5) != 0 { Some(m.read_region(a.get(5))?) } else { None };
    let mut dt = open_bits(m, db)?;
    let clip_port = match port_for_bits(m, db) { Ok(p) => p, Err(e) => { finish(m, dt, Rect::default())?; return Err(e); } };
    let hidden = clip_port != 0 && m.mem.read_i16(clip_port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis")).map_or(false, |v| v < 0);
    let mut st = if sb == db { None } else {
        match open_bits(m, sb) { Ok(t) => Some(t), Err(e) => { finish(m, dt, Rect::default())?; return Err(e); } }
    };
    let touched = if hidden { Ok(Rect::default()) } else { blit_them(m, clip_port, &mut dt, st.as_mut(), sr, dr, minterm(mode_bits), mask.as_ref()) };
    if let Some(s) = st { finish(m, s, Rect::default())?; }
    finish(m, dt, *touched.as_ref().unwrap_or(&Rect::default()))?;
    touched?;
    Ok(Done { result: 0, log: format!("[quickdraw] CopyBits mode {mode} {}x{} {sb:#x} -> {db:#x}{}", dr.width(), dr.height(), if mask.is_some() { " (masked)" } else { "" }) })
}

