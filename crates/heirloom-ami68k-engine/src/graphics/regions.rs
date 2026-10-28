//! Regions: a guest `struct Region` (bounds + a RegionRectangle chain, rebuilt after every operation so a program that
//! walks it sees the pixel set) over a host-side pixel set that does the set algebra. Plus layers.library's clip-region
//! install, which is how a program restricts its window drawing to a region.
//!
//! Operand order follows graphics.library: the *RegionRegion calls change their SECOND region (R2 = R2 op R1);
//! ClearRegionRegion removes R1's area from R2.
use super::layout::*;
use super::state::GRegion;
use crate::{machine::Amiga, memory, Refusal};
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::{Rect, Region};

fn get<U: GuestUi>(a: &Amiga<U>, r: u32, what: &str) -> Result<(), Refusal> {
    if a.gfx.regions.contains_key(&r) { Ok(()) } else { Err(Refusal(format!("{what}: {r:#x} is not a Region from NewRegion"))) }
}

/// Rebuild the guest-visible Region from the host set: bounds, then one RegionRectangle per slab rectangle, linked,
/// coordinates relative to the bounds' top-left.
fn sync<U: GuestUi>(a: &mut Amiga<U>, rg: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("Region");
    let g = a.gfx.regions.get_mut(&rg).ok_or_else(|| Refusal(format!("Region {rg:#x} vanished")))?;
    let (rects, old) = (g.region.rects().to_vec(), g.chain.take());
    if let Some((p, n)) = old { a.heap.free(p, n).map_err(|e| Refusal(format!("Region chain: {e}")))?; }
    let b = rects.iter().fold(Rect::default(), |x, r| x.union(r));
    for (i, v) in [b.x0, b.y0, b.x1 - 1, b.y1 - 1].into_iter().enumerate() { a.mem.write_u16(rg + RG_BOUNDS + 2 * i as u32, (if rects.is_empty() { 0 } else { v }) as u16).map_err(&m)?; }
    if rects.is_empty() { return a.mem.write_u32(rg + RG_FIRST, 0).map_err(&m); }
    let size = RR_SIZE * rects.len() as u32;
    let blk = memory::alloc_zeroed(a, size, "Region chain")?;
    for (i, r) in rects.iter().enumerate() {
        let at = blk + RR_SIZE * i as u32;
        a.mem.write_u32(at + RR_NEXT, if i + 1 < rects.len() { at + RR_SIZE } else { 0 }).map_err(&m)?;
        a.mem.write_u32(at + RR_PREV, if i > 0 { at - RR_SIZE } else { 0 }).map_err(&m)?;
        for (k, v) in [r.x0 - b.x0, r.y0 - b.y0, r.x1 - 1 - b.x0, r.y1 - 1 - b.y0].into_iter().enumerate() { a.mem.write_u16(at + RR_BOUNDS + 2 * k as u32, v as u16).map_err(&m)?; }
    }
    a.mem.write_u32(rg + RG_FIRST, blk).map_err(&m)?;
    a.gfx.regions.get_mut(&rg).unwrap().chain = Some((blk, size));
    Ok(())
}

pub fn new_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rg = memory::alloc_zeroed(a, RG_SIZE, "NewRegion")?;
    a.gfx.regions.insert(rg, GRegion { region: Region::new(), chain: None });
    cpu.set_d0(rg);
    Ok(format!("[graphics] NewRegion -> {rg:#x}"))
}

pub fn dispose_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rg = cpu.areg(0);
    if rg == 0 { return Ok("[graphics] DisposeRegion NULL".into()); }
    get(a, rg, "DisposeRegion")?;
    let g = a.gfx.regions.remove(&rg).unwrap();
    if let Some((p, n)) = g.chain { a.heap.free(p, n).map_err(|e| Refusal(format!("DisposeRegion: {e}")))?; }
    a.heap.free(rg, RG_SIZE).map_err(|e| Refusal(format!("DisposeRegion: {e}")))?;
    Ok(format!("[graphics] DisposeRegion {rg:#x}"))
}

pub fn clear_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rg = cpu.areg(0);
    get(a, rg, "ClearRegion")?;
    a.gfx.regions.get_mut(&rg).unwrap().region.clear();
    sync(a, rg)?;
    Ok(format!("[graphics] ClearRegion {rg:#x}"))
}

fn read_rect<U: GuestUi>(a: &Amiga<U>, p: u32) -> Result<Rect, Refusal> {
    let m = Refusal::mem("Rectangle");
    let r = |o| a.mem.read_i16(p + o).map(i32::from).map_err(&m);
    Ok(Rect::from_inclusive(r(0)?, r(2)?, r(4)?, r(6)?))
}

#[derive(Clone, Copy)]
enum Op { Or, And, Xor, Clear }

fn rect_op<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str, op: Op) -> Result<String, Refusal> {
    let (rg, rp) = (cpu.areg(0), cpu.areg(1));
    get(a, rg, name)?;
    let r = read_rect(a, rp)?;
    let g = &mut a.gfx.regions.get_mut(&rg).unwrap().region;
    match op { Op::Or => g.or_rect(r), Op::And => g.and_rect(r), Op::Xor => g.xor_rect(r), Op::Clear => g.clear_rect(r) }
    sync(a, rg)?;
    cpu.set_d0(1);
    Ok(format!("[graphics] {name} {rg:#x} ({},{})-({},{})", r.x0, r.y0, r.x1 - 1, r.y1 - 1))
}
pub fn or_rect_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { rect_op(a, cpu, "OrRectRegion", Op::Or) }
pub fn and_rect_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { rect_op(a, cpu, "AndRectRegion", Op::And) }
pub fn xor_rect_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { rect_op(a, cpu, "XorRectRegion", Op::Xor) }
pub fn clear_rect_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { rect_op(a, cpu, "ClearRectRegion", Op::Clear) }

fn region_op<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str, op: Op) -> Result<String, Refusal> {
    let (r1, r2) = (cpu.areg(0), cpu.areg(1));
    get(a, r1, name)?; get(a, r2, name)?;
    let first = a.gfx.regions[&r1].region.clone();
    let g = &mut a.gfx.regions.get_mut(&r2).unwrap().region;
    match op { Op::Or => g.or_region(&first), Op::And => g.and_region(&first), Op::Xor => g.xor_region(&first), Op::Clear => g.clear_region(&first) }
    sync(a, r2)?;
    cpu.set_d0(1);
    Ok(format!("[graphics] {name} {r1:#x} into {r2:#x}"))
}
pub fn or_region_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { region_op(a, cpu, "OrRegionRegion", Op::Or) }
pub fn and_region_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { region_op(a, cpu, "AndRegionRegion", Op::And) }
pub fn xor_region_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { region_op(a, cpu, "XorRegionRegion", Op::Xor) }
pub fn clear_region_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { region_op(a, cpu, "ClearRegionRegion", Op::Clear) }

/// IsPointInRegion(a0 = region, d0 = x, d1 = y).
pub fn is_point_in_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rg, x, y) = (cpu.areg(0), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    get(a, rg, "IsPointInRegion")?;
    let v = a.gfx.regions[&rg].region.contains(x, y);
    cpu.set_d0(v as u32);
    Ok(format!("[graphics] IsPointInRegion {rg:#x} {x},{y} -> {v}"))
}

/// layers.library InstallClipRegion(a0 = layer, a1 = region) -> the previously installed region (or NULL). The region
/// is in the layer's coordinates; the caller keeps ownership of both.
pub fn install_clip_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (layer, rg) = (cpu.areg(0), cpu.areg(1));
    if !a.gfx.layers.contains_key(&layer) { return Err(Refusal(format!("InstallClipRegion: {layer:#x} is not one of this engine's layers"))); }
    if rg != 0 { get(a, rg, "InstallClipRegion")?; }
    let old = a.mem.read_u32(layer + LY_CLIP_REGION).map_err(Refusal::mem("Layer.ClipRegion"))?;
    a.mem.write_u32(layer + LY_CLIP_REGION, rg).map_err(Refusal::mem("Layer.ClipRegion"))?;
    cpu.set_d0(old);
    Ok(format!("[layers] InstallClipRegion layer {layer:#x} region {rg:#x} (was {old:#x})"))
}

const LY_DAMAGE_LIST: u32 = 156;

/// BeginRefresh(a0 = window): the layer's DamageList becomes a real Region covering the whole window area (the engine's canvas is
/// never partially damaged), so a program that ANDs its own region with `window->WLayer->DamageList` (DOpus's refresh) has one to
/// use. It is in screen coordinates, the layer's bounds. EndRefresh disposes it.
pub fn begin_refresh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let win = cpu.areg(0);
    let m = Refusal::mem("Layer");
    let layer = a.mem.read_u32(win + WD_WLAYER).map_err(&m)?;
    if layer == 0 || !a.gfx.layers.contains_key(&layer) { cpu.set_d0(1); return Ok("[intuition] BeginRefresh: no engine layer (nothing to damage)".into()); }
    if a.mem.read_u32(layer + LY_DAMAGE_LIST).map_err(&m)? != 0 { cpu.set_d0(1); return Ok(format!("[intuition] BeginRefresh {win:#x}: DamageList already set")); }
    let b: [i32; 4] = [0, 1, 2, 3].map(|i| a.mem.read_u16(layer + LY_BOUNDS + 2 * i).map(|v| v as i16 as i32).unwrap_or(0));
    let rg = damage_whole(a, layer, b)?;
    cpu.set_d0(1);
    Ok(format!("[intuition] BeginRefresh {win:#x}: DamageList {rg:#x} covers the whole window"))
}

/// Give `layer` a DamageList Region covering `b` (screen coordinates, inclusive).
fn damage_whole<U: GuestUi>(a: &mut Amiga<U>, layer: u32, b: [i32; 4]) -> Result<u32, Refusal> {
    let rg = memory::alloc_zeroed(a, RG_SIZE, "DamageList")?;
    let mut region = Region::new();
    region.or_rect(Rect::from_inclusive(b[0], b[1], b[2], b[3]));
    a.gfx.regions.insert(rg, GRegion { region, chain: None });
    sync(a, rg)?;
    a.mem.write_u32(layer + LY_DAMAGE_LIST, rg).map_err(Refusal::mem("Layer"))?;
    Ok(rg)
}

/// A window that is about to be told IDCMP_REFRESHWINDOW has its whole area damaged: its layer's DamageList is a real Region.
pub fn mark_damaged<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("Layer");
    let layer = a.mem.read_u32(win + WD_WLAYER).map_err(&m)?;
    if layer == 0 || !a.gfx.layers.contains_key(&layer) || a.mem.read_u32(layer + LY_DAMAGE_LIST).map_err(&m)? != 0 { return Ok(()); }
    let b: [i32; 4] = [0, 1, 2, 3].map(|i| a.mem.read_u16(layer + LY_BOUNDS + 2 * i).map(|v| v as i16 as i32).unwrap_or(0));
    damage_whole(a, layer, b).map(|_| ())
}

pub(crate) fn end_refresh_of<U: GuestUi>(a: &mut Amiga<U>, layer: u32) -> Result<(), Refusal> {
    let rg = a.mem.read_u32(layer + LY_DAMAGE_LIST).map_err(Refusal::mem("Layer"))?;
    if rg != 0 {
        if let Some(g) = a.gfx.regions.remove(&rg) {
            if let Some((p, n)) = g.chain { a.heap.free(p, n).map_err(|e| Refusal(format!("DamageList chain: {e}")))?; }
            a.heap.free(rg, RG_SIZE).map_err(|e| Refusal(format!("DamageList: {e}")))?;
        }
        a.mem.write_u32(layer + LY_DAMAGE_LIST, 0).map_err(Refusal::mem("Layer"))?;
    }
    Ok(())
}

/// EndRefresh(a0 = window, d0 = complete).
pub fn end_refresh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let win = cpu.areg(0);
    let layer = a.mem.read_u32(win + WD_WLAYER).map_err(Refusal::mem("Layer"))?;
    if layer != 0 && a.gfx.layers.contains_key(&layer) { end_refresh_of(a, layer)?; }
    Ok(format!("[intuition] EndRefresh {win:#x}"))
}

/// CloseWindow: give back a DamageList nobody refreshed (mark_damaged made it; the program never ran BeginRefresh/EndRefresh).
pub fn release_damage<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    let layer = a.mem.read_u32(win + WD_WLAYER).map_err(Refusal::mem("Layer"))?;
    if layer != 0 && a.gfx.layers.contains_key(&layer) { end_refresh_of(a, layer)?; }
    Ok(())
}
