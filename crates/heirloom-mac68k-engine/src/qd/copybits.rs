//! `CopyBits(srcBits, dstBits, srcRect, dstRect, mode, maskRgn)` between any two BitMaps: a window's `portBits` (its
//! canvas), or a real BitMap / PixMap in guest memory (an offscreen port or world). One `heirloom_raster::blit` with the transfer
//! mode as a minterm; `maskRgn` (destination LOCAL coordinates) leaves the destination alone outside it. The destination
//! is clipped by `thePort`'s clipRgn and visRgn ONLY when it is `thePort`'s `portBits` (same baseAddr and
//! bounds.topLeft, QuickDraw/Bitmaps.a TOPORT), and not at all when `thePort`'s pen is hidden (StdBits: `pnVis < 0`);
//! any other destination is drawn with a wide-open clip (NOTPORT).
//!
//! Scaling (srcRect and dstRect of different size) is StretchBits ([`super::stretch`]), one destination row at a time.
//!
//! **Depths.** Source and destination may be 1-bit BitMaps, indexed PixMaps (2, 4, 8 bits) or direct PixMaps (16, 32 bits).
//! The source is first expressed in the DESTINATION's pixel values ([`convert_source`], through the one converter
//! `crate::cqd::direct::Conv`: colourising a 1-bit source with the port's pens, table to table, table to direct, 16 <-> 32,
//! direct to an indexed map through its inverse table, anything to 1 bit by luminance; no dithering, `ditherCopy` or not).
//! The Boolean transfer modes 0..7 then act on the destination's pixel values, one implementation for all depths; the
//! arithmetic modes 32..39 ([`crate::cqd::direct::arith`]) combine colours per component with the port's OpColor and are the
//! Boolean equivalents on a 1-bit destination. Refused by name: pattern modes 8..15, hilite (50), an arithmetic mode with
//! scaling.
use super::port::{bits_addr, is_cport, raster, BM_BASE, BM_BOUNDS, P_PN_VIS};
use super::target::{clip_of, clip_region_via, current_port, finish, open_bits, pens, Target};
use crate::cqd::color::{self, Rgb};
use crate::cqd::direct::{self, Conv, Space};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::blit::{blit, blit_within};
use heirloom_raster::paint::stencil_with;
use super::stretch::stretch;
use heirloom_raster::{Bitmap, Rect, Region};

/// srcCopy..notSrcBic as a minterm over (A = mask, B = source, C = destination), with the mask-off terms keeping C.
pub fn minterm(mode: u16) -> u8 {
    let base = match mode & 7 { 0 => 0xC0, 1 => 0xE0, 2 => 0x60, 3 => 0x20, 4 => 0x30, 5 => 0xB0, 6 => 0x90, _ => 0x80 };
    base | 0x0A
}

/// `grafVars` of a CGrafPort (IWQ "GrafVars": `rgbOpColor` 0, `rgbHiliteColor` 6, `pmFgColor` 12 ... 26 bytes).
pub const CP_GRAF_VARS: u32 = 8;
pub const GV_SIZE: u32 = 26;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA8EC, "CopyBits", &[Arg::L, Arg::L, Arg::L, Arg::L, Arg::W, Arg::L], None, copy_bits::<U>)?;
    super::reg(m, 0xA817, "CopyMask", &[Arg::L, Arg::L, Arg::L, Arg::L, Arg::L, Arg::L], None, copy_mask::<U>)?;
    super::reg(m, 0xAA51, "CopyDeepMask", &[Arg::L, Arg::L, Arg::L, Arg::L, Arg::L, Arg::L, Arg::W, Arg::L], None, copy_deep_mask::<U>)?;
    super::reg(m, 0xAA21, "OpColor", &[Arg::P], None, op_color_trap::<U>)
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

impl<U: GuestUi> Mac<U> {
    /// The inverse table of `table` at the default resolution (4 bits, `gdResPref`). A cache of the most recent tables: a miss
    /// rebuilds it (4096 nearest-colour cells), it is never a refusal.
    pub(crate) fn inverse_table(&mut self, table: &[Rgb]) -> direct::ITab {
        const RECENT: usize = 8; // a cache's working set, not a limit on anything the guest has
        if let Some(i) = self.cqd.itabs.iter().position(|(t, _)| t.as_slice() == table) {
            let e = self.cqd.itabs.remove(i);
            let it = e.1.clone();
            self.cqd.itabs.insert(0, e);
            return it;
        }
        let it = direct::ITab::build(table, 4).unwrap_or(direct::ITab { res: 4, cells: alloc::vec![0; 4096] });
        self.cqd.itabs.insert(0, (table.to_vec(), it.clone()));
        self.cqd.itabs.truncate(RECENT);
        it
    }
}

/// `src` in the pixel values of `dst` (see the module comment), or `None` when they already agree (same depth and table).
pub fn convert_source<U: GuestUi>(m: &mut Mac<U>, src: &Target, dst: &Target, fg: u32, bg: u32) -> Option<Bitmap> {
    if src.bm.depth() == dst.bm.depth() && src.clut == dst.clut { return None; }
    let itab = match (src.space(), &dst.clut) { (Space::Direct(_), Some(t)) if !dst.bm.is_direct() && dst.bm.depth() > 1 => Some(m.inverse_table(t)), _ => None };
    let conv = Conv::new(src.space(), src.bm.depth(), dst.space(), fg, bg, itab.as_ref())?;
    let mut out = Bitmap::new(src.bm.width(), src.bm.height(), dst.bm.depth())?;
    let mut row = alloc::vec::Vec::with_capacity(src.bm.width() as usize);
    for y in 0..src.bm.height() {
        row.clear();
        src.bm.read_span(y, 0, src.bm.width(), &mut row);
        conv.row(&mut row);
        out.write_span(y, 0, &row);
    }
    Some(out)
}

/// The port's OpColor: `grafVars^^.rgbOpColor` of a colour port; black when the port has no `grafVars` (or is classic).
pub fn op_color<U: GuestUi>(m: &Mac<U>, port: u32) -> Rgb {
    if port == 0 || !is_cport(&m.mem, port) { return Rgb::BLACK; }
    let h = m.mem.read_u32(port.wrapping_add(CP_GRAF_VARS)).unwrap_or(0);
    m.mm.block(m.mem.strip(h)).map_or(Rgb::BLACK, |b| color::read_rgb(&m.mem, b.ptr).unwrap_or(Rgb::BLACK))
}

/// `OpColor(&rgb)`: the current colour port's OpColor (its `grafVars` record is made when it has none). A classic port has none:
/// the call does nothing there, as on a Mac.
fn op_color_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (port, rgb) = (current_port(m)?, color::read_rgb(&m.mem, c.args.get(0))?);
    if !is_cport(&m.mem, port) { return Ok(Done { result: 0, log: format!("[quickdraw] OpColor on the classic port {port:#x}: no grafVars, nothing kept") }); }
    let mut h = m.mem.read_u32(port.wrapping_add(CP_GRAF_VARS)).map_err(Refusal::mem("CGrafPort"))?;
    if m.mm.block(m.mem.strip(h)).is_none() {
        h = m.alloc_handle(GV_SIZE, "grafVars")?.0;
        m.mem.write_u32(port.wrapping_add(CP_GRAF_VARS), h).map_err(Refusal::mem("CGrafPort"))?;
    }
    let p = m.deref_handle(h, "grafVars")?;
    color::write_rgb(&mut m.mem, p, rgb)?;
    Ok(Done { result: 0, log: format!("[quickdraw] OpColor ({:04x},{:04x},{:04x})", rgb.r, rgb.g, rgb.b) })
}

fn blit_them<U: GuestUi>(m: &Mac<U>, clip_port: u32, dt: &mut Target, st: Option<Src>, sr: Rect, dr: Rect, mt: u8, mask: Option<&Region>) -> Result<Rect, Refusal> {
    let clip_region = clip_region_via(m, clip_port, dt)?;
    let c = clip_of(&clip_region, dt);
    let (l0, d) = (dt.local0, dt.to_pixels(dr));
    let maskf = |x: i32, y: i32| mask.map_or(true, |g| g.contains(x + l0.0, y + l0.1));
    let mf: heirloom_raster::blit::MaskFn = if mask.is_some() { Some(&maskf) } else { None };
    if (sr.width(), sr.height()) != (dr.width(), dr.height()) { return stretch_them(dt, st, sr, d, mt, mf, &c); }
    Ok(match st {
        Some(s) => { let sp = s.to_pixels(sr); blit(&mut dt.bm, d.x0, d.y0, s.bm, sp.x0, sp.y0, sp.width(), sp.height(), mt, u32::MAX, mf, &c) }
        None => { let sp = dt.to_pixels(sr); blit_within(&mut dt.bm, d.x0, d.y0, sp.x0, sp.y0, sp.width(), sp.height(), mt, u32::MAX, mf, &c) }
    })
}

/// How a pixel of the destination is combined with the (converted) source pixel: an arithmetic mode, `transparent`, or a
/// deep mask's weighted average.
/// `Bool` is a Boolean mode 1..7 into a DIRECT destination, by IWQ's colour definitions ("Boolean transfer modes with color
/// pixels"): the source pixel is INK when it is black-ish (a 1-bit source's 1; a deeper source's colour below half luminance), and
/// srcOr writes the foreground there, srcXor inverts the destination's colour bits, srcBic writes the background (the not- modes
/// take the paper instead); notSrcCopy is the source colour inverted. The source is read UNCONVERTED, in its own space.
enum Combine<'a> { Arith(u16, Rgb), Transparent(u32), Weigh(&'a Bitmap, (i32, i32), Space<'a>), Bool { mode: u16, fg: u32, bg: u32, src: Space<'a> } }

/// The arithmetic modes and deep masks: per destination pixel inside the clip (and the mask region), the source and destination
/// colours combined, written back as a destination pixel (direct: its bits; indexed: through the inverse table). Unscaled only.
fn combine_them<U: GuestUi>(m: &mut Mac<U>, clip_port: u32, dt: &mut Target, st: Option<Src>, sr: Rect, dr: Rect, how: Combine, mask: Option<&Region>) -> Result<Rect, Refusal> {
    if (sr.width(), sr.height()) != (dr.width(), dr.height()) { return Err(Refusal(format!("CopyBits: an arithmetic mode or a deep mask with scaling ({}x{} -> {}x{}) is not implemented", sr.width(), sr.height(), dr.width(), dr.height()))); }
    let clip_region = clip_region_via(m, clip_port, dt)?;
    let mut g = clip_region.clone();
    if let Some(mk) = mask { let mut mm = mk.clone(); mm.translate(-dt.local0.0, -dt.local0.1); g.and_region(&mm); }
    let itab = match &dt.clut { Some(t) if dt.bm.depth() > 1 => Some(m.inverse_table(t)), _ => None };
    let snapshot; // a copy within one bitmap reads the source before any of it is drawn
    let (sbm, sp): (&Bitmap, Rect) = match st { Some(s) => (s.bm, s.to_pixels(sr)), None => { snapshot = dt.bm.clone(); (&snapshot, dt.to_pixels(sr)) } };
    let d = dt.to_pixels(dr);
    let (depth, clut) = (dt.bm.depth(), dt.clut.clone());
    let space = match &clut { _ if depth > 8 => Space::Direct(depth), Some(t) => Space::Indexed(t), None => Space::Mono };
    let to_px = |c: Rgb| -> u32 { match space { Space::Direct(dd) => direct::pixel_of(dd, c), Space::Indexed(_) => itab.as_ref().map_or(0, |i| u32::from(i.lookup(c))), Space::Mono => u32::from(direct::luminance(c) < 0x8000) } };
    let c = Clip { rect: dt.bm.rect(), region: Some(&g) };
    Ok(stencil_with(&mut dt.bm, d, &c, |x, y, old| {
        let s = sbm.get(x - d.x0 + sp.x0, y - d.y0 + sp.y0)?;
        Some(match &how {
            Combine::Transparent(bg) => direct::transparent(s, old, *bg),
            Combine::Bool { mode, fg, bg, src } => {
                let cm = if depth == 16 { 0x7FFF } else { 0x00FF_FFFF };
                let c = src.rgb(s);
                if mode & 7 == 4 { return Some(to_px(c) ^ cm); }
                let ink = (match src { Space::Mono => s == 1, _ => direct::luminance(c) < 0x8000 }) ^ (mode & 4 != 0);
                match (mode & 3, ink) { (1, true) => *fg, (2, true) => old ^ cm, (3, true) => *bg, _ => old }
            }
            Combine::Arith(md, op) => to_px(direct::arith(*md, space.rgb(s), space.rgb(old), *op)?),
            Combine::Weigh(mb, (mx, my), ms) => {
                let w = ms.rgb(mb.get(x - d.x0 + mx, y - d.y0 + my)?);
                let (a, b) = (space.rgb(s), space.rgb(old));
                to_px(Rgb { r: direct::weigh(a.r, b.r, w.r), g: direct::weigh(a.g, b.g, w.g), b: direct::weigh(a.b, b.b, w.b) })
            }
        })
    }))
}
use heirloom_raster::paint::Clip;

/// StretchBits: each destination row of the DDA is blitted as a one-row source through the same mode, mask and clip.
fn stretch_them(dt: &mut Target, st: Option<Src>, sr: Rect, d: Rect, mt: u8, mf: heirloom_raster::blit::MaskFn, c: &heirloom_raster::paint::Clip) -> Result<Rect, Refusal> {
    let mut touched = Rect::default();
    let depth = dt.bm.depth();
    let mut emit_into = |dst: &mut Bitmap, row: i32, pens: &[u32]| -> Result<(), Refusal> {
        let mut one = Bitmap::new(pens.len() as i32, 1, depth).ok_or_else(|| Refusal("CopyBits: scaled row too wide".into()))?;
        one.write_span(0, 0, pens);
        let t = blit(dst, d.x0, d.y0 + row, &one, 0, 0, pens.len() as i32, 1, mt, u32::MAX, mf, c);
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

/// The transfer modes `CopyBits` draws: the source modes 0..7 (with `ditherCopy`, 64, ignored) and the arithmetic modes 32..39.
fn check_mode(name: &str, mode: u16) -> Result<u16, Refusal> {
    let bits = mode & !64; // 64 = ditherCopy: no dithering is done, whatever the depths
    if bits <= 7 || (32..=39).contains(&bits) { return Ok(bits); }
    Err(Refusal(format!("{name}: transfer mode {mode} is not implemented (source modes 0..7 and arithmetic 32..39 only: pattern modes 8..15 need a pattern source, hilite 50 the highlight colour)")))
}

fn copy_bits<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (sb, db, mode) = (a.get(0), a.get(1), a.get(4) as u16);
    let (sr, dr) = (raster(read_rect(&m.mem, a.get(2))?), raster(read_rect(&m.mem, a.get(3))?));
    let mode_bits = check_mode("CopyBits", mode)?;
    let mask = if a.get(5) != 0 { Some(m.read_region(a.get(5))?) } else { None };
    copy_through(m, "CopyBits", sb, db, sr, dr, mode_bits, mask)
}

/// The black pixels of the 1-bit mask `mt` over `mr` as a region in DESTINATION local coordinates (`dr`'s top-left).
fn mask_region(mt: &Target, mr: Rect, dr: Rect) -> Region {
    let mp = mt.to_pixels(mr);
    let mut g = Region::new();
    for y in 0..mp.height() {
        let mut x = 0;
        while x < mp.width() {
            if mt.bm.get(mp.x0 + x, mp.y0 + y) != Some(1) { x += 1; continue; }
            let start = x;
            while x < mp.width() && mt.bm.get(mp.x0 + x, mp.y0 + y) == Some(1) { x += 1; }
            g.or_rect(Rect::new(dr.x0 + start, dr.y0 + y, dr.x0 + x, dr.y0 + y + 1));
        }
    }
    g
}

/// `CopyMask(srcBits, maskBits, dstBits, srcRect, maskRect, dstRect)`: srcCopy of the source into the destination where the
/// 1-bit mask has a black pixel (Inside Macintosh VI "QuickDraw"; the three rectangles are the same size here, a scaling mask
/// is refused by name). It is `CopyBits` with the mask expressed as a destination region, so clipping, depths and the pen
/// rules are the very same.
fn copy_mask<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (sb, mb, db) = (a.get(0), a.get(1), a.get(2));
    let (sr, mr, dr) = (raster(read_rect(&m.mem, a.get(3))?), raster(read_rect(&m.mem, a.get(4))?), raster(read_rect(&m.mem, a.get(5))?));
    same_sizes("CopyMask", sr, mr, dr)?;
    let mt = open_bits(m, mb)?;
    let region = if mt.bm.depth() != 1 { Err(Refusal(format!("CopyMask: the mask {mb:#x} is {} bits deep: CopyMask takes a 1-bit mask (CopyDeepMask a deeper one)", mt.bm.depth()))) } else { Ok(mask_region(&mt, mr, dr)) };
    finish(m, mt, Rect::default())?;
    copy_through(m, "CopyMask", sb, db, sr, dr, 0, Some(region?))
}

fn same_sizes(name: &str, sr: Rect, mr: Rect, dr: Rect) -> Result<(), Refusal> {
    if (sr.width(), sr.height()) != (mr.width(), mr.height()) || (sr.width(), sr.height()) != (dr.width(), dr.height()) {
        return Err(Refusal(format!("{name}: source {}x{}, mask {}x{} and destination {}x{} differ: a scaling mask is not implemented", sr.width(), sr.height(), mr.width(), mr.height(), dr.width(), dr.height())));
    }
    Ok(())
}

/// `CopyDeepMask(srcBits, maskBits, dstBits, srcRect, maskRect, dstRect, mode, maskRgn)` (IWQ "CopyDeepMask"): a 1-bit mask is
/// CopyMask's region (with `mode` and `maskRgn`); a deeper mask is a per-component weighted average of source and destination,
/// the mask pixel's colour giving each component's weight (white: all source, black: all destination). With a deep mask only
/// srcCopy is implemented (another mode is refused by name).
fn copy_deep_mask<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (sb, mb, db, mode) = (a.get(0), a.get(1), a.get(2), a.get(6) as u16);
    let (sr, mr, dr) = (raster(read_rect(&m.mem, a.get(3))?), raster(read_rect(&m.mem, a.get(4))?), raster(read_rect(&m.mem, a.get(5))?));
    same_sizes("CopyDeepMask", sr, mr, dr)?;
    let mode_bits = check_mode("CopyDeepMask", mode)?;
    let rgn = if a.get(7) != 0 { Some(m.read_region(a.get(7))?) } else { None };
    let mt = open_bits(m, mb)?;
    if mt.bm.depth() == 1 {
        let mut g = mask_region(&mt, mr, dr);
        finish(m, mt, Rect::default())?;
        if let Some(r) = rgn { g.and_region(&r); }
        return copy_through(m, "CopyDeepMask", sb, db, sr, dr, mode_bits, Some(g));
    }
    if mode_bits != 0 { let d = mt.bm.depth(); finish(m, mt, Rect::default())?; return Err(Refusal(format!("CopyDeepMask: transfer mode {mode} with a {d}-bit mask is not implemented (srcCopy only)"))); }
    let r = copy_with(m, "CopyDeepMask", sb, db, sr, dr, 0, rgn, Some(&mt), mr);
    finish(m, mt, Rect::default())?;
    r
}

/// The shared body of `CopyBits`, `CopyMask` and `CopyDeepMask`: open both ends, express the source in the destination's
/// pixel values, then blit (Boolean modes) or combine (arithmetic modes, a deep mask).
pub(crate) fn copy_through<U: GuestUi>(m: &mut Mac<U>, name: &str, sb: u32, db: u32, sr: Rect, dr: Rect, mode_bits: u16, mask: Option<Region>) -> Result<Done, Refusal> {
    copy_with(m, name, sb, db, sr, dr, mode_bits, mask, None, Rect::default())
}

#[allow(clippy::too_many_arguments)]
fn copy_with<U: GuestUi>(m: &mut Mac<U>, name: &str, sb: u32, db: u32, sr: Rect, dr: Rect, mode_bits: u16, mask: Option<Region>, deep: Option<&Target>, mr: Rect) -> Result<Done, Refusal> {
    let mode = mode_bits;
    let mut dt = open_bits(m, db)?;
    let clip_port = match port_for_bits(m, db) { Ok(p) => p, Err(e) => { finish(m, dt, Rect::default())?; return Err(e); } };
    let hidden = clip_port != 0 && m.mem.read_i16(clip_port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis")).map_or(false, |v| v < 0);
    let st = if sb == db { None } else {
        match open_bits(m, sb) { Ok(t) => Some(t), Err(e) => { finish(m, dt, Rect::default())?; return Err(e); } }
    };
    let touched = if hidden { Ok(Rect::default()) } else {
        let port = current_port(m).unwrap_or(0);
        match pens(m, port, &dt) {
            Err(e) => Err(e),
            Ok((fg, bg)) => {
                let direct_bool = dt.bm.is_direct() && (1..=7).contains(&mode);
                let converted = match &st { Some(s) if !direct_bool => convert_source(m, s, &dt, fg, bg), _ => None };
                let src = st.as_ref().map(|s| Src { bm: converted.as_ref().unwrap_or(&s.bm), local0: s.local0 });
                let mono = dt.bm.depth() == 1;
                if let Some(mt) = deep {
                    let mp = mt.to_pixels(mr);
                    combine_them(m, clip_port, &mut dt, src, sr, dr, Combine::Weigh(&mt.bm, (mp.x0, mp.y0), mt.space()), mask.as_ref())
                } else if direct_bool {
                    let sp = st.as_ref().map_or(Space::Direct(dt.bm.depth()), |s| s.space());
                    combine_them(m, clip_port, &mut dt, src, sr, dr, Combine::Bool { mode, fg, bg, src: sp }, mask.as_ref())
                } else if mode >= 32 && mono {
                    blit_them(m, clip_port, &mut dt, src, sr, dr, minterm(direct::mono_equivalent(mode).unwrap_or(0)), mask.as_ref())
                } else if mode == direct::mode::TRANSPARENT {
                    combine_them(m, clip_port, &mut dt, src, sr, dr, Combine::Transparent(bg), mask.as_ref())
                } else if mode >= 32 {
                    let op = op_color(m, port);
                    combine_them(m, clip_port, &mut dt, src, sr, dr, Combine::Arith(mode, op), mask.as_ref())
                } else {
                    blit_them(m, clip_port, &mut dt, src, sr, dr, minterm(mode), mask.as_ref())
                }
            }
        }
    };
    if let Some(s) = st { finish(m, s, Rect::default())?; }
    finish(m, dt, *touched.as_ref().unwrap_or(&Rect::default()))?;
    touched?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name} mode {mode} {}x{} {sb:#x} -> {db:#x}{}", dr.width(), dr.height(), if mask.is_some() { " (masked)" } else { "" }) })
}
