//! The screen palette: pens 0..256 of 8-bit RGB on the host, mirrored into the guest's ColorMap ColorTable (RGB4
//! words) so programs reading it directly agree. ObtainPen / ReleasePen / ObtainBestPenA / FindColor allocate pens the
//! way the Workbench screen does: pens 0..8 are the system's (shared, never handed out as free); a changed colour
//! damages every window canvas, because the engine presents resolved pixels.
use super::layout::*;
use crate::{machine::Amiga, tags, Refusal};
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::Rect;

const PENF_EXCLUSIVE: u32 = 1;
const PENF_NO_SETCOLOR: u32 = 2;
const OBP_FAIL_IF_BAD: u32 = 0x8400_0001;

fn rgb8(r: u32, g: u32, b: u32) -> u32 { (r & 0xFF) << 16 | (g & 0xFF) << 8 | (b & 0xFF) }
fn expand4(v: u32) -> u32 { (v & 15) * 17 }
fn dist(c: u32, r: u32, g: u32, b: u32) -> u32 {
    let d = |x: u32, y: u32| (x as i32 - y as i32).unsigned_abs();
    let (dr, dg, db) = (d(c >> 16 & 0xFF, r), d(c >> 8 & 0xFF, g), d(c & 0xFF, b));
    dr * dr + dg * dg + db * db
}

/// Write the whole palette into the guest ColorTable.
pub fn write_table<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    let table: alloc::vec::Vec<u16> = a.gfx.palette().rgb.iter().map(|&c| ((c >> 20 & 15) << 8 | (c >> 12 & 15) << 4 | (c >> 4 & 15)) as u16).collect();
    for (i, w) in table.into_iter().enumerate() { a.mem.write_u16(SCREEN_COLOR_TABLE + 2 * i as u32, w).map_err(Refusal::mem("ColorTable"))?; }
    Ok(())
}

/// The palette changed: mirror it to the guest and damage every window canvas (the host gets resolved pixels).
pub fn changed<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    write_table(a)?;
    let canvases: alloc::vec::Vec<(u32, Rect)> = a.gfx.layers.values().filter_map(|l| a.gfx.bitmaps.get(&l.canvas).map(|g| (l.window, g.bm.rect()))).collect();
    for (w, r) in canvases { a.gfx.add_damage(w, r); }
    Ok(())
}

fn set<U: GuestUi>(a: &mut Amiga<U>, n: usize, rgb: u32) -> Result<(), Refusal> {
    if n >= 256 { return Err(Refusal(format!("palette: pen {n} is outside 0..256"))); }
    a.gfx.palette().rgb[n] = rgb;
    changed(a)
}

pub fn set_rgb4<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (n, r, g, b) = (cpu.dreg(0) as u16 as i16, cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    if n >= 0 { set(a, n as usize, rgb8(expand4(r), expand4(g), expand4(b)))?; }
    Ok(format!("[graphics] SetRGB4 pen {n} = {:x}{:x}{:x}", r & 15, g & 15, b & 15))
}

pub fn set_rgb32<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (n, r, g, b) = (cpu.dreg(0), cpu.dreg(1) >> 24, cpu.dreg(2) >> 24, cpu.dreg(3) >> 24);
    if n < 256 { set(a, n as usize, rgb8(r, g, b))?; }
    Ok(format!("[graphics] SetRGB32 pen {n} = {r:02x}{g:02x}{b:02x}"))
}

pub fn load_rgb4<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (p, n) = (cpu.areg(1), (cpu.dreg(0) as u16 as usize).min(256));
    for i in 0..n {
        let w = a.mem.read_u16(p + 2 * i as u32).map_err(Refusal::mem("LoadRGB4"))? as u32;
        a.gfx.palette().rgb[i] = rgb8(expand4(w >> 8), expand4(w >> 4), expand4(w));
    }
    changed(a)?;
    Ok(format!("[graphics] LoadRGB4 {n} colours"))
}

/// LoadRGB32(a0 = vp, a1 = table): `{ UWORD count; UWORD first; ULONG r, g, b; ...; 0 }`.
pub fn load_rgb32<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(1);
    let m = Refusal::mem("LoadRGB32");
    let (n, first) = (a.mem.read_u16(p).map_err(&m)? as u32, a.mem.read_u16(p + 2).map_err(&m)? as u32);
    for i in 0..n {
        let at = p + 4 + 12 * i;
        let (r, g, b) = (a.mem.read_u32(at).map_err(&m)? >> 24, a.mem.read_u32(at + 4).map_err(&m)? >> 24, a.mem.read_u32(at + 8).map_err(&m)? >> 24);
        if first + i < 256 { a.gfx.palette().rgb[(first + i) as usize] = rgb8(r, g, b); }
    }
    changed(a)?;
    Ok(format!("[graphics] LoadRGB32 {n} colours from {first}"))
}

pub fn get_rgb4<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let n = cpu.dreg(0) as i32;
    let v = if (0..256).contains(&n) { let c = a.gfx.palette().rgb[n as usize]; (c >> 20 & 15) << 8 | (c >> 12 & 15) << 4 | (c >> 4 & 15) } else { u32::MAX };
    cpu.set_d0(v);
    Ok(format!("[graphics] GetRGB4 pen {n} -> {v:#x}"))
}

/// GetRGB32(a0 = cm, d0 = first, d1 = count, a1 = table).
pub fn get_rgb32<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (first, n, t) = (cpu.dreg(0), cpu.dreg(1), cpu.areg(1));
    for i in 0..n {
        let c = if first + i < 256 { a.gfx.palette().rgb[(first + i) as usize] } else { 0 };
        for (k, sh) in [16u32, 8, 0].into_iter().enumerate() {
            a.mem.write_u32(t + 12 * i + 4 * k as u32, (c >> sh & 0xFF) * 0x0101_0101).map_err(Refusal::mem("GetRGB32 table"))?;
        }
    }
    Ok(format!("[graphics] GetRGB32 {n} from {first}"))
}

/// ObtainPen(a0 = cm, d0 = n or -1, d1..d3 = r g b (32-bit), d4 = flags) -> the pen, or -1.
pub fn obtain_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (n, r, g, b, flags) = (cpu.dreg(0) as i32, cpu.dreg(1) >> 24, cpu.dreg(2) >> 24, cpu.dreg(3) >> 24, cpu.dreg(4));
    let pick = if n == -1 { (8..256usize).find(|&i| a.gfx.palette().refs[i] == 0) } else if (0..256).contains(&n) && a.gfx.palette().refs[n as usize] == 0 { Some(n as usize) } else { None };
    let res = match pick {
        None => -1,
        Some(i) => {
            let p = a.gfx.palette();
            p.refs[i] = 1;
            p.exclusive[i] = flags & PENF_EXCLUSIVE != 0;
            if flags & PENF_NO_SETCOLOR == 0 { set(a, i, rgb8(r, g, b))?; }
            i as i32
        }
    };
    cpu.set_d0(res as u32);
    Ok(format!("[graphics] ObtainPen {n} {r:02x}{g:02x}{b:02x} flags {flags:#x} -> {res}"))
}

pub fn release_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let n = cpu.dreg(0) as usize;
    release(a, n);
    Ok(format!("[graphics] ReleasePen {n}"))
}

/// Give one reference on pen `n` back (the host half of ReleasePen; pens outside the palette are ignored, as ReleasePen does).
pub fn release<U: GuestUi>(a: &mut Amiga<U>, n: usize) {
    if n < 256 {
        let p = a.gfx.palette();
        p.refs[n] = p.refs[n].saturating_sub(1);
        if p.refs[n] == 0 { p.exclusive[n] = false; }
    }
}

/// ObtainBestPenA(a0 = cm, d1..d3 = r g b, a1 = tags): an exact shared match, else a free pen set to the colour, else
/// the closest shared pen. OBP_FailIfBad turns "closest" into -1.
pub fn obtain_best_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (r, g, b) = (cpu.dreg(1) >> 24, cpu.dreg(2) >> 24, cpu.dreg(3) >> 24);
    let fail_if_bad = tags::get(&tags::walk(&a.mem, cpu.areg(1))?, OBP_FAIL_IF_BAD).unwrap_or(0) != 0;
    let res = obtain_best(a, r, g, b, fail_if_bad)?;
    cpu.set_d0(res as u32);
    Ok(format!("[graphics] ObtainBestPenA {r:02x}{g:02x}{b:02x} -> {res}"))
}

/// The pen allocator behind ObtainBestPenA (8-bit components); -1 when none can be had.
pub fn obtain_best<U: GuestUi>(a: &mut Amiga<U>, r: u32, g: u32, b: u32, fail_if_bad: bool) -> Result<i32, Refusal> {
    let p = a.gfx.palette();
    let exact = (0..256).find(|&i| p.refs[i] > 0 && !p.exclusive[i] && dist(p.rgb[i], r, g, b) == 0);
    Ok(if let Some(i) = exact { p.refs[i] += 1; i as i32 }
        else if let Some(i) = (8..256usize).find(|&i| p.refs[i] == 0) { p.refs[i] = 1; set(a, i, rgb8(r, g, b))?; i as i32 }
        else if fail_if_bad { -1 }
        else {
            let near = (0..256).filter(|&i| p.refs[i] > 0 && !p.exclusive[i]).min_by_key(|&i| dist(p.rgb[i], r, g, b));
            match near { Some(i) => { p.refs[i] += 1; i as i32 } None => -1 }
        })
}

/// FindColor(a3 = cm, d1..d3 = r g b, d4 = maxpen): the closest pen in 0..=maxpen.
pub fn find_color<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (r, g, b, max) = (cpu.dreg(1) >> 24, cpu.dreg(2) >> 24, cpu.dreg(3) >> 24, cpu.dreg(4) as i32);
    let best = closest(a, r, g, b, max);
    cpu.set_d0(best as u32);
    Ok(format!("[graphics] FindColor {r:02x}{g:02x}{b:02x} max {max} -> {best}"))
}

/// The closest pen (8-bit components) in 0..=max (max outside 0..=255 means all).
pub fn closest<U: GuestUi>(a: &mut Amiga<U>, r: u32, g: u32, b: u32, max: i32) -> usize {
    let hi = if !(0..=255).contains(&max) { 255 } else { max as usize };
    let p = a.gfx.palette();
    (0..=hi).min_by_key(|&i| dist(p.rgb[i], r, g, b)).unwrap_or(0)
}
