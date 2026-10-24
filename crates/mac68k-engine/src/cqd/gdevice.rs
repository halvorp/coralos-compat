//! `GDevice` / `GDHandle` (Imaging With QuickDraw, "Graphics Devices"): the record of a display (or an offscreen pixel map),
//! and the device list. The Macintosh II profile has one device, the main screen: 8 bits deep, the standard 8-bit color
//! table (a device table, seed 8), `gdRect` the screen. It is made when `InitGraf` runs ([`Mac::init_devices`]) and
//! published in the low-memory globals `MainDevice` ($8A4), `DeviceList` ($8A8) and `TheGDevice` ($CC8).
//!
//! ```text
//! gdRefNum 0  gdID 2  gdType 4  gdITable 6 (ITabHandle)  gdResPref 10  gdSearchProc 12  gdCompProc 16  gdFlags 20
//! gdPMap 22 (PixMapHandle)  gdRefCon 26 (LONGINT)  gdNextGD 30 (GDHandle)  gdRect 34  gdMode 42  gdCCBytes 46  gdCCDepth 48
//! gdCCXData 50  gdCCXMask 54  gdReserved 58                                                             = 62 bytes
//! ```
//! (`gdPMap` is a 4-byte handle: Inside Macintosh's fields sum to 62 bytes only so, and systemless writes `gdRect.top` at +34.
//! Programs read `(**GetMainDevice()).gdRect` themselves, so every offset here is load-bearing.)
//! `gdFlags` bits (Quickdraw.h): 0 gdDevType (color), 10 ramInit, 11 mainScreen, 12 allInit, 13 screenDevice, 14 noDriver,
//! 15 screenActive. `gdITable` is NIL: the inverse table is not modelled (`Color2Index` is answered by the engine).
use super::color;
use super::pixmap::{PM_PIXEL_SIZE, PM_TABLE};
use crate::lowmem::{DEVICE_LIST, MAIN_DEVICE, SCREEN_BASE, THE_GDEVICE};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg, Rect};
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const GD_TYPE: u32 = 4;
pub const GD_FLAGS: u32 = 20;
pub const GD_PMAP: u32 = 22;
pub const GD_REF_CON: u32 = 26;
pub const GD_NEXT: u32 = 30;
pub const GD_RECT: u32 = 34;
pub const GD_MODE: u32 = 42;
pub const GD_SIZE: u32 = 62;
pub const GD_DEV_TYPE: u16 = 0;
pub const NO_DRIVER: u16 = 14;
const MAIN_FLAGS: u16 = 1 << GD_DEV_TYPE | 1 << 10 | 1 << 11 | 1 << 12 | 1 << 13 | 1 << 15;
/// The screen depth of the main device.
pub const SCREEN_DEPTH: u8 = 8;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA2A, "GetMainDevice", &[], Some(Arg::L), get_main::<U>)?;
    reg(m, 0xAA32, "GetGDevice", &[], Some(Arg::L), get_gdevice::<U>)?;
    reg(m, 0xAA31, "SetGDevice", &[Arg::L], None, set_gdevice::<U>)?;
    reg(m, 0xAA29, "GetDeviceList", &[], Some(Arg::L), get_device_list::<U>)?;
    reg(m, 0xAA2B, "GetNextDevice", &[Arg::L], Some(Arg::L), get_next::<U>)?;
    reg(m, 0xAA2C, "TestDeviceAttribute", &[Arg::L, Arg::W], Some(Arg::B), test_attr::<U>)?;
    reg(m, 0xAA2D, "SetDeviceAttribute", &[Arg::L, Arg::W, Arg::B], None, set_attr::<U>)?;
    reg(m, 0xAA27, "GetMaxDevice", &[Arg::L], Some(Arg::L), get_max::<U>)?;
    reg(m, 0xAA2F, "NewGDevice", &[Arg::W, Arg::L], Some(Arg::L), new_gdevice::<U>)?;
    reg(m, 0xAA30, "DisposeGDevice", &[Arg::L], None, dispose_gdevice::<U>)
}

impl<U: GuestUi> Mac<U> {
    /// Make the main screen device (once) when this machine has Color QuickDraw, and publish it.
    pub fn init_devices(&mut self) -> Result<(), Refusal> {
        if !self.profile.color_qd || self.cqd.main_device != 0 { return Ok(()); }
        let (w, h) = crate::lowmem::screen_size(true);
        let screen = Rect { top: 0, left: 0, bottom: h, right: w };
        let table = self.new_ctab(&color::clut8(), u32::from(SCREEN_DEPTH), super::ctab::DEVICE_TABLE)?;
        let pm = self.new_pixmap(SCREEN_DEPTH, screen, SCREEN_BASE, u32::from(w as u16), table)?;
        let gd = self.new_device_record(pm, screen, MAIN_FLAGS)?;
        let e = Refusal::mem("device globals");
        for g in [MAIN_DEVICE, DEVICE_LIST, THE_GDEVICE] { self.mem.write_u32(g, gd).map_err(&e)?; }
        self.cqd.main_device = gd;
        self.cqd.devices = alloc::vec![gd];
        Ok(())
    }

    /// A GDevice record over the pixel map `pm`.
    pub(crate) fn new_device_record(&mut self, pm: u32, rect: Rect, flags: u16) -> Result<u32, Refusal> {
        let (h, p) = self.alloc_handle(GD_SIZE, "GDevice")?;
        let e = Refusal::mem("GDevice");
        self.mem.write_u16(p + GD_FLAGS, flags).map_err(&e)?;
        self.mem.write_u32(p + GD_PMAP, pm).map_err(&e)?;
        crate::qd::port::write_rect(&mut self.mem, p + GD_RECT, rect)?;
        Ok(h)
    }

    /// `TheGDevice`, or the refusal that says this machine has none.
    pub(crate) fn the_gdevice(&self) -> Result<u32, Refusal> {
        match self.mem.read_u32(THE_GDEVICE).map_err(Refusal::mem("TheGDevice"))? {
            0 => Err(Refusal(format!("no current GDevice: Color QuickDraw is not present (machine profile {})", self.profile.name))),
            gd => Ok(gd),
        }
    }

    /// The depth and color table handle of a device's pixel map.
    pub fn device_pixmap(&self, gd: u32) -> Result<(u32, u32), Refusal> {
        let e = Refusal::mem("GDevice");
        let pm = self.mem.read_u32(self.deref_handle(gd, "GDHandle")? + GD_PMAP).map_err(&e)?;
        let p = self.deref_handle(pm, "gdPMap")?;
        Ok((self.mem.read_u16(p + PM_PIXEL_SIZE).map_err(&e)?.into(), self.mem.read_u32(p + PM_TABLE).map_err(&e)?))
    }
}

fn get_main<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let h = m.mem.read_u32(MAIN_DEVICE).map_err(Refusal::mem("MainDevice"))?;
    if h == 0 { return Err(Refusal(format!("GetMainDevice: no main device (InitGraf has not run, or machine profile {} has no Color QuickDraw)", m.profile.name))); }
    Ok(Done { result: h, log: format!("[quickdraw] GetMainDevice -> {h:#x}") })
}

fn get_gdevice<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let h = m.the_gdevice()?;
    Ok(Done { result: h, log: format!("[quickdraw] GetGDevice -> {h:#x}") })
}

fn set_gdevice<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    m.deref_handle(h, "SetGDevice")?;
    m.mem.write_u32(THE_GDEVICE, h).map_err(Refusal::mem("TheGDevice"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] SetGDevice {h:#x}") })
}

fn get_device_list<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let h = m.mem.read_u32(DEVICE_LIST).map_err(Refusal::mem("DeviceList"))?;
    Ok(Done { result: h, log: format!("[quickdraw] GetDeviceList -> {h:#x}") })
}

fn get_next<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let gd = c.args.get(0);
    let next = m.mem.read_u32(m.deref_handle(gd, "GetNextDevice")? + GD_NEXT).map_err(Refusal::mem("GDevice"))?;
    Ok(Done { result: next, log: format!("[quickdraw] GetNextDevice {gd:#x} -> {next:#x}") })
}

fn attr<U: GuestUi>(m: &Mac<U>, gd: u32, bit: u16, what: &str) -> Result<(u32, u16), Refusal> {
    if bit > 15 { return Err(Refusal(format!("{what}: attribute bit {bit} is outside gdFlags (0..15)"))); }
    let p = m.deref_handle(gd, what)?;
    Ok((p + GD_FLAGS, m.mem.read_u16(p + GD_FLAGS).map_err(Refusal::mem("GDevice"))?))
}

fn test_attr<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (gd, bit) = (c.args.get(0), c.args.get(1) as u16);
    let set = attr(m, gd, bit, "TestDeviceAttribute")?.1 >> bit & 1 != 0;
    Ok(Done { result: u32::from(set), log: format!("[quickdraw] TestDeviceAttribute {gd:#x} bit {bit} -> {set}") })
}

fn set_attr<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (gd, bit, on) = (c.args.get(0), c.args.get(1) as u16, c.args.get(2) as u8 != 0);
    let (at, flags) = attr(m, gd, bit, "SetDeviceAttribute")?;
    m.mem.write_u16(at, if on { flags | 1 << bit } else { flags & !(1 << bit) }).map_err(Refusal::mem("GDevice"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] SetDeviceAttribute {gd:#x} bit {bit} := {on}") })
}

/// `GetMaxDevice(globalRect)`: the deepest screen device whose `gdRect` intersects the rectangle, else NIL.
fn get_max<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = read_rect(&m.mem, c.args.get(0))?;
    let mut best = (0u32, 0u32);
    for gd in m.cqd.devices.clone() {
        let p = m.deref_handle(gd, "GetMaxDevice")?;
        let g = read_rect(&m.mem, p + GD_RECT)?;
        let hit = r.left < g.right && g.left < r.right && r.top < g.bottom && g.top < r.bottom;
        let depth = m.device_pixmap(gd)?.0;
        if hit && (best.0 == 0 || depth > best.1) { best = (gd, depth); }
    }
    Ok(Done { result: best.0, log: format!("[quickdraw] GetMaxDevice ({},{},{},{}) -> {:#x}", r.top, r.left, r.bottom, r.right, best.0) })
}

/// `NewGDevice(refNum, mode)`: a device record with its own (blank, 1-bit) PixMap; refNum -1 is a device with no driver
/// (the offscreen kind, `noDriver`). A real display driver is not modelled: any other refNum is refused.
fn new_gdevice<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (rn, mode) = (c.args.get(0) as u16 as i16, c.args.get(1));
    if rn != -1 { return Err(Refusal(format!("NewGDevice: driver refNum {rn}: only refNum -1 (no driver) is modelled, the engine has no video drivers"))); }
    let blank = Rect { top: 0, left: 0, bottom: 0, right: 0 };
    let pm = m.new_pixmap(1, blank, 0, 0, 0)?;
    let gd = m.new_device_record(pm, blank, 1 << NO_DRIVER)?;
    let p = m.deref_handle(gd, "NewGDevice")?;
    m.mem.write_u16(p + 0, rn as u16).and_then(|_| m.mem.write_u32(p + GD_MODE, mode)).map_err(Refusal::mem("GDevice"))?;
    Ok(Done { result: gd, log: format!("[quickdraw] NewGDevice -1 mode {mode:#x} -> {gd:#x}") })
}

fn dispose_gdevice<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let gd = c.args.get(0);
    if gd == m.cqd.main_device { return Err(Refusal("DisposeGDevice: the main screen device cannot be disposed".into())); }
    let pm = m.mem.read_u32(m.deref_handle(gd, "DisposeGDevice")? + GD_PMAP).map_err(Refusal::mem("GDevice"))?;
    if pm != 0 { let _ = m.dispose_handle(pm); }
    m.dispose_handle(gd).map_err(|e| Refusal(format!("DisposeGDevice {gd:#x}: memory manager error {e}")))?;
    Ok(Done { result: 0, log: format!("[quickdraw] DisposeGDevice {gd:#x}") })
}
