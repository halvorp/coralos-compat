//! BitMaps: a guest `struct BitMap` (real: BytesPerRow, Rows, Flags, Depth, Planes[8]) in guest memory, backed by a
//! host-side CHUNKY bitmap of pen indices. QUALIFIED LAYOUT: chunky, depth 1..=8. The struct's Planes[] are NULL: the
//! pixels do not live in guest planes, so a program that reads or writes plane memory directly sees nothing (it is
//! not mirrored; a plane store is not observed). EXCEPTION: a BitMap whose Planes[] point at guest memory (a program's own
//! planar data, e.g. a DiskObject's icon Image through InitBitMap) is a blit SOURCE read from those planes ([`ensure_source`]). Programs that go through graphics.library calls, which is nearly all
//! of them, never notice. AllocRaster/FreeRaster still hand out real guest memory for programs that build their own
//! planes; a BitMap they InitBitMap is adopted by its Rows/BytesPerRow/Depth on first use.
use super::layout::*;
use super::state::GBitmap;
use crate::{machine::Amiga, memory, Refusal};
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::Bitmap;

const BMF_CLEAR: u32 = 1;
const BMF_MINPLANES: u32 = 16;

pub fn bytes_per_row(w: i32) -> u32 { ((w as u32 + 15) >> 4) << 1 }

/// Fill a guest BitMap struct for a w x h x depth bitmap (planes NULL).
pub fn write_struct<U: GuestUi>(a: &mut Amiga<U>, bm: u32, w: i32, h: i32, depth: u8) -> Result<(), Refusal> {
    let m = Refusal::mem("BitMap");
    a.mem.zero(bm, BM_SIZE).map_err(&m)?;
    a.mem.write_u16(bm + BM_BYTES_PER_ROW, bytes_per_row(w) as u16).map_err(&m)?;
    a.mem.write_u16(bm + BM_ROWS, h as u16).map_err(&m)?;
    a.mem.write_u8(bm + BM_DEPTH, depth).map_err(&m)
}

/// [`ensure`] for a blit SOURCE: when the struct's Planes[] hold guest pointers (not NULL, not -1) the pixels live in guest planes
/// the program may have changed, so the host bitmap is rebuilt from them first (plane i = bit i of the pen; a NULL plane reads as
/// 0 and a -1 plane as all ones, graphics.library's convention).
pub fn ensure_source<U: GuestUi>(a: &mut Amiga<U>, bm: u32) -> Result<(), Refusal> {
    ensure(a, bm)?;
    let m = Refusal::mem("BitMap");
    let depth = a.mem.read_u8(bm + BM_DEPTH).map_err(&m)? as u32;
    let mut planes = alloc::vec::Vec::new();
    for i in 0..depth.min(8) { planes.push(a.mem.read_u32(bm + BM_PLANES + 4 * i).map_err(&m)?); }
    if !planes.iter().any(|&p| p != 0 && p != u32::MAX) { return Ok(()); }
    let (bpr, rows) = (a.mem.read_u16(bm + BM_BYTES_PER_ROW).map_err(&m)? as u32, a.mem.read_u16(bm + BM_ROWS).map_err(&m)? as u32);
    let mut pens = alloc::vec![0u8; (bpr * 8 * rows) as usize];
    for (i, &p) in planes.iter().enumerate() {
        if p == 0 { continue; }
        for y in 0..rows {
            for xb in 0..bpr {
                let byte = if p == u32::MAX { 0xFF } else { a.mem.read_u8(p + y * bpr + xb).map_err(&m)? };
                for bit in 0..8u32 { if byte & (0x80 >> bit) != 0 { pens[(y * bpr * 8 + xb * 8 + bit) as usize] |= 1 << i; } }
            }
        }
    }
    let g = a.gfx.bitmaps.get_mut(&bm).expect("ensured");
    for y in 0..rows as i32 { for x in 0..(bpr * 8) as i32 { g.bm.set(x, y, pens[(y as u32 * bpr * 8 + x as u32) as usize]); } }
    Ok(())
}

/// The other direction: after the engine drew into a bitmap whose Planes[] are real guest pointers, write the pixels back to the planes
/// (plane i = bit i of the pen), so the guest reads what was drawn (DOpus's image remap writes with WritePixelLine8 and then draws
/// from the planes). A NULL or -1 plane is skipped.
pub fn export_planes<U: GuestUi>(a: &mut Amiga<U>, bm: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("BitMap");
    let depth = a.mem.read_u8(bm + BM_DEPTH).map_err(&m)? as u32;
    let mut planes = alloc::vec::Vec::new();
    for i in 0..depth.min(8) { planes.push(a.mem.read_u32(bm + BM_PLANES + 4 * i).map_err(&m)?); }
    if !planes.iter().any(|&p| p != 0 && p != u32::MAX) { return Ok(()); }
    let (bpr, rows) = (a.mem.read_u16(bm + BM_BYTES_PER_ROW).map_err(&m)? as u32, a.mem.read_u16(bm + BM_ROWS).map_err(&m)? as u32);
    let Some(g) = a.gfx.bitmaps.get(&bm) else { return Ok(()) };
    let mut bytes = alloc::vec![0u8; (bpr * rows) as usize];
    for (i, &p) in planes.iter().enumerate() {
        if p == 0 || p == u32::MAX { continue; }
        for y in 0..rows as i32 { for xb in 0..bpr as i32 {
            let mut byte = 0u8;
            for bit in 0..8 { if g.bm.get(xb * 8 + bit, y).unwrap_or(0) & (1 << i) != 0 { byte |= 0x80 >> bit; } }
            bytes[(y as u32 * bpr + xb as u32) as usize] = byte;
        } }
        a.mem.write_bytes(p, &bytes).map_err(&m)?;
    }
    Ok(())
}

/// A new guest BitMap struct plus its host bitmap; returns the struct's address.
pub fn create<U: GuestUi>(a: &mut Amiga<U>, w: i32, h: i32, depth: u8, window: Option<u32>) -> Result<u32, Refusal> {
    let host = Bitmap::new(w, h, depth).ok_or_else(|| Refusal(format!("BitMap {w}x{h}x{depth}: unsupported size or depth (chunky 1..=8 bits)")))?;
    let bm = memory::alloc_zeroed(a, BM_SIZE, "BitMap struct")?;
    write_struct(a, bm, w, h, depth)?;
    a.gfx.bitmaps.insert(bm, GBitmap { bm: host, window });
    Ok(bm)
}

/// Make sure the host side of the guest BitMap at `bm` exists, adopting a program-built (InitBitMap) struct.
pub fn ensure<U: GuestUi>(a: &mut Amiga<U>, bm: u32) -> Result<(), Refusal> {
    if a.gfx.bitmaps.contains_key(&bm) { return Ok(()); }
    let m = Refusal::mem("BitMap");
    let (bpr, rows, depth) = (a.mem.read_u16(bm + BM_BYTES_PER_ROW).map_err(&m)? as i32, a.mem.read_u16(bm + BM_ROWS).map_err(&m)? as i32, a.mem.read_u8(bm + BM_DEPTH).map_err(&m)?);
    let host = Bitmap::new(bpr * 8, rows, depth).ok_or_else(|| Refusal(format!("BitMap {bm:#x}: BytesPerRow {bpr} Rows {rows} Depth {depth} is not a usable bitmap (chunky 1..=8 bits)")))?;
    a.gfx.bitmaps.insert(bm, GBitmap { bm: host, window: None });
    Ok(())
}

pub fn alloc_bit_map<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (w, h, mut depth, flags, friend) = (cpu.dreg(0), cpu.dreg(1), cpu.dreg(2), cpu.dreg(3), cpu.areg(0));
    if flags & BMF_MINPLANES == 0 && friend > 0x100 && friend < 0x100_0000 {
        if let Ok(fd) = a.mem.read_u8(friend + BM_DEPTH) { depth = depth.max(fd as u32); }
    }
    if depth == 0 || depth > 8 { return Err(Refusal(format!("AllocBitMap depth {depth}: only chunky 1..=8 bits are qualified"))); }
    let bm = create(a, w as i32, h as i32, depth as u8, None)?;
    if flags & BMF_CLEAR != 0 { /* host bitmaps start zeroed */ }
    cpu.set_d0(bm);
    Ok(format!("[graphics] AllocBitMap {w}x{h}x{depth} flags={flags:#x} -> {bm:#x}"))
}

pub fn free_bit_map<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let bm = cpu.areg(0);
    if bm == 0 { return Ok("[graphics] FreeBitMap NULL".into()); }
    if a.gfx.bitmaps.get(&bm).map_or(false, |g| g.window.is_some()) { return Err(Refusal(format!("FreeBitMap {bm:#x}: it is a window's canvas"))); }
    if a.gfx.bitmaps.remove(&bm).is_none() { return Err(Refusal(format!("FreeBitMap {bm:#x}: not a bitmap AllocBitMap returned"))); }
    a.heap.free(bm, BM_SIZE).map_err(|e| Refusal(format!("FreeBitMap: {e}")))?;
    Ok(format!("[graphics] FreeBitMap {bm:#x}"))
}

/// InitBitMap(a0 = bm, d0 = depth, d1 = width, d2 = height).
pub fn init_bit_map<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (bm, depth, w, h) = (cpu.areg(0), cpu.dreg(0) as u8, cpu.dreg(1) as u16 as i32, cpu.dreg(2) as u16 as i32);
    write_struct(a, bm, w, h, depth)?;
    a.gfx.bitmaps.remove(&bm);
    Ok(format!("[graphics] InitBitMap {bm:#x} {w}x{h}x{depth}"))
}

pub fn alloc_raster<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (w, h) = (cpu.dreg(0) as u16 as i32, cpu.dreg(1) as u16 as i32);
    let size = bytes_per_row(w) * h as u32;
    let p = if size == 0 { 0 } else { memory::alloc_zeroed(a, size, "AllocRaster")? };
    cpu.set_d0(p);
    Ok(format!("[graphics] AllocRaster {w}x{h} -> {p:#x}"))
}

pub fn free_raster<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (p, w, h) = (cpu.areg(0), cpu.dreg(0) as u16 as i32, cpu.dreg(1) as u16 as i32);
    if p != 0 { a.heap.free(p, bytes_per_row(w) * h as u32).map_err(|e| Refusal(format!("FreeRaster: {e}")))?; }
    Ok(format!("[graphics] FreeRaster {p:#x}"))
}

// BMA_ attributes (graphics/gfx.h).
pub fn get_bit_map_attr<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (bm, attr) = (cpu.areg(0), cpu.dreg(1));
    ensure(a, bm)?;
    let g = &a.gfx.bitmaps[&bm].bm;
    let v = match attr { 0 => g.height() as u32, 4 => g.depth() as u32, 8 => g.width() as u32, 12 => a.mem.read_u8(bm + BM_FLAGS).map_err(Refusal::mem("BitMap.Flags"))? as u32,
        other => return Err(Refusal(format!("GetBitMapAttr: attribute {other} is not implemented"))) };
    cpu.set_d0(v);
    Ok(format!("[graphics] GetBitMapAttr {bm:#x} {attr} -> {v}"))
}

/// BltClear(a1 = block, d0 = bytecount, d1 = flags): plain memory clear (flags bit 1 = row/byte packing is refused).
pub fn blt_clear<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (p, n, flags) = (cpu.areg(1), cpu.dreg(0), cpu.dreg(1));
    if flags & 2 != 0 { return Err(Refusal("BltClear: the (rows, bytes-per-row) packed count (flags bit 1) is not implemented".into())); }
    a.mem.zero(p, n).map_err(Refusal::mem("BltClear"))?;
    Ok(format!("[graphics] BltClear {p:#x} {n} bytes"))
}
