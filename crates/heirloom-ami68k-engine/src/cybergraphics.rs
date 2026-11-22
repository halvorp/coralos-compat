//! `cybergraphics.library` (CyberGraphX RTG): 8-bit chunky RTG modes, `BestCModeIDTagList`,
//! `IsCyberModeID`, `GetCyberIDAttr`, `GetCyberMapAttr`, `WritePixelArray`, `WriteLUTPixelArray`,
//! and `LockBitMapTagList` / `UnLockBitMap`, routed straight into the engine's `graphics` bitmaps
//! and `LoadRGB32` palette pipeline (`graphics/content.rs`).
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_raster::Rect;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use crate::graphics::target;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{tags, Refusal};

/// Base mask for CyberGraphX RTG DisplayIDs created by this engine.
pub const CYBER_MODE_PREFIX: u32 = 0x5000_0000;
pub const CYBER_MODE_MASK: u32 = 0xffff_0000;

// BestCModeIDTagList tags (cybergraphx/cybergraphics.h)
const CYBRBIDTG_DEPTH: u32 = tags::TAG_USER + 0x50000;
const CYBRBIDTG_NOMINALWIDTH: u32 = tags::TAG_USER + 0x50001;
const CYBRBIDTG_NOMINALHEIGHT: u32 = tags::TAG_USER + 0x50002;

// GetCyberIDAttr attributes
const CYBRIDATTR_PIXFMT: u32 = 0x8000_0001;
const CYBRIDATTR_WIDTH: u32 = 0x8000_0002;
const CYBRIDATTR_HEIGHT: u32 = 0x8000_0003;
const CYBRIDATTR_DEPTH: u32 = 0x8000_0004;
const CYBRIDATTR_BPPIX: u32 = 0x8000_0005;

// GetCyberMapAttr attributes
const CYBRMATTR_XMOD: u32 = 0x8000_0001;
const CYBRMATTR_BPPIX: u32 = 0x8000_0002;
const CYBRMATTR_DISPADR: u32 = 0x8000_0003;
const CYBRMATTR_PIXFMT: u32 = 0x8000_0004;
const CYBRMATTR_WIDTH: u32 = 0x8000_0005;
const CYBRMATTR_HEIGHT: u32 = 0x8000_0006;
const CYBRMATTR_DEPTH: u32 = 0x8000_0007;
const CYBRMATTR_ISCYBERGFX: u32 = 0x8000_0008;
const CYBRMATTR_ISLINEARMEM: u32 = 0x8000_0009;

// LockBitMapTagList tags
const LBMI_WIDTH: u32 = 0x8400_1001;
const LBMI_HEIGHT: u32 = 0x8400_1002;
const LBMI_DEPTH: u32 = 0x8400_1003;
const LBMI_PIXFMT: u32 = 0x8400_1004;
const LBMI_BYTESPERPIX: u32 = 0x8400_1005;
const LBMI_BYTESPERROW: u32 = 0x8400_1006;
const LBMI_BASEADDRESS: u32 = 0x8400_1007;

const PIXFMT_LUT8: u32 = 0;
const RECTFMT_LUT8: u32 = 3;
const RECTFMT_GREY8: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub struct CyberMode {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

#[derive(Clone, Copy, Debug)]
struct LockedBitMap {
    bitmap: u32,
    buf: u32,
    width: u32,
    height: u32,
}

#[derive(Default)]
pub struct State {
    modes: BTreeMap<u32, CyberMode>,
    next_mode: u32,
    /// Persistent linear chunky shadow buffer per guest BitMap address (reused across frames).
    buffers: BTreeMap<u32, (u32, u32, u32)>, // bitmap -> (guest_buf, w, h)
    locks: BTreeMap<u32, LockedBitMap>,
    next_lock: u32,
}

impl State {
    pub(crate) fn mode_info(&self, id: u32) -> CyberMode {
        self.modes.get(&id).copied().unwrap_or(CyberMode { width: 320, height: 200, depth: 8 })
    }

    fn alloc_mode(&mut self, width: u32, height: u32, depth: u32) -> u32 {
        for (&id, m) in &self.modes {
            if m.width == width && m.height == height && m.depth == depth {
                return id;
            }
        }
        let id = crate::asl::DEFAULT_RTG_DISPLAY_ID + self.next_mode;
        self.next_mode += 1;
        self.modes.insert(id, CyberMode { width, height, depth });
        id
    }
}

pub fn is_cyber_mode_id(id: u32) -> bool {
    (id & CYBER_MODE_MASK) == CYBER_MODE_PREFIX
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("IsCyberModeID", is_cyber_mode::<U>)
        .named("BestCModeIDTagList", best_cmode_id::<U>)
        .named("GetCyberIDAttr", get_cyber_id_attr::<U>)
        .named("GetCyberMapAttr", get_cyber_map_attr::<U>)
        .named("WritePixelArray", write_pixel_array::<U>)
        .named("WriteLUTPixelArray", write_lut_pixel_array::<U>)
        .named("LockBitMapTagList", lock_bitmap::<U>)
        .named("UnLockBitMap", unlock_bitmap::<U>)
        .named("UnLockBitMapTagList", unlock_bitmap::<U>)
}

fn is_cyber_mode<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let id = cpu.dreg(0);
    let yes = is_cyber_mode_id(id);
    cpu.set_d0(u32::from(yes));
    Ok(format!("[cybergraphics] IsCyberModeID({id:#x}) -> {}", if yes { "TRUE" } else { "FALSE" }))
}

fn best_cmode_id<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let t = tags::walk(&a.mem, cpu.areg(0))?;
    let depth = tags::get(&t, CYBRBIDTG_DEPTH).filter(|&v| v > 0).unwrap_or(8);
    let width = tags::get(&t, CYBRBIDTG_NOMINALWIDTH).filter(|&v| v > 0).unwrap_or(320);
    let height = tags::get(&t, CYBRBIDTG_NOMINALHEIGHT).filter(|&v| v > 0).unwrap_or(200);
    let id = a.cyber.alloc_mode(width, height, depth.min(8));
    cpu.set_d0(id);
    Ok(format!("[cybergraphics] BestCModeIDTagList({width}x{height}x{depth}) -> {id:#x}"))
}

fn get_cyber_id_attr<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (attr, id) = (cpu.dreg(0), cpu.dreg(1));
    let m = a.cyber.mode_info(id);
    let v = match attr {
        CYBRIDATTR_PIXFMT => PIXFMT_LUT8,
        CYBRIDATTR_WIDTH => m.width,
        CYBRIDATTR_HEIGHT => m.height,
        CYBRIDATTR_DEPTH => m.depth,
        CYBRIDATTR_BPPIX => 1,
        _ => return Err(Refusal(format!("GetCyberIDAttr: unknown attr {attr:#x}"))),
    };
    cpu.set_d0(v);
    Ok(format!("[cybergraphics] GetCyberIDAttr({attr:#x}, {id:#x}) -> {v}"))
}

fn bitmap_dims<U: GuestUi>(a: &Amiga<U>, bm: u32) -> Result<(u32, u32, u32), Refusal> {
    if let Some(gb) = a.gfx.bitmaps.get(&bm) {
        return Ok((gb.bm.width() as u32, gb.bm.height() as u32, 8));
    }
    let bpr = u32::from(a.mem.read_u16(bm).map_err(Refusal::mem("bm_BytesPerRow"))?);
    let rows = u32::from(a.mem.read_u16(bm + 2).map_err(Refusal::mem("bm_Rows"))?);
    let depth = u32::from(a.mem.read_u8(bm + 5).map_err(Refusal::mem("bm_Depth"))?);
    Ok(((bpr * 8).max(1), rows.max(1), depth.max(8)))
}

fn get_cyber_map_attr<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (bm, attr) = (cpu.areg(0), cpu.dreg(0));
    let (w, h, d) = bitmap_dims(a, bm)?;
    let v = match attr {
        CYBRMATTR_XMOD => w,
        CYBRMATTR_BPPIX => 1,
        CYBRMATTR_DISPADR => a.cyber.buffers.get(&bm).map(|&(b, _, _)| b).unwrap_or(0),
        CYBRMATTR_PIXFMT => PIXFMT_LUT8,
        CYBRMATTR_WIDTH => w,
        CYBRMATTR_HEIGHT => h,
        CYBRMATTR_DEPTH => d,
        CYBRMATTR_ISCYBERGFX => 1,
        CYBRMATTR_ISLINEARMEM => 1,
        _ => return Err(Refusal(format!("GetCyberMapAttr: unknown attr {attr:#x}"))),
    };
    cpu.set_d0(v);
    Ok(format!("[cybergraphics] GetCyberMapAttr({bm:#x}, {attr:#x}) -> {v}"))
}

fn blit_lut8<U: GuestUi>(
    a: &mut Amiga<U>,
    src: u32,
    src_x: i32,
    src_y: i32,
    src_mod: i32,
    rp: u32,
    dst_x: i32,
    dst_y: i32,
    w: i32,
    h: i32,
    what: &str,
) -> Result<u32, Refusal> {
    if w <= 0 || h <= 0 {
        return Ok(0);
    }
    let mut rows = Vec::with_capacity((w * h) as usize);
    for j in 0..h {
        let base = src.wrapping_add(((src_y + j) * src_mod + src_x) as u32);
        for i in 0..w {
            rows.push(a.mem.read_u8(base + i as u32).map_err(Refusal::mem("WritePixelArray src"))?);
        }
    }
    target::paint(a, rp, what, |bm, clip, (ox, oy)| {
        let mut touched = Rect::default();
        for j in 0..h {
            let py = dst_y + j - oy;
            for i in 0..w {
                let px = dst_x + i - ox;
                if !clip.allows(px, py) || bm.get(px, py).is_none() {
                    continue;
                }
                let idx = (j * w + i) as usize;
                bm.set(px, py, u32::from(rows[idx]));
                touched = touched.union(&Rect::new(px, py, px + 1, py + 1));
            }
        }
        touched
    })?;
    Ok((w * h) as u32)
}

/// WritePixelArray(a0=srcRect, d0=srcX, d1=srcY, d2=srcMod, a1=rp, d3=destX, d4=destY, d5=sizeX, d6=sizeY, d7=srcFormat)
fn write_pixel_array<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (src, rp) = (cpu.areg(0), cpu.areg(1));
    let (sx, sy, smod) = (cpu.dreg(0) as u16 as i32, cpu.dreg(1) as u16 as i32, cpu.dreg(2) as u16 as i32);
    let (dx, dy, w, h, fmt) = (
        cpu.dreg(3) as u16 as i32,
        cpu.dreg(4) as u16 as i32,
        cpu.dreg(5) as u16 as i32,
        cpu.dreg(6) as u16 as i32,
        cpu.dreg(7),
    );
    if fmt != RECTFMT_LUT8 && fmt != RECTFMT_GREY8 {
        return Err(Refusal(format!("WritePixelArray: unsupported srcFormat {fmt} (only LUT8/GREY8 implemented)")));
    }
    let n = blit_lut8(a, src, sx, sy, smod, rp, dx, dy, w, h, "WritePixelArray")?;
    cpu.set_d0(n);
    Ok(format!("[cybergraphics] WritePixelArray {w}x{h} -> {rp:#x} ({dx},{dy}) fmt={fmt}"))
}

/// WriteLUTPixelArray(a0=srcRect, d0=srcX, d1=srcY, d2=srcMod, a1=rp, a2=colorTab, d3=destX, d4=destY, d5=sizeX, d6=sizeY, d7=ctFormat)
fn write_lut_pixel_array<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (src, rp, ctab) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let (sx, sy, smod) = (cpu.dreg(0) as u16 as i32, cpu.dreg(1) as u16 as i32, cpu.dreg(2) as u16 as i32);
    let (dx, dy, w, h) = (
        cpu.dreg(3) as u16 as i32,
        cpu.dreg(4) as u16 as i32,
        cpu.dreg(5) as u16 as i32,
        cpu.dreg(6) as u16 as i32,
    );
    if ctab != 0 {
        for i in 0..256usize {
            let rgb = a.mem.read_u32(ctab + (i as u32) * 4).map_err(Refusal::mem("WriteLUTPixelArray colorTab"))? & 0x00ff_ffff;
            a.gfx.palette().rgb[i] = rgb;
        }
    }
    let n = blit_lut8(a, src, sx, sy, smod, rp, dx, dy, w, h, "WriteLUTPixelArray")?;
    cpu.set_d0(n);
    Ok(format!("[cybergraphics] WriteLUTPixelArray {w}x{h} -> {rp:#x} ({dx},{dy})"))
}

fn lock_bitmap<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (bm, tag_list) = (cpu.areg(0), cpu.areg(1));
    let (w, h, d) = bitmap_dims(a, bm)?;
    let buf = match a.cyber.buffers.get(&bm).copied() {
        Some((b, bw, bh)) if bw == w && bh == h => b,
        _ => {
            let size = (w * h).max(1);
            let Some(b) = a.heap.alloc(size) else {
                cpu.set_d0(0);
                return Ok("[cybergraphics] LockBitMapTagList -> NULL (out of heap)".into());
            };
            a.mem.zero(b, size).map_err(Refusal::mem("LockBitMap buffer"))?;
            a.cyber.buffers.insert(bm, (b, w, h));
            b
        }
    };
    let t = tags::walk(&a.mem, tag_list)?;
    for (tag, val) in [
        (LBMI_WIDTH, w),
        (LBMI_HEIGHT, h),
        (LBMI_DEPTH, d),
        (LBMI_PIXFMT, PIXFMT_LUT8),
        (LBMI_BYTESPERPIX, 1),
        (LBMI_BYTESPERROW, w),
        (LBMI_BASEADDRESS, buf),
    ] {
        if let Some(ptr) = tags::get(&t, tag) {
            if ptr != 0 {
                a.mem.write_u32(ptr, val).map_err(Refusal::mem("LockBitMap out"))?;
            }
        }
    }
    a.cyber.next_lock += 1;
    let handle = 0x5c00_0000 | a.cyber.next_lock;
    a.cyber.locks.insert(handle, LockedBitMap { bitmap: bm, buf, width: w, height: h });
    cpu.set_d0(handle);
    Ok(format!("[cybergraphics] LockBitMapTagList {bm:#x} ({w}x{h}) -> handle {handle:#x} buf {buf:#x}"))
}

fn unlock_bitmap<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let handle = cpu.areg(0);
    let Some(lock) = a.cyber.locks.remove(&handle) else {
        return Err(Refusal(format!("UnLockBitMap: {handle:#x} is not a live LockBitMap handle")));
    };
    let n = (lock.width * lock.height) as usize;
    let mut pixels = Vec::with_capacity(n);
    for i in 0..n {
        pixels.push(a.mem.read_u8(lock.buf + i as u32).map_err(Refusal::mem("UnLockBitMap buf"))?);
    }
    if let Some(gb) = a.gfx.bitmaps.get_mut(&lock.bitmap) {
        let bw = gb.bm.width().min(lock.width as i32);
        let bh = gb.bm.height().min(lock.height as i32);
        for y in 0..bh {
            let row = (y as u32 * lock.width) as usize;
            for x in 0..bw {
                gb.bm.set(x, y, u32::from(pixels[row + x as usize]));
            }
        }
        let rect = Rect::new(0, 0, bw, bh);
        let wins: Vec<u32> = a.gfx.layers.values()
            .filter(|l| l.canvas == lock.bitmap)
            .map(|l| l.window)
            .collect();
        for w in wins {
            a.gfx.add_damage(w, rect);
        }
    }
    Ok(format!("[cybergraphics] UnLockBitMap {handle:#x} ({}x{})", lock.width, lock.height))
}
