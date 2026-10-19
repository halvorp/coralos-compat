//! `CopyBits(srcBits, dstBits, srcRect, dstRect, mode, maskRgn)` between any two BitMaps: a window's `portBits` (its
//! canvas), or a real 1-bit BitMap in guest memory (an offscreen port). One `retro_raster::blit` with the transfer mode
//! as a minterm; `maskRgn` (destination LOCAL coordinates) leaves the destination alone outside it. The destination
//! is clipped by `thePort`'s clipRgn and visRgn ONLY when it is `thePort`'s `portBits` (same baseAddr and
//! bounds.topLeft, QuickDraw/Bitmaps.a TOPORT), and not at all when `thePort`'s pen is hidden (StdBits: `pnVis < 0`);
//! any other destination is drawn with a wide-open clip (NOTPORT).
//!
//! Scaling (srcRect and dstRect of different size) is StretchBits ([`super::stretch`]), one destination row at a time.
//!
//! **Depths.** Source and destination may be 1-bit BitMaps or indexed PixMaps (2, 4, 8 bits; a colour window's pixels). The
//! source is first expressed in the DESTINATION's pen numbers ([`convert_source`]): a 1-bit source into a colour destination
//! takes the current port's foreground (black pixels) and background (white pixels) colours, as Imaging With QuickDraw
//! describes; an indexed source into a different colour table is mapped through the destination's table (an exact entry, else
//! the nearest); an indexed source into a 1-bit destination becomes black where its luminance is below half (no dithering,
//! `ditherCopy` or not). The Boolean transfer modes then act on the destination's pixel values, one implementation for all.
//! Refused by name: pattern modes 8..15 and the arithmetic modes 32..39, direct (16/32-bit) pixel maps.
use super::port::{bits_addr, raster, BM_BASE, BM_BOUNDS, P_PN_VIS};
use super::target::{clip_of, clip_region_via, current_port, finish, open_bits, pens, Target};
use crate::cqd::color::{self, Rgb};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::blit::{blit, blit_within};
use super::stretch::stretch;
use retro_raster::{Bitmap, Rect};

/// srcCopy..notSrcBic as a minterm over (A = mask, B = source, C = destination), with the mask-off terms keeping C.
pub fn minterm(mode: u16) -> u8 {
    let base = match mode & 7 { 0 => 0xC0, 1 => 0xE0, 2 => 0x60, 3 => 0x20, 4 => 0x30, 5 => 0xB0, 6 => 0x90, _ => 0x80 };
    base | 0x0A
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA8EC, "CopyBits", &[Arg::L, Arg::L, Arg::L, Arg::L, Arg::W, Arg::L], None, copy_bits::<U>)?;
    super::reg(m, 0xA817, "CopyMask", &[Arg::L, Arg::L, Arg::L, Arg::L, Arg::L, Arg::L], None, copy_mask::<U>)
}

/// `thePort` when `db` is its `portBits` (the struct itself, or a copy with the same baseAddr and bounds.topLeft).
fn port_for_bits<U: GuestUi>(m: &Mac<U>, db: u32) -> Result<u32, Refusal> {
    let Ok(p) = current_port(m) else { return Ok(0) };
    let pb = bits_addr(&m.mem, p)?;
    if pb == db { return Ok(p); }
    let e = Refusal::mem("BitMap");
    let key = |bm: u32| -> Result<(u32, u32), Refusal> {
        Ok((m.mem.read_u32(bm.wrapping_add(BM_BASE)).map_err(&e)?, m.mem.read_u32(bm.wrapping_add(BM_BOUNDS)).map_err(&e)?)) // baseAddr, (top, left)
    };
    Ok(match key(pb) { Ok(k) if k == key(db)? => p, _ => 0 })
}

/// A copy's source: a bitmap and the local coordinate of its pixel (0,0).
struct Src<'a> { bm: &'a Bitmap, local0: (i32, i32) }
impl Src<'_> { fn to_pixels(&self, r: Rect) -> Rect { r.translate(-self.local0.0, -self.local0.1) } }

/// `src` in the pen numbers of `dst` (see the module comment), or `None` when they already agree (same depth and table).
pub fn convert_source(src: &Target, dst: &Target, fg: u8, bg: u8) -> Option<Bitmap> {
    if src.bm.depth() == dst.bm.depth() && src.clut == dst.clut { return None; }
    let rgb_of = |t: &Target, p: u8| -> Rgb { match &t.clut { Some(c) => c.get(usize::from(p)).copied().unwrap_or(Rgb::BLACK), None => if p == 0 { Rgb::WHITE } else { Rgb::BLACK } } };
    let lut: alloc::vec::Vec<u8> = (0..=src.bm.pen_mask()).map(|p| match (&src.clut, &dst.clut) {
        (None, Some(_)) => if p == 0 { bg } else { fg }, // a 1-bit image is coloured with the port's pens
        (_, None) => u8::from(luminance(rgb_of(src, p)) < 0x8000),
        (Some(_), Some(d)) => color::nearest(d, rgb_of(src, p)),
    }).collect();
    let mut out = Bitmap::new(src.bm.width(), src.bm.height(), dst.bm.depth())?;
    for y in 0..src.bm.height() { for x in 0..src.bm.width() { out.set(x, y, lut[usize::from(src.bm.get(x, y).unwrap_or(0))]); } }
    Some(out)
}

/// Rec. 601 luma of a colour, 16 bits (what decides black or white when a deep image goes to a 1-bit map).
fn luminance(c: Rgb) -> u32 { (u32::from(c.r) * 299 + u32::from(c.g) * 587 + u32::from(c.b) * 114) / 1000 }

fn blit_them<U: GuestUi>(m: &Mac<U>, clip_port: u32, dt: &mut Target, st: Option<Src>, sr: Rect, dr: Rect, mt: u8, mask: Option<&retro_raster::Region>) -> Result<Rect, Refusal> {
    let clip_region = clip_region_via(m, clip_port, dt)?;
    let c = clip_of(&clip_region, dt);
    let (l0, d) = (dt.local0, dt.to_pixels(dr));
    let maskf = |x: i32, y: i32| mask.map_or(true, |g| g.contains(x + l0.0, y + l0.1));
    let mf: retro_raster::blit::MaskFn = if mask.is_some() { Some(&maskf) } else { None };
    if (sr.width(), sr.height()) != (dr.width(), dr.height()) { return stretch_them(dt, st, sr, d, mt, mf, &c); }
    Ok(match st {
        Some(s) => { let sp = s.to_pixels(sr); blit(&mut dt.bm, d.x0, d.y0, s.bm, sp.x0, sp.y0, sp.width(), sp.height(), mt, 0xFF, mf, &c) }
        None => { let sp = dt.to_pixels(sr); blit_within(&mut dt.bm, d.x0, d.y0, sp.x0, sp.y0, sp.width(), sp.height(), mt, 0xFF, mf, &c) }
    })
}

/// StretchBits: each destination row of the DDA is blitted as a one-row source through the same mode, mask and clip.
fn stretch_them(dt: &mut Target, st: Option<Src>, sr: Rect, d: Rect, mt: u8, mf: retro_raster::blit::MaskFn, c: &retro_raster::paint::Clip) -> Result<Rect, Refusal> {
    let mut touched = Rect::default();
    let depth = dt.bm.depth();
    let mut emit_into = |dst: &mut retro_raster::Bitmap, row: i32, pens: &[u8]| -> Result<(), Refusal> {
        let mut one = retro_raster::Bitmap::new(pens.len() as i32, 1, depth).ok_or_else(|| Refusal("CopyBits: scaled row too wide".into()))?;
        for (x, &p) in pens.iter().enumerate() { one.set(x as i32, 0, p); }
        let t = blit(dst, d.x0, d.y0 + row, &one, 0, 0, pens.len() as i32, 1, mt, 0xFF, mf, c);
        touched = if touched.is_empty() { t } else { touched.union(&t) };
        Ok(())
    };
    match st {
        Some(s) => { let sp = s.to_pixels(sr); stretch(s.bm, sp, d.width(), d.height(), |row, pens| emit_into(&mut dt.bm, row, pens))?; }
        // a scaled copy within one bitmap must not read what it has drawn: only then is the source snapshotted
        None => { let sp = dt.to_pixels(sr); let source = dt.bm.clone(); stretch(&source, sp, d.width(), d.height(), |row, pens| emit_into(&mut dt.bm, row, pens))?; }
    }
    Ok(touched)
}

fn copy_bits<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (sb, db, mode) = (a.get(0), a.get(1), a.get(4) as u16);
    let (sr, dr) = (raster(read_rect(&m.mem, a.get(2))?), raster(read_rect(&m.mem, a.get(3))?));
    let mode_bits = mode & !64; // 64 = ditherCopy: no dithering is done, whatever the depths
    if mode_bits > 7 { return Err(Refusal(format!("CopyBits: transfer mode {mode} is not implemented (source modes 0..7 only: pattern modes 8..15 need a pattern source, the arithmetic modes 32..39 a blend)"))); }
    let mask = if a.get(5) != 0 { Some(m.read_region(a.get(5))?) } else { None };
    copy_through(m, "CopyBits", sb, db, sr, dr, mode_bits, mask)
}

/// `CopyMask(srcBits, maskBits, dstBits, srcRect, maskRect, dstRect)`: srcCopy of the source into the destination where the
/// 1-bit mask has a black pixel (Inside Macintosh VI "QuickDraw"; the three rectangles are the same size here, a scaling mask
/// is refused by name). It is `CopyBits` with the mask expressed as a destination region, so clipping, depths and the pen
/// rules are the very same.
fn copy_mask<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (sb, mb, db) = (a.get(0), a.get(1), a.get(2));
    let (sr, mr, dr) = (raster(read_rect(&m.mem, a.get(3))?), raster(read_rect(&m.mem, a.get(4))?), raster(read_rect(&m.mem, a.get(5))?));
    if (sr.width(), sr.height()) != (mr.width(), mr.height()) || (sr.width(), sr.height()) != (dr.width(), dr.height()) {
        return Err(Refusal(format!("CopyMask: source {}x{}, mask {}x{} and destination {}x{} differ: a scaling mask is not implemented", sr.width(), sr.height(), mr.width(), mr.height(), dr.width(), dr.height())));
    }
    let mt = open_bits(m, mb)?;
    let region = (|| {
        if mt.bm.depth() != 1 { return Err(Refusal(format!("CopyMask: the mask {mb:#x} is {} bits deep: only a 1-bit mask is implemented", mt.bm.depth()))); }
        let mp = mt.to_pixels(mr);
        let mut g = retro_raster::Region::new();
        for y in 0..mp.height() {
            let mut x = 0;
            while x < mp.width() {
                if mt.bm.get(mp.x0 + x, mp.y0 + y) != Some(1) { x += 1; continue; }
                let start = x;
                while x < mp.width() && mt.bm.get(mp.x0 + x, mp.y0 + y) == Some(1) { x += 1; }
                g.or_rect(Rect::new(dr.x0 + start, dr.y0 + y, dr.x0 + x, dr.y0 + y + 1)); // destination LOCAL coordinates
            }
        }
        Ok(g)
    })();
    finish(m, mt, Rect::default())?;
    copy_through(m, "CopyMask", sb, db, sr, dr, 0, Some(region?))
}

/// The shared body of `CopyBits` and `CopyMask`: open both ends, express the source in the destination's pens, blit.
fn copy_through<U: GuestUi>(m: &mut Mac<U>, name: &str, sb: u32, db: u32, sr: Rect, dr: Rect, mode_bits: u16, mask: Option<retro_raster::Region>) -> Result<Done, Refusal> {
    let mode = mode_bits;
    let mut dt = open_bits(m, db)?;
    let clip_port = match port_for_bits(m, db) { Ok(p) => p, Err(e) => { finish(m, dt, Rect::default())?; return Err(e); } };
    let hidden = clip_port != 0 && m.mem.read_i16(clip_port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis")).map_or(false, |v| v < 0);
    let st = if sb == db { None } else {
        match open_bits(m, sb) { Ok(t) => Some(t), Err(e) => { finish(m, dt, Rect::default())?; return Err(e); } }
    };
    let touched = if hidden { Ok(Rect::default()) } else {
        let (fg, bg) = match pens(m, current_port(m).unwrap_or(0), &dt) {
            Ok(p) => p,
            Err(e) => { if let Some(s) = st { finish(m, s, Rect::default())?; } finish(m, dt, Rect::default())?; return Err(e); }
        };
        let converted = st.as_ref().and_then(|s| convert_source(s, &dt, fg, bg));
        let src = st.as_ref().map(|s| Src { bm: converted.as_ref().unwrap_or(&s.bm), local0: s.local0 });
        blit_them(m, clip_port, &mut dt, src, sr, dr, minterm(mode_bits), mask.as_ref())
    };
    if let Some(s) = st { finish(m, s, Rect::default())?; }
    finish(m, dt, *touched.as_ref().unwrap_or(&Rect::default()))?;
    touched?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name} mode {mode} {}x{} {sb:#x} -> {db:#x}{}", dr.width(), dr.height(), if mask.is_some() { " (masked)" } else { "" }) })
}

