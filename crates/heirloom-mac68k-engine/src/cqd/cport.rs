//! `CGrafPort` records (Imaging With QuickDraw, "Color Graphics Ports"): `OpenCPort`, `InitCPort`, `CloseCPort`, and the
//! colour window's port (`NewCWindow`, `GetNewCWindow`).
//!
//! ```text
//! device 0   portPixMap 2 (PixMapHandle)   portVersion 6 ($C000)   grafVars 8   chExtra 12   pnLocHFrac 14   portRect 16
//! visRgn 24  clipRgn 28  bkPixPat 32   rgbFgColor 36   rgbBkColor 42   pnLoc 48   pnSize 52   pnMode 56
//! pnPixPat 58   fillPixPat 62   pnVis 66   txFont 68 ... (the GrafPort's text and region fields at the same offsets)
//! ```
//! A fresh colour port has black ink on white paper and old-style black / white / black patterns (`PixPat` `patType` 0).
//! `grafVars` (the `OpColor`/`HiliteColor` record) is NIL: those two traps are not modelled.
//!
//! `OpenCPort` gives the port a COPY of the current device's PixMap (and colour table), pixels at the screen (the engine
//! draws nothing to the pretend screen address: a program points the port at its own pixels with `SetPortPix`, or opens a
//! colour window). A colour window's port shares the main device's colour table and its pixels are the window's host canvas.
use super::color::Rgb;
use super::pixmap::{PIXMAP_FLAG, PM_BASE, PM_BOUNDS, PM_ROW, PM_TABLE};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::qd::port::*;
use crate::qd::regions::WIDE_OPEN;
use crate::quickdraw::set_the_port;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::Region;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA00, "OpenCPort", &[Arg::P], None, open_cport::<U>)?;
    reg(m, 0xAA01, "InitCPort", &[Arg::P], None, init_cport::<U>)?;
    reg(m, 0xAA02, "CloseCPort", &[Arg::P], None, close_cport::<U>)?;
    reg(m, 0xAA06, "SetPortPix", &[Arg::L], None, set_port_pix::<U>)
}

impl<U: GuestUi> Mac<U> {
    /// A `PixPatHandle` holding the old-style pattern `pat` (`patType` 0; `pat1Data` is the pattern).
    pub(crate) fn new_pixpat(&mut self, pat: &[u8; 8]) -> Result<u32, Refusal> {
        let (h, p) = self.alloc_handle(PIXPAT_SIZE, "PixPat")?;
        write_pat(&mut self.mem, p + PIXPAT_1DATA, pat)?;
        Ok(h)
    }

    /// Make the record at `port` a colour port over the PixMap `pixmap`: version, pixel map, patterns, colours. The regions,
    /// `portRect` and the pen/text fields shared with a GrafPort are the caller's (`init_fields`).
    pub(crate) fn begin_cport(&mut self, port: u32, pixmap: u32) -> Result<(), Refusal> {
        let e = Refusal::mem("CGrafPort");
        let (bk, pn, fill) = (self.new_pixpat(&[0; 8])?, self.new_pixpat(&[0xFF; 8])?, self.new_pixpat(&[0xFF; 8])?);
        self.mem.write_u16(port.wrapping_add(P_PORT_VERSION), CPORT_VERSION).map_err(&e)?;
        self.mem.write_u32(port.wrapping_add(CP_PIXMAP), pixmap).map_err(&e)?;
        for (off, h) in [(CP_BK_PIXPAT, bk), (CP_PN_PIXPAT, pn), (CP_FILL_PIXPAT, fill)] { self.mem.write_u32(port.wrapping_add(off), h).map_err(&e)?; }
        super::color::write_rgb(&mut self.mem, port.wrapping_add(CP_RGB_FG), Rgb::BLACK)?;
        super::color::write_rgb(&mut self.mem, port.wrapping_add(CP_RGB_BK), Rgb::WHITE)
    }

    /// A colour window's port: a PixMap the size of the screen mapping (pixels are the window's canvas), sharing the main
    /// device's colour table, registered so a drawing op finds the canvas through the PixMap.
    pub(crate) fn begin_window_cport(&mut self, win: u32) -> Result<(), Refusal> {
        self.init_devices()?;
        let (depth, table) = self.device_pixmap(self.cqd.main_device)?;
        let bounds = crate::pascal::Rect { top: 0, left: 0, bottom: 0, right: 0 };
        let pm = self.new_pixmap(depth as u8, bounds, crate::lowmem::SCREEN_BASE, u32::from(crate::lowmem::screen_size(true).0 as u16), table)?;
        let p = self.deref_handle(pm, "PixMap")?;
        self.begin_cport(win, pm)?;
        self.cqd.window_pix.insert(p, win);
        Ok(())
    }

    /// Free what `begin_cport` made (the colour table only when the port owns a copy, never the main device's).
    pub(crate) fn release_cport(&mut self, port: u32) {
        let rd = |m: &Self, off: u32| m.mem.read_u32(port.wrapping_add(off)).unwrap_or(0);
        for off in [CP_BK_PIXPAT, CP_PN_PIXPAT, CP_FILL_PIXPAT] { let h = rd(self, off); if h != 0 { let _ = self.dispose_handle(h); } }
        let pm = rd(self, CP_PIXMAP);
        if let Ok(p) = self.deref_handle(pm, "portPixMap") {
            self.cqd.window_pix.remove(&p);
            let table = self.mem.read_u32(p + PM_TABLE).unwrap_or(0);
            let shared = self.device_pixmap(self.cqd.main_device).map_or(0, |d| d.1);
            if table != 0 && table != shared { let _ = self.dispose_handle(table); }
            let _ = self.dispose_handle(pm);
        }
    }
}

/// The record's regions and pixel map for a (re)opened colour port, then the common fields; the port becomes `thePort`.
fn open_or_init<U: GuestUi>(m: &mut Mac<U>, port: u32, fresh: bool, name: &str) -> Result<Done, Refusal> {
    if m.profile.color_qd { m.init_devices()?; } else { return Err(Refusal(format!("{name}: Color QuickDraw is not present on machine profile {}", m.profile.name))); }
    let port = crate::qd::target::checked_port(port)?;
    let e = Refusal::mem("CGrafPort");
    let (depth, table) = m.device_pixmap(m.cqd.main_device)?;
    if fresh {
        m.mem.zero(port, PORT_SIZE).map_err(&e)?;
        // the device's PixMap, copied: the port owns it and its colour table
        let dev_pm = m.mem.read_u32(m.deref_handle(m.cqd.main_device, name)? + super::gdevice::GD_PMAP).map_err(&e)?;
        let dev = m.deref_handle(dev_pm, name)?;
        let bounds = read_rect(&m.mem, dev + PM_BOUNDS)?;
        let own = m.new_ctab(&m.read_ctab(table)?, depth, super::ctab::DEVICE_TABLE)?;
        let (base, row) = (m.mem.read_u32(dev + PM_BASE).map_err(&e)?, m.mem.read_u16(dev + PM_ROW).map_err(&e)? & !PIXMAP_FLAG);
        let pm = m.new_pixmap(depth as u8, bounds, base, u32::from(row), own)?;
        m.begin_cport(port, pm)?;
        crate::qd::port::write_rect(&mut m.mem, port.wrapping_add(P_RECT), bounds)?;
        let r = crate::qd::port::raster(bounds);
        let (vis, clip) = (m.new_region(&Region::from_rect(r))?, m.new_region(&Region::from_rect(WIDE_OPEN))?);
        crate::qd::lifecycle::init_common_fields(m, port, vis, clip)?;
    } else {
        // InitCPort: the pixel map stays; regions, pens and patterns return to their defaults
        let (vis, clip) = (m.mem.read_u32(port.wrapping_add(P_VIS)).map_err(&e)?, m.mem.read_u32(port.wrapping_add(P_CLIP)).map_err(&e)?);
        if !is_cport(&m.mem, port) || vis == 0 || clip == 0 { return Err(Refusal(format!("{name} {port:#x}: not an open CGrafPort (use OpenCPort first)"))); }
        let pm = bits_addr(&m.mem, port)?;
        let b = read_rect(&m.mem, pm + PM_BOUNDS)?;
        m.mem.write_bytes(port.wrapping_add(P_RECT), &m.mem.bytes(m.mem.span(pm + PM_BOUNDS, 8).map_err(&e)?).map_err(&e)?.to_vec()).map_err(&e)?;
        m.write_region(vis, &Region::from_rect(crate::qd::port::raster(b)))?;
        m.write_region(clip, &Region::from_rect(WIDE_OPEN))?;
        for (slot, pat) in [(PatSlot::Bk, [0u8; 8]), (PatSlot::Pn, [0xFF; 8]), (PatSlot::Fill, [0xFF; 8])] { write_port_pat(&mut m.mem, port, slot, &pat)?; }
        super::color::write_rgb(&mut m.mem, port.wrapping_add(CP_RGB_FG), Rgb::BLACK)?;
        super::color::write_rgb(&mut m.mem, port.wrapping_add(CP_RGB_BK), Rgb::WHITE)?;
        crate::qd::lifecycle::init_common_fields(m, port, vis, clip)?;
    }
    set_the_port(m, port)?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name} {port:#x}") })
}

fn open_cport<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { open_or_init(m, c.args.get(0), true, "OpenCPort") }
fn init_cport<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { open_or_init(m, c.args.get(0), false, "InitCPort") }

fn close_cport<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = crate::qd::target::checked_port(c.args.get(0))?;
    if !is_cport(&m.mem, port) { return Err(Refusal(format!("CloseCPort {port:#x}: not a CGrafPort"))); }
    if m.qd.canvases.contains_key(&port) { return Err(Refusal(format!("CloseCPort {port:#x}: that is a window's port (use DisposeWindow)"))); }
    for off in [P_VIS, P_CLIP] {
        let h = m.mem.read_u32(port.wrapping_add(off)).map_err(Refusal::mem("CGrafPort"))?;
        if h != 0 { m.dispose_handle(h).map_err(|e| Refusal(format!("CloseCPort {port:#x}: region handle {h:#x}: memory manager error {e}")))?; m.mem.write_u32(port.wrapping_add(off), 0).map_err(Refusal::mem("CGrafPort"))?; }
    }
    m.release_cport(port);
    Ok(Done { result: 0, log: format!("[quickdraw] CloseCPort {port:#x}") })
}

/// `SetPortPix(pm)`: the current CGrafPort draws into the pixel map `pm` from now on. The port's previous pixel map is not
/// disposed (the program owns what it replaced); `portRect` and the regions are the program's to change.
fn set_port_pix<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (port, pm) = (crate::qd::target::current_port(m)?, c.args.get(0));
    if !is_cport(&m.mem, port) { return Err(Refusal(format!("SetPortPix: thePort {port:#x} is a basic GrafPort (use SetPortBits)"))); }
    if m.qd.canvases.contains_key(&port) { return Err(Refusal(format!("SetPortPix: {port:#x} is a window's port; its pixels are the window"))); }
    m.deref_handle(pm, "SetPortPix")?;
    m.mem.write_u32(port.wrapping_add(CP_PIXMAP), pm).map_err(Refusal::mem("portPixMap"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] SetPortPix {pm:#x} into {port:#x}") })
}
