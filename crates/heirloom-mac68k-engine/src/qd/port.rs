//! GrafPort field offsets (Inside Macintosh, 108 bytes; the first 76 agree with the Window Manager fields metamage_1's
//! FindWindow walks, see toolbox.rs) and readers for the fields drawing needs.
use crate::pascal::{read_rect, Rect};
use crate::Refusal;
use heirloom_mem_flat32::GuestMem;
use heirloom_raster::paint::Pattern;

pub const P_BITS: u32 = 2; // BitMap (baseAddr 4, rowBytes 2, bounds 8)
pub const P_RECT: u32 = 16;
pub const P_VIS: u32 = 24; // RgnHandle
pub const P_CLIP: u32 = 28; // RgnHandle
pub const P_BK_PAT: u32 = 32;
pub const P_FILL_PAT: u32 = 40;
pub const P_PN_LOC: u32 = 48; // Point: v, h
pub const P_PN_SIZE: u32 = 52; // Point: v (height), h (width)
pub const P_PN_MODE: u32 = 56;
pub const P_PN_PAT: u32 = 58;
pub const P_PN_VIS: u32 = 66;
pub const P_TX_FONT: u32 = 68;
pub const P_TX_FACE: u32 = 70;
pub const P_TX_MODE: u32 = 72;
pub const P_TX_SIZE: u32 = 74;
pub const P_SP_EXTRA: u32 = 76;
pub const P_FG: u32 = 80;
pub const P_BK: u32 = 84;
pub const P_RGN_SAVE: u32 = 96;
pub const PORT_SIZE: u32 = 108;
/// A BitMap's fields.
pub const BM_BASE: u32 = 0;
pub const BM_ROW: u32 = 4;
pub const BM_BOUNDS: u32 = 6;

/// CGrafPort fields (Imaging With QuickDraw, "Color Graphics Ports"): `portVersion` shares the word of a GrafPort's
/// `portBits.rowBytes`, so its two high bits mark a colour port; `portPixMap` (a PixMapHandle) takes the place of
/// `portBits`; the three patterns are PixPatHandles; `rgbFgColor`/`rgbBkColor` are real RGBColors.
pub const P_PORT_VERSION: u32 = 6;
pub const CPORT_VERSION: u16 = 0xC000;
pub const CP_PIXMAP: u32 = 2;
pub const CP_BK_PIXPAT: u32 = 32;
pub const CP_RGB_FG: u32 = 36;
pub const CP_RGB_BK: u32 = 42;
pub const CP_PN_PIXPAT: u32 = 58;
pub const CP_FILL_PIXPAT: u32 = 62;
/// PixPat: `patType` 0 (an old-style pattern), `patMap` 2, `patData` 6, `patXData` 10, `patXValid` 14, `patXMap` 18,
/// `pat1Data` 22 (the 8-byte Pattern), 28 bytes.
pub const PIXPAT_SIZE: u32 = 28;
pub const PIXPAT_1DATA: u32 = 22;

pub const PAT_COPY: u16 = 8;
pub const SRC_OR: u16 = 1;

/// Which of a port's three patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatSlot { Bk, Pn, Fill }

/// Is `port` a CGrafPort (its `portVersion` word has both high bits set)?
pub fn is_cport(mem: &GuestMem, port: u32) -> bool { mem.read_u16(port.wrapping_add(P_PORT_VERSION)).is_ok_and(|v| v & CPORT_VERSION == CPORT_VERSION) }

/// The address of the BitMap or PixMap record `port` draws into: its embedded `portBits`, or the PixMap `portPixMap` names.
pub fn bits_addr(mem: &GuestMem, port: u32) -> Result<u32, Refusal> {
    if !is_cport(mem, port) { return Ok(port.wrapping_add(P_BITS)); }
    let h = mem.read_u32(port.wrapping_add(CP_PIXMAP)).map_err(Refusal::mem("portPixMap"))?;
    mem.read_u32(h).map_err(Refusal::mem("portPixMap"))
}

/// The 8 pattern bytes of a port's pattern slot. A GrafPort holds them inline; a CGrafPort holds a PixPatHandle, whose
/// `pat1Data` is the pattern when `patType` is 0. A colour pattern (`patType` 1 or 2) is refused by name.
pub fn pat_addr(mem: &GuestMem, port: u32, slot: PatSlot) -> Result<u32, Refusal> {
    if !is_cport(mem, port) {
        return Ok(port.wrapping_add(match slot { PatSlot::Bk => P_BK_PAT, PatSlot::Pn => P_PN_PAT, PatSlot::Fill => P_FILL_PAT }));
    }
    let e = Refusal::mem("PixPat");
    let h = mem.read_u32(port.wrapping_add(match slot { PatSlot::Bk => CP_BK_PIXPAT, PatSlot::Pn => CP_PN_PIXPAT, PatSlot::Fill => CP_FILL_PIXPAT })).map_err(&e)?;
    let p = mem.read_u32(h).map_err(&e)?;
    let kind = mem.read_u16(p).map_err(&e)?;
    if kind != 0 { return Err(Refusal(alloc::format!("{slot:?} pattern of port {port:#x} is a colour pattern (patType {kind}): colour patterns are not implemented (old-style patterns only)"))); }
    Ok(p + PIXPAT_1DATA)
}

pub fn read_port_pat(mem: &GuestMem, port: u32, slot: PatSlot) -> Result<[u8; 8], Refusal> { read_pat(mem, pat_addr(mem, port, slot)?) }
pub fn write_port_pat(mem: &mut GuestMem, port: u32, slot: PatSlot, p: &[u8; 8]) -> Result<(), Refusal> { let a = pat_addr(mem, port, slot)?; write_pat(mem, a, p) }

/// The port's pen colours as RGB: a CGrafPort's `rgbFgColor`/`rgbBkColor`, else the colour constants in `fgColor`/`bkColor`.
pub fn port_rgb(mem: &GuestMem, port: u32) -> Result<(crate::cqd::color::Rgb, crate::cqd::color::Rgb), Refusal> {
    use crate::cqd::color::{read_rgb, standard_color, Rgb};
    if is_cport(mem, port) { return Ok((read_rgb(mem, port.wrapping_add(CP_RGB_FG))?, read_rgb(mem, port.wrapping_add(CP_RGB_BK))?)); }
    let e = Refusal::mem("GrafPort");
    let code = |off: u32, d: Rgb| -> Result<Rgb, Refusal> { Ok(standard_color(mem.read_u32(port.wrapping_add(off)).map_err(&e)?).unwrap_or(d)) };
    Ok((code(P_FG, Rgb::BLACK)?, code(P_BK, Rgb::WHITE)?))
}

/// Everything a pen operation reads from the port.
#[derive(Debug, Clone, Copy)]
pub struct Pen { pub v: i16, pub h: i16, pub height: i16, pub width: i16, pub mode: u16, pub pat: [u8; 8], pub vis: i16 }

pub fn read_pat(mem: &GuestMem, addr: u32) -> Result<[u8; 8], Refusal> {
    let mut p = [0u8; 8];
    for (i, b) in p.iter_mut().enumerate() { *b = mem.read_u8(addr + i as u32).map_err(Refusal::mem("Pattern"))?; }
    Ok(p)
}

pub fn write_pat(mem: &mut GuestMem, addr: u32, p: &[u8; 8]) -> Result<(), Refusal> {
    mem.write_bytes(addr, p).map_err(Refusal::mem("Pattern"))
}

/// An 8x8 Mac pattern as the raster crate's repeating pattern (each row byte doubled: 16 wide, period 8).
pub fn pattern(p: &[u8; 8]) -> Pattern { Pattern { rows: p.iter().map(|&b| u16::from(b) * 0x0101).collect() } }

pub fn read_pen(mem: &GuestMem, port: u32) -> Result<Pen, Refusal> {
    let e = Refusal::mem("GrafPort");
    Ok(Pen {
        v: mem.read_i16(port.wrapping_add(P_PN_LOC)).map_err(&e)?, h: mem.read_i16(port.wrapping_add(P_PN_LOC + 2)).map_err(&e)?,
        height: mem.read_i16(port.wrapping_add(P_PN_SIZE)).map_err(&e)?, width: mem.read_i16(port.wrapping_add(P_PN_SIZE + 2)).map_err(&e)?,
        mode: mem.read_u16(port.wrapping_add(P_PN_MODE)).map_err(&e)?, pat: read_port_pat(mem, port, PatSlot::Pn)?, vis: mem.read_i16(port.wrapping_add(P_PN_VIS)).map_err(&e)?,
    })
}

pub fn set_pen_loc(mem: &mut GuestMem, port: u32, v: i16, h: i16) -> Result<(), Refusal> {
    let e = Refusal::mem("pnLoc");
    mem.write_u16(port.wrapping_add(P_PN_LOC), v as u16).map_err(&e)?;
    mem.write_u16(port.wrapping_add(P_PN_LOC + 2), h as u16).map_err(&e)
}

pub fn port_rect(mem: &GuestMem, port: u32) -> Result<Rect, Refusal> { read_rect(mem, port.wrapping_add(P_RECT)) }

pub fn write_rect(mem: &mut GuestMem, at: u32, r: Rect) -> Result<(), Refusal> {
    let e = Refusal::mem("Rect");
    for (i, v) in [r.top, r.left, r.bottom, r.right].into_iter().enumerate() { mem.write_u16(at + 2 * i as u32, v as u16).map_err(&e)?; }
    Ok(())
}

/// Raster rectangle from a Mac Rect (top, left, bottom, right -> x0, y0, x1, y1).
pub fn raster(r: Rect) -> heirloom_raster::Rect { heirloom_raster::Rect::new(i32::from(r.left), i32::from(r.top), i32::from(r.right), i32::from(r.bottom)) }
