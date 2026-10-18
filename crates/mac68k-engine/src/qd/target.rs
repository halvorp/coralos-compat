//! A drawing destination: the chunky [`Bitmap`] a port draws into, taken out of its home (the window's canvas, or a real
//! 1-bit BitMap in guest memory) for the duration of one operation and put back (and its damage recorded) after.
//!
//! * `local0`: the LOCAL coordinate of pixel (0,0): `pixel = local - local0`.
//! * `phase0`: pattern alignment. A pattern is aligned to the top-left of the bitmap it is drawn into (Inside
//!   Macintosh): a window draws into the screen, so its alignment is GLOBAL (pixel + the window's global origin); an
//!   offscreen BitMap aligns to its own pixel (0,0).
use super::port::*;
use crate::machine::Mac;
use crate::pascal::read_rect;
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_raster::{paint::Clip, Bitmap, Rect, Region};

pub enum Home { Canvas, Guest { addr: u32, row_bytes: u16 } }

pub struct Target { pub port: u32, pub bm: Bitmap, pub local0: (i32, i32), pub phase0: (i32, i32), pub home: Home }

impl Target {
    /// Local rectangle -> pixel rectangle.
    pub fn to_pixels(&self, r: Rect) -> Rect { r.translate(-self.local0.0, -self.local0.1) }
}

/// thePort, or the refusal that says there is none.
pub fn current_port<U: GuestUi>(m: &Mac<U>) -> Result<u32, Refusal> {
    if m.qd_globals == 0 { return Err(Refusal("QuickDraw: InitGraf has not run".into())); }
    let p = m.mem.read_u32(m.qd_globals).map_err(Refusal::mem("thePort"))?;
    if p == 0 { Err(Refusal("QuickDraw: thePort is NIL (SetPort a window or an OpenPort'd port first)".into())) } else { Ok(p) }
}

/// Unpack a 1-bit guest BitMap (`bm` = the struct's address) into a chunky bitmap; 1 = black = pen 1.
pub fn read_bitmap<U: GuestUi>(m: &Mac<U>, bm: u32) -> Result<(Bitmap, u32, u16, Rect), Refusal> {
    let e = Refusal::mem("BitMap");
    let (base, row) = (m.mem.read_u32(bm + BM_BASE).map_err(&e)?, m.mem.read_u16(bm + BM_ROW).map_err(&e)?);
    if row & 0x8000 != 0 { return Err(Refusal(format!("BitMap {bm:#x}: rowBytes {row:#06x} has the PixMap flag; color pixel maps are not implemented (1-bit BitMaps only)"))); }
    let b = read_rect(&m.mem, bm + BM_BOUNDS)?;
    let (w, h) = (i32::from(b.right) - i32::from(b.left), i32::from(b.bottom) - i32::from(b.top));
    let mut out = Bitmap::new(w, h, 1).ok_or_else(|| Refusal(format!("BitMap {bm:#x}: bounds ({},{},{},{}) are empty or too large", b.top, b.left, b.bottom, b.right)))?;
    if i32::from(row) * 8 < w { return Err(Refusal(format!("BitMap {bm:#x}: rowBytes {row} is too small for {w} pixels"))); }
    let bytes = m.mem.bytes(m.mem.span(base, u32::from(row) * h as u32).map_err(&e)?).map_err(&e)?;
    for y in 0..h { for x in 0..w { out.set(x, y, (bytes[y as usize * row as usize + (x >> 3) as usize] >> (7 - (x & 7))) & 1); } }
    Ok((out, base, row, Rect::new(i32::from(b.left), i32::from(b.top), i32::from(b.right), i32::from(b.bottom))))
}

fn write_bitmap<U: GuestUi>(m: &mut Mac<U>, bm: &Bitmap, base: u32, row: u16) -> Result<(), Refusal> {
    let mut out = alloc::vec![0u8; usize::from(row) * bm.height() as usize];
    for y in 0..bm.height() { for x in 0..bm.width() { if bm.get(x, y) == Some(1) { out[y as usize * usize::from(row) + (x >> 3) as usize] |= 0x80 >> (x & 7); } } }
    m.mem.write_bytes(base, &out).map_err(Refusal::mem("BitMap"))
}

/// Open the BitMap struct at `bm`: the window canvas whose embedded `portBits` it is (`bm == port + 2`), else the real
/// 1-bit BitMap in guest memory. `port` is the window's port for a canvas, 0 for a plain BitMap.
pub fn open_bits<U: GuestUi>(m: &mut Mac<U>, bm_addr: u32) -> Result<Target, Refusal> {
    let port = bm_addr.wrapping_sub(P_BITS);
    if let Some(bm) = m.qd.canvases.remove(&port) {
        let r = port_rect(&m.mem, port);
        let g = m.windows.get(&port).map(|w| w.rect);
        return match (r, g) {
            (Ok(r), Some(g)) => Ok(Target { port, bm, local0: (i32::from(r.left), i32::from(r.top)), phase0: (i32::from(g.left), i32::from(g.top)), home: Home::Canvas }),
            (r, _) => { m.qd.canvases.insert(port, bm); Err(r.err().unwrap_or_else(|| Refusal(format!("{port:#x}: a canvas without a window record")))) }
        };
    }
    let (bm, base, row, bounds) = read_bitmap(m, bm_addr)?;
    Ok(Target { port: 0, bm, local0: (bounds.x0, bounds.y0), phase0: (0, 0), home: Home::Guest { addr: base, row_bytes: row } })
}

/// Open the destination of `port`'s drawing.
pub fn open<U: GuestUi>(m: &mut Mac<U>, port: u32) -> Result<Target, Refusal> {
    let mut t = open_bits(m, port + P_BITS)
        .map_err(|Refusal(e)| Refusal(format!("port {port:#x} has no drawable bitmap ({e}); it is neither a window nor an OpenPort'd port with a 1-bit portBits")))?;
    t.port = port;
    Ok(t)
}

/// Put the destination back; `touched` (PIXEL coordinates) becomes damage for a window canvas.
pub fn finish<U: GuestUi>(m: &mut Mac<U>, t: Target, touched: Rect) -> Result<(), Refusal> {
    match t.home {
        Home::Canvas => { m.qd.canvases.insert(t.port, t.bm); m.qd.present.add_damage(t.port, touched); Ok(()) }
        Home::Guest { addr, row_bytes } => if touched.is_empty() { Ok(()) } else { write_bitmap(m, &t.bm, addr, row_bytes) },
    }
}

/// Run `f` against the destination of `port`, always putting it back.
pub fn with<U: GuestUi, F>(m: &mut Mac<U>, port: u32, f: F) -> Result<(), Refusal>
where F: FnOnce(&mut Mac<U>, &mut Target) -> Result<Rect, Refusal> {
    let mut t = open(m, port)?;
    let r = f(m, &mut t);
    let touched = r.as_ref().map(|r| *r).unwrap_or_default();
    finish(m, t, touched)?;
    r.map(|_| ())
}

/// clipRgn AND visRgn, in PIXEL coordinates, inside the bitmap.
pub fn clip_region<U: GuestUi>(m: &Mac<U>, t: &Target) -> Result<Region, Refusal> {
    let mut g = Region::from_rect(t.bm.rect());
    for off in [P_CLIP, P_VIS] {
        let h = m.mem.read_u32(t.port + off).map_err(Refusal::mem("GrafPort"))?;
        if h == 0 { continue; }
        let mut r = m.read_region(h)?;
        r.translate(-t.local0.0, -t.local0.1);
        g.and_region(&r);
    }
    Ok(g)
}

/// Helper so callers can build a [`Clip`] from a region they hold.
pub fn clip_of<'a>(g: &'a Region, t: &Target) -> Clip<'a> { Clip { rect: t.bm.rect(), region: Some(g) } }
