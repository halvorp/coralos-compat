//! Offscreen graphics worlds (Inside Macintosh: Imaging With QuickDraw, "Offscreen Graphics Worlds"; `QDOffscreen.h`): `NewGWorld`, `UpdateGWorld`, `DisposeGWorld`,
//! `GetGWorld`/`SetGWorld`, `LockPixels`/`UnlockPixels`, `GetGWorldPixMap`, `GetGWorldDevice`, `GetPixBaseAddr`, `GetPixRowBytes`, `Get/SetPixelsState`, `PixMap32Bit`.
//!
//! The bodies live HERE, once, as plain methods on [`Mac`] taking and returning VALUES: the PowerPC engine binds the C names onto them (`macppc-engine/src/gworld.rs`).
//! A GWorld is exactly what the rest of Color QuickDraw already draws into: a `CGrafPort` ([`super::cport`]) over a `PixMap` whose `baseAddr` is a block of GUEST MEMORY
//! (packed 1, 2, 4 or 8 bit pixels, rows padded to 32 bits as QuickDraw does), so every drawing op and `CopyBits` between a GWorld and a window already work through
//! `qd::target` with no second code path; the window side of a `CopyBits` presents through the native path like any other drawing.
//! Not modelled, by name: DIRECT pixel depths (16 and 32 bits: the raster is indexed -- see `cqd/mod.rs`), a GWorld on a caller-supplied `GDHandle` of another device,
//! purgeable pixels (they are always resident, so `LockPixels` is TRUE and the purge flags only record the program's state), `useTempMem` (one heap).
//! The 68k `_QDExtensions` ($AB1D) selector numbers are NOT registered here: no verified table of them is in the reference tree and a guessed one would bind the wrong routine.
use super::pixmap::{PM_BASE, PM_BOUNDS, PM_ROW, PM_TABLE};
use crate::machine::Mac;
use crate::pascal::{read_rect, Rect};
use crate::qd::port::*;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_raster::Region;

/// `GWorldFlags` bits (QDOffscreen.h).
pub const PIXELS_PURGEABLE: u32 = 1 << 6;
pub const PIXELS_LOCKED: u32 = 1 << 7;
pub const GW_FLAG_ERR: u32 = 1 << 31;
pub const MEM_FULL_ERR: i16 = -108;
pub const PARAM_ERR: i16 = -50;

/// One live graphics world.
#[derive(Debug, Clone)]
pub struct Gw { pub pixmap: u32, pub device: u32, pub base: u32, pub row: u32, pub bounds: Rect, pub depth: u8, pub state: u32 }

fn row_bytes(w: i32, depth: u8) -> u32 { ((w as u32 * u32::from(depth)).div_ceil(32)) * 4 }

impl<U: GuestUi> Mac<U> {
    /// `NewGWorld(&gw, depth, &bounds, cTable, aGDevice, flags)`: the new world's port, or the QDErr. `depth` 0 = the main device's depth.
    pub fn gw_new(&mut self, depth: u16, bounds: Rect, ctab: u32, flags: u32) -> Result<Result<u32, i16>, Refusal> {
        self.init_devices()?;
        let depth = if depth == 0 { self.device_pixmap(self.cqd.main_device)?.0 as u16 } else { depth };
        if !super::pixmap::supported_depth(depth) {
            return Err(Refusal(format!("NewGWorld: {depth}-bit pixels are not implemented (indexed 1, 2, 4 and 8 bits only; direct 16/32-bit offscreen worlds need direct-colour QuickDraw)")));
        }
        let depth = depth as u8;
        let (w, h) = (i32::from(bounds.right) - i32::from(bounds.left), i32::from(bounds.bottom) - i32::from(bounds.top));
        if w <= 0 || h <= 0 || w > 1 << 15 || h > 1 << 15 { return Ok(Err(PARAM_ERR)); }
        let row = row_bytes(w, depth);
        let Some(size) = row.checked_mul(h as u32) else { return Ok(Err(MEM_FULL_ERR)) };
        let Ok(base) = self.mm.new_ptr(&mut self.mem, &mut self.heap, size, true) else { return Ok(Err(MEM_FULL_ERR)) };
        let colors = if ctab != 0 { self.read_ctab(ctab)? } else {
            match depth { 4 => super::color::clut4(), 8 => super::color::clut8(), d => super::color::gray(d) }
        };
        let table = self.new_ctab(&colors, u32::from(depth), super::ctab::DEVICE_TABLE)?;
        let pm = self.new_pixmap(depth, bounds, base, row, table)?;
        let Ok(port) = self.mm.new_ptr(&mut self.mem, &mut self.heap, PORT_SIZE, true) else { let _ = self.mm.dispose_ptr(&mut self.heap, base); return Ok(Err(MEM_FULL_ERR)) };
        self.begin_cport(port, pm)?;
        write_rect(&mut self.mem, port.wrapping_add(P_RECT), bounds)?;
        let r = raster(bounds);
        let (vis, clip) = (self.new_region(&Region::from_rect(r))?, self.new_region(&Region::from_rect(crate::qd::regions::WIDE_OPEN))?);
        crate::qd::lifecycle::init_common_fields(self, port, vis, clip)?;
        let device = self.new_device_record(pm, bounds, 0)?;
        self.cqd.gworlds.insert(port, Gw { pixmap: pm, device, base, row, bounds, depth, state: flags & PIXELS_PURGEABLE });
        self.log.push(format!("[quickdraw] NewGWorld {w}x{h} {depth}-bit -> port {port:#x}, {size} bytes of pixels at {base:#x}"));
        Ok(Ok(port))
    }

    /// `DisposeGWorld(gw)`.
    pub fn gw_dispose(&mut self, port: u32) -> Result<(), Refusal> {
        let gw = self.cqd.gworlds.remove(&port).ok_or_else(|| Refusal(format!("DisposeGWorld: {port:#x} is not a graphics world this engine made")))?;
        for off in [P_VIS, P_CLIP] { let h = self.mem.read_u32(port.wrapping_add(off)).unwrap_or(0); if h != 0 { let _ = self.dispose_handle(h); } }
        self.release_cport(port); // pattern handles, the PixMap and its colour table
        let _ = self.dispose_handle(gw.device);
        let _ = self.mm.dispose_ptr(&mut self.heap, gw.base);
        let _ = self.mm.dispose_ptr(&mut self.heap, port);
        if self.mem.read_u32(self.qd_globals).unwrap_or(0) == port { let _ = self.mem.write_u32(self.qd_globals, 0); }
        if self.mem.read_u32(crate::lowmem::THE_GDEVICE).unwrap_or(0) == gw.device { let _ = self.mem.write_u32(crate::lowmem::THE_GDEVICE, self.cqd.main_device); }
        Ok(())
    }

    /// `UpdateGWorld(&gw, depth, &bounds, cTable, aGDevice, flags)`: give the world new bounds (and depth when `depth` != 0). The pixels are NOT carried over (QuickDraw may keep
    /// or lose them; `reallocPix` says which): the world is rebuilt and the OLD one disposed. Returns (`GWorldFlags` -- `reallocPix` (bit 20) or `gwFlagErr` --, the world's port now).
    pub fn gw_update(&mut self, port: u32, depth: u16, bounds: Rect, ctab: u32, flags: u32) -> Result<(u32, u32), Refusal> {
        let old = self.cqd.gworlds.get(&port).cloned().ok_or_else(|| Refusal(format!("UpdateGWorld: {port:#x} is not a graphics world this engine made")))?;
        let depth = if depth == 0 { u16::from(old.depth) } else { depth };
        match self.gw_new(depth, bounds, ctab, flags)? {
            Err(_) => Ok((GW_FLAG_ERR, port)),
            Ok(fresh) => {
                let current = self.gw_get()?.0;
                self.gw_dispose(port)?;
                if current == port { self.gw_set(fresh, 0)?; }
                Ok((1 << 20, fresh))
            }
        }
    }

    /// `GetGWorldPixMap(gw)`.
    pub fn gw_pixmap(&self, port: u32) -> Result<u32, Refusal> {
        self.cqd.gworlds.get(&port).map(|g| g.pixmap).ok_or_else(|| Refusal(format!("GetGWorldPixMap: {port:#x} is not a graphics world this engine made")))
    }
    /// `GetGWorldDevice(gw)`.
    pub fn gw_device(&self, port: u32) -> Result<u32, Refusal> {
        self.cqd.gworlds.get(&port).map(|g| g.device).ok_or_else(|| Refusal(format!("GetGWorldDevice: {port:#x} is not a graphics world this engine made")))
    }
    fn gw_of_pixmap(&mut self, pm: u32, who: &str) -> Result<&mut Gw, Refusal> {
        self.cqd.gworlds.values_mut().find(|g| g.pixmap == pm).ok_or_else(|| Refusal(format!("{who}: PixMap {pm:#x} belongs to no graphics world this engine made")))
    }
    /// `LockPixels(pm)`: always TRUE (the pixels are never purged); records the locked state.
    pub fn gw_lock(&mut self, pm: u32) -> Result<bool, Refusal> { self.gw_of_pixmap(pm, "LockPixels")?.state |= PIXELS_LOCKED; Ok(true) }
    pub fn gw_unlock(&mut self, pm: u32) -> Result<(), Refusal> { self.gw_of_pixmap(pm, "UnlockPixels")?.state &= !PIXELS_LOCKED; Ok(()) }
    pub fn gw_state(&mut self, pm: u32) -> Result<u32, Refusal> { Ok(self.gw_of_pixmap(pm, "GetPixelsState")?.state) }
    pub fn gw_set_state(&mut self, pm: u32, state: u32) -> Result<(), Refusal> { self.gw_of_pixmap(pm, "SetPixelsState")?.state = state & (PIXELS_LOCKED | PIXELS_PURGEABLE); Ok(()) }
    pub fn gw_purge(&mut self, pm: u32, purgeable: bool) -> Result<(), Refusal> {
        let g = self.gw_of_pixmap(pm, if purgeable { "AllowPurgePixels" } else { "NoPurgePixels" })?;
        if purgeable { g.state |= PIXELS_PURGEABLE } else { g.state &= !PIXELS_PURGEABLE }
        Ok(())
    }

    /// `GetPixBaseAddr(pm)`: the pixels' address (any PixMap, not only a GWorld's).
    pub fn pix_base_addr(&self, pm: u32) -> Result<u32, Refusal> {
        let p = self.deref_handle(pm, "PixMapHandle")?;
        Ok(self.mem.strip(self.mem.read_u32(p + PM_BASE).map_err(Refusal::mem("PixMap"))?))
    }
    /// `GetPixRowBytes(pm)`.
    pub fn pix_row_bytes(&self, pm: u32) -> Result<u32, Refusal> {
        let p = self.deref_handle(pm, "PixMapHandle")?;
        Ok(u32::from(self.mem.read_u16(p + PM_ROW).map_err(Refusal::mem("PixMap"))? & 0x3FFF))
    }
    /// `GetPixBounds(pm, &rect)`.
    pub fn pix_bounds(&self, pm: u32) -> Result<Rect, Refusal> { read_rect(&self.mem, self.deref_handle(pm, "PixMapHandle")? + PM_BOUNDS) }
    /// The colour table handle of a PixMap (`GetPixelsCTab`-style).
    pub fn pix_table(&self, pm: u32) -> Result<u32, Refusal> { self.mem.read_u32(self.deref_handle(pm, "PixMapHandle")? + PM_TABLE).map_err(Refusal::mem("PixMap")) }

    /// `SetGWorld(port, gdh)`: make `port` thePort and `gdh` (or, with NULL, the world's own device / the main device) TheGDevice.
    pub fn gw_set(&mut self, port: u32, gdh: u32) -> Result<(), Refusal> {
        crate::quickdraw::set_the_port(self, port)?;
        let dev = if gdh != 0 { gdh } else { self.cqd.gworlds.get(&port).map_or(self.cqd.main_device, |g| g.device) };
        if dev != 0 { self.mem.write_u32(crate::lowmem::THE_GDEVICE, dev).map_err(Refusal::mem("TheGDevice"))?; }
        Ok(())
    }
    /// `GetGWorld(&port, &gdh)`: thePort and TheGDevice.
    pub fn gw_get(&self) -> Result<(u32, u32), Refusal> {
        let port = if self.qd_globals == 0 { 0 } else { self.mem.read_u32(self.qd_globals).map_err(Refusal::mem("thePort"))? };
        Ok((port, self.mem.read_u32(crate::lowmem::THE_GDEVICE).unwrap_or(0)))
    }

    /// The BitMap/PixMap pointer `CopyBits` wants for `port` as a destination or source (`GetPortBitMapForCopyBits`): a colour port's PixMap record, a classic port's `portBits`.
    pub fn port_bits_for_copybits(&self, port: u32) -> Result<u32, Refusal> {
        if is_cport(&self.mem, port) { bits_addr(&self.mem, port) } else { Ok(port.wrapping_add(P_BITS)) }
    }
    pub fn gworld_summary(&self) -> String { format!("{} graphics world(s)", self.cqd.gworlds.len()) }
}
