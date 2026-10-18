//! GrafPort field offsets (Inside Macintosh, 108 bytes; the first 76 agree with the Window Manager fields metamage_1's
//! FindWindow walks, see toolbox.rs) and readers for the fields drawing needs.
use crate::pascal::{read_rect, Rect};
use crate::Refusal;
use retro_guest_mem::GuestMem;
use retro_raster::paint::Pattern;

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

pub const PAT_COPY: u16 = 8;
pub const SRC_OR: u16 = 1;

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
        v: mem.read_i16(port + P_PN_LOC).map_err(&e)?, h: mem.read_i16(port + P_PN_LOC + 2).map_err(&e)?,
        height: mem.read_i16(port + P_PN_SIZE).map_err(&e)?, width: mem.read_i16(port + P_PN_SIZE + 2).map_err(&e)?,
        mode: mem.read_u16(port + P_PN_MODE).map_err(&e)?, pat: read_pat(mem, port + P_PN_PAT)?, vis: mem.read_i16(port + P_PN_VIS).map_err(&e)?,
    })
}

pub fn set_pen_loc(mem: &mut GuestMem, port: u32, v: i16, h: i16) -> Result<(), Refusal> {
    let e = Refusal::mem("pnLoc");
    mem.write_u16(port + P_PN_LOC, v as u16).map_err(&e)?;
    mem.write_u16(port + P_PN_LOC + 2, h as u16).map_err(&e)
}

pub fn port_rect(mem: &GuestMem, port: u32) -> Result<Rect, Refusal> { read_rect(mem, port + P_RECT) }

pub fn write_rect(mem: &mut GuestMem, at: u32, r: Rect) -> Result<(), Refusal> {
    let e = Refusal::mem("Rect");
    for (i, v) in [r.top, r.left, r.bottom, r.right].into_iter().enumerate() { mem.write_u16(at + 2 * i as u32, v as u16).map_err(&e)?; }
    Ok(())
}

/// Raster rectangle from a Mac Rect (top, left, bottom, right -> x0, y0, x1, y1).
pub fn raster(r: Rect) -> retro_raster::Rect { retro_raster::Rect::new(i32::from(r.left), i32::from(r.top), i32::from(r.right), i32::from(r.bottom)) }
