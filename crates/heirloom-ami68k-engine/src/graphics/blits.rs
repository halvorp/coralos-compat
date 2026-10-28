//! BltBitMap, BltBitMapRastPort, ClipBlit, BltMaskBitMapRastPort, BltTemplate: minterm blits (every minterm, see
//! heirloom_raster::blit) between host-backed bitmaps and window canvases. A source or destination coordinate outside a
//! bitmap is clipped, never faulted; a destination RastPort's layer clip region applies.
use super::layout::*;
use super::{bitmaps, rastport, target::{self, Dest}};
use crate::{machine::Amiga, Refusal};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::{blit::{blit, blit_within, MaskFn}, paint::{stencil, Clip}, Rect};

/// One blit from the bitmap `src` at (sx, sy) into `dst` at RastPort coordinates (dx, dy).
#[allow(clippy::too_many_arguments)]
fn do_blit<U: GuestUi>(a: &mut Amiga<U>, src: u32, (sxx, syy): (i32, i32), dst: Dest, (dx, dy): (i32, i32), (w, h): (i32, i32), mt: u8, planes: u8, mask: MaskFn) -> Result<Rect, Refusal> {
    let Dest::Bitmap { bm, origin, window, region } = dst else { return Ok(Rect::default()) };
    let (bx, by) = (dx - origin.0, dy - origin.1);
    bitmaps::ensure_source(a, bm)?;
    let hit = if bm == src {
        let g = a.gfx.bitmaps.get_mut(&bm).ok_or_else(|| Refusal(format!("blit: BitMap {bm:#x} vanished")))?;
        let clip = Clip { rect: g.bm.rect(), region: region.as_ref() };
        blit_within(&mut g.bm, bx, by, sxx, syy, w, h, mt, planes, mask, &clip)
    } else {
        let s = a.gfx.bitmaps.remove(&src).ok_or_else(|| Refusal(format!("blit: source BitMap {src:#x} vanished")))?;
        let r = match a.gfx.bitmaps.get_mut(&bm) {
            Some(g) => { let clip = Clip { rect: g.bm.rect(), region: region.as_ref() }; Ok(blit(&mut g.bm, bx, by, &s.bm, sxx, syy, w, h, mt, planes, mask, &clip)) }
            None => Err(Refusal(format!("blit: destination BitMap {bm:#x} vanished"))),
        };
        a.gfx.bitmaps.insert(src, s);
        r?
    };
    // A blit is drawing the engine cannot name: damage AND a hole, so the plane route never loses it.
    if let Some(win) = window { a.gfx.add_damage(win, hit); super::record::note(a, win, None, hit, (0, 0)); }
    bitmaps::export_planes(a, bm)?;
    Ok(hit)
}

/// BltBitMap(a0 = src, d0/d1 = xSrc/ySrc, a1 = dst, d2/d3 = xDest/yDest, d4/d5 = w/h, d6 = minterm, d7 = plane mask).
pub fn blt_bit_map<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s, d) = (cpu.areg(0), cpu.areg(1));
    let (sxy, dxy, wh) = ((sx(cpu.dreg(0)), sx(cpu.dreg(1))), (sx(cpu.dreg(2)), sx(cpu.dreg(3))), (sx(cpu.dreg(4)), sx(cpu.dreg(5))));
    let (mt, planes) = (cpu.dreg(6) as u8, cpu.dreg(7) as u8);
    bitmaps::ensure_source(a, s)?; bitmaps::ensure(a, d)?;
    let window = a.gfx.bitmaps[&d].window;
    let n = a.gfx.bitmaps[&s].bm.depth().min(a.gfx.bitmaps[&d].bm.depth());
    do_blit(a, s, sxy, Dest::Bitmap { bm: d, origin: (0, 0), window, region: None }, dxy, wh, mt, planes, None)?;
    cpu.set_d0(n as u32);
    Ok(format!("[graphics] BltBitMap {s:#x} ({},{}) -> {d:#x} ({},{}) {}x{} minterm {mt:#x}", sxy.0, sxy.1, dxy.0, dxy.1, wh.0, wh.1))
}

/// BltBitMapRastPort(a0 = src, d0/d1 = xSrc/ySrc, a1 = rp, d2/d3 = xDest/yDest, d4/d5 = w/h, d6 = minterm).
pub fn blt_bit_map_rast_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s, rp) = (cpu.areg(0), cpu.areg(1));
    let (sxy, dxy, wh, mt) = ((sx(cpu.dreg(0)), sx(cpu.dreg(1))), (sx(cpu.dreg(2)), sx(cpu.dreg(3))), (sx(cpu.dreg(4)), sx(cpu.dreg(5))), cpu.dreg(6) as u8);
    bitmaps::ensure_source(a, s)?;
    let dst = target::resolve(a, rp)?;
    do_blit(a, s, sxy, dst, dxy, wh, mt, 0xFF, None)?;
    Ok(format!("[graphics] BltBitMapRastPort {s:#x} -> rp {rp:#x} ({},{}) {}x{} minterm {mt:#x}", dxy.0, dxy.1, wh.0, wh.1))
}

/// ClipBlit(a0 = srcRP, d0/d1 = xSrc/ySrc, a1 = dstRP, d2/d3 = xDest/yDest, d4/d5 = w/h, d6 = minterm).
pub fn clip_blit<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (srp, drp) = (cpu.areg(0), cpu.areg(1));
    let (sxy, dxy, wh, mt) = ((sx(cpu.dreg(0)), sx(cpu.dreg(1))), (sx(cpu.dreg(2)), sx(cpu.dreg(3))), (sx(cpu.dreg(4)), sx(cpu.dreg(5))), cpu.dreg(6) as u8);
    let Dest::Bitmap { bm: sbm, origin: so, .. } = target::resolve(a, srp)? else { return Ok(format!("[graphics] ClipBlit from a dropped RastPort {srp:#x}: nothing")) };
    let dst = target::resolve(a, drp)?;
    do_blit(a, sbm, (sxy.0 - so.0, sxy.1 - so.1), dst, dxy, wh, mt, 0xFF, None)?;
    Ok(format!("[graphics] ClipBlit rp {srp:#x} ({},{}) -> rp {drp:#x} ({},{}) {}x{} minterm {mt:#x}", sxy.0, sxy.1, dxy.0, dxy.1, wh.0, wh.1))
}

/// Rows `first..first+n` of a 1-bit plane at `p` (`bpr` bytes per row) as a bit lookup closure's backing store.
fn plane_rows<U: GuestUi>(a: &Amiga<U>, p: u32, bpr: u32, first: i32, n: i32) -> Result<Vec<u8>, Refusal> {
    let mut v = Vec::with_capacity((bpr as i32 * n.max(0)) as usize);
    for r in 0..n.max(0) { for c in 0..bpr { v.push(a.mem.read_u8(p + (first + r) as u32 * bpr + c).map_err(Refusal::mem("1-bit plane"))?); } }
    Ok(v)
}
fn bit(rows: &[u8], bpr: u32, x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as u32) < bpr * 8 && rows.get((y as u32 * bpr + (x as u32 >> 3)) as usize).map_or(false, |b| (b >> (7 - (x & 7))) & 1 != 0)
}

/// BltMaskBitMapRastPort(a0 = src, d0/d1 = xSrc/ySrc, a1 = rp, d2/d3 = xDest/yDest, d4/d5 = w/h, d6 = minterm, a2 = mask plane).
pub fn blt_mask_bit_map_rast_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s, rp, mp) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let (sxy, dxy, wh, mt) = ((sx(cpu.dreg(0)), sx(cpu.dreg(1))), (sx(cpu.dreg(2)), sx(cpu.dreg(3))), (sx(cpu.dreg(4)), sx(cpu.dreg(5))), cpu.dreg(6) as u8);
    bitmaps::ensure_source(a, s)?;
    let dst = target::resolve(a, rp)?;
    let origin = match &dst { Dest::Bitmap { origin, .. } => *origin, Dest::Dropped(_) => (0, 0) };
    let bpr = a.mem.read_u16(s + BM_BYTES_PER_ROW).map_err(Refusal::mem("BitMap"))? as u32;
    let rows = if mp == 0 { Vec::new() } else { plane_rows(a, mp, bpr, 0, (sxy.1 + wh.1).max(0))? };
    // destination pixel (bitmap coords) -> the source pixel that fed it
    let (ox, oy) = (dxy.0 - origin.0 - sxy.0, dxy.1 - origin.1 - sxy.1);
    let mask = move |x: i32, y: i32| mp == 0 || bit(&rows, bpr, x - ox, y - oy);
    do_blit(a, s, sxy, dst, dxy, wh, mt, 0xFF, Some(&mask))?;
    Ok(format!("[graphics] BltMaskBitMapRastPort {s:#x} -> rp {rp:#x} ({},{}) {}x{} minterm {mt:#x} mask {mp:#x}", dxy.0, dxy.1, wh.0, wh.1))
}

/// BltTemplate(a0 = template, d0 = xSrc, d1 = srcMod, a1 = rp, d2/d3 = xDest/yDest, d4/d5 = w/h): a 1-bit template
/// drawn in the RastPort's pens and draw mode.
pub fn blt_template<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (src, xs, modulo, rp) = (cpu.areg(0), sx(cpu.dreg(0)), cpu.dreg(1) as u16 as u32, cpu.areg(1));
    let (dx, dy, w, h) = (sx(cpu.dreg(2)), sx(cpu.dreg(3)), sx(cpu.dreg(4)), sx(cpu.dreg(5)));
    if w <= 0 || h <= 0 { return Ok("[graphics] BltTemplate empty".into()); }
    let st = rastport::read(a, rp)?;
    let cols = ((xs.max(0) + w + 7) >> 3) as u32;
    let mut rows = Vec::with_capacity((cols as i32 * h) as usize);
    for r in 0..h as u32 { for c in 0..cols { rows.push(a.mem.read_u8(src + r * modulo + c).map_err(Refusal::mem("BltTemplate source"))?); } }
    let brush = st.brush();
    target::paint(a, rp, "BltTemplate", |bm, clip, (ox, oy)| {
        let area = Rect::from_size(dx - ox, dy - oy, w, h);
        stencil(bm, area, &brush, clip, |x, y| bit(&rows, cols, x - (dx - ox) + xs, y - (dy - oy)))
    })?;
    Ok(format!("[graphics] BltTemplate {src:#x} -> rp {rp:#x} ({dx},{dy}) {w}x{h}"))
}
