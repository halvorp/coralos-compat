//! A drawing destination: the chunky [`Bitmap`] a port draws into, taken out of its home (the window's canvas, or a real
//! 1-bit BitMap in guest memory) for the duration of one operation and put back (and its damage recorded) after.
//!
//! * A target is 1 bit deep (a classic port) or an indexed colour depth (2, 4 or 8 bits: a CGrafPort or a PixMap), and then
//!   carries the colour table that gives its pixel values meaning ([`Target::clut`]). Every drawing op reads its pens from
//!   [`pens`], so one implementation serves both.
//! * `local0`: the LOCAL coordinate of pixel (0,0): `pixel = local - local0`.
//! * Pattern alignment is to LOCAL coordinates (Atkinson's QuickDraw/Util.a PATEXPAND "ALIGN TO LOCAL COORDS": the
//!   pattern is rotated by the global-local offset), so a pattern bit is chosen by `pixel + local0`, in a window and in an
//!   offscreen BitMap alike.
use super::port::*;
use crate::cqd::color::{self, Rgb};
use crate::cqd::pixmap::{self, PIXMAP_FLAG, PM_PIXEL_SIZE, PM_TABLE};
use crate::machine::Mac;
use crate::pascal::read_rect;
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_raster::{paint::Clip, Bitmap, Rect, Region};

pub enum Home { Canvas, Guest { addr: u32, row_bytes: u16 } }

pub struct Target {
    pub port: u32, pub bm: Bitmap, pub local0: (i32, i32), pub home: Home,
    /// The colour table of a deeper-than-1-bit target (`None`: 1 bit, white 0 / black 1).
    pub clut: Option<alloc::vec::Vec<Rgb>>,
    /// A display-list recorder (plane.rs) already accounted for this drawing; `finish` records the touched pixels when none did.
    pub recorded: bool,
}

impl Target {
    /// Local rectangle -> pixel rectangle.
    pub fn to_pixels(&self, r: Rect) -> Rect { r.translate(-self.local0.0, -self.local0.1) }

    /// A pen number as the host's `0x00RRGGBB`.
    pub fn host_color(&self, pen: u8) -> u32 {
        match &self.clut { Some(t) => t.get(usize::from(pen)).map_or(0, |c| c.to_host()), None => super::PALETTE[usize::from(pen).min(1)] }
    }
}

/// The pen numbers of `port`'s foreground and background colours in `t`: 1 and 0 in a 1-bit target (black ink, white
/// paper; a colour on a classic port is not rendered), else the nearest entries of the target's table to the port's RGB pens.
pub fn pens<U: GuestUi>(m: &Mac<U>, port: u32, t: &Target) -> Result<(u8, u8), Refusal> {
    let Some(table) = &t.clut else { return Ok((1, 0)) };
    if port == 0 { return Ok((color::nearest(table, Rgb::BLACK), color::nearest(table, Rgb::WHITE))); }
    let (fg, bg) = port_rgb(&m.mem, port)?;
    Ok((color::nearest(table, fg), color::nearest(table, bg)))
}

/// thePort, or the refusal that says there is none.
pub fn current_port<U: GuestUi>(m: &Mac<U>) -> Result<u32, Refusal> {
    if m.qd_globals == 0 { return Err(Refusal("QuickDraw: InitGraf has not run".into())); }
    let p = m.mem.read_u32(m.qd_globals).map_err(Refusal::mem("thePort"))?;
    if p == 0 { return Err(Refusal("QuickDraw: thePort is NIL (SetPort a window or an OpenPort'd port first)".into())); }
    checked_port(p)
}

/// A GrafPort address whose whole record fits below 4 GiB; anything else is a corrupt pointer, refused by name
/// instead of wrapping into low memory.
pub fn checked_port(p: u32) -> Result<u32, Refusal> {
    if p.checked_add(PORT_SIZE).is_none() { Err(Refusal(format!("QuickDraw: GrafPort pointer {p:#010x} runs past the end of the address space"))) } else { Ok(p) }
}

/// A guest BitMap or PixMap unpacked into a chunky bitmap: (bitmap, base address, row bytes, bounds, colour table).
pub type Unpacked = (Bitmap, u32, u16, Rect, Option<alloc::vec::Vec<Rgb>>);

/// Unpack the BitMap or PixMap at `bm` (`rowBytes` bit 15 = PixMap) into a chunky bitmap; in a 1-bit map 1 = black = pen 1.
pub fn read_bitmap<U: GuestUi>(m: &Mac<U>, bm: u32) -> Result<Unpacked, Refusal> {
    let e = Refusal::mem("BitMap");
    let (base, raw_row) = (m.mem.strip(m.mem.read_u32(bm.wrapping_add(BM_BASE)).map_err(&e)?), m.mem.read_u16(bm.wrapping_add(BM_ROW)).map_err(&e)?);
    let is_pixmap = raw_row & PIXMAP_FLAG != 0;
    let (row, depth) = if is_pixmap { (raw_row & 0x3FFF, m.mem.read_u16(bm.wrapping_add(PM_PIXEL_SIZE)).map_err(&e)?) } else { (raw_row, 1) };
    if !pixmap::supported_depth(depth) { return Err(Refusal(format!("PixMap {bm:#x}: {depth}-bit pixels are not implemented (indexed 1, 2, 4 and 8 bits only)"))); }
    let b = read_rect(&m.mem, bm.wrapping_add(BM_BOUNDS))?;
    let (w, h) = (i32::from(b.right) - i32::from(b.left), i32::from(b.bottom) - i32::from(b.top));
    if w <= 0 || h <= 0 || w > 1 << 15 || h > 1 << 15 { return Err(Refusal(format!("BitMap {bm:#x}: bounds ({},{},{},{}) are empty or too large", b.top, b.left, b.bottom, b.right))); }
    if i32::from(row) * 8 < w * i32::from(depth) { return Err(Refusal(format!("BitMap {bm:#x}: rowBytes {row} is too small for {w} pixels of {depth} bits"))); }
    let bytes = m.mem.bytes(m.mem.span(base, u32::from(row) * h as u32).map_err(&e)?).map_err(&e)?;
    let out = pixmap::unpack(bytes, usize::from(row), w, h, depth as u8)?;
    let clut = if is_pixmap && depth > 1 { Some(m.pixmap_clut(bm, depth as u8)?) } else { None };
    Ok((out, base, row, Rect::new(i32::from(b.left), i32::from(b.top), i32::from(b.right), i32::from(b.bottom)), clut))
}

fn write_bitmap<U: GuestUi>(m: &mut Mac<U>, bm: &Bitmap, base: u32, row: u16) -> Result<(), Refusal> {
    let mut out = alloc::vec![0u8; usize::from(row) * bm.height() as usize];
    pixmap::pack(bm, usize::from(row), &mut out);
    m.mem.write_bytes(base, &out).map_err(Refusal::mem("BitMap"))
}

impl<U: GuestUi> Mac<U> {
    /// The colour table of the PixMap at `pm`: its `pmTable`, or the standard table of its depth when it has none.
    pub(crate) fn pixmap_clut(&self, pm: u32, depth: u8) -> Result<alloc::vec::Vec<Rgb>, Refusal> {
        let h = self.mem.read_u32(pm.wrapping_add(PM_TABLE)).map_err(Refusal::mem("PixMap"))?;
        if h != 0 { return self.read_ctab(h); }
        Ok(match depth { 4 => color::clut4(), 8 => color::clut8(), d => color::gray(d) })
    }
}

/// Open the BitMap struct at `bm`: the window canvas whose embedded `portBits` it is (`bm == port + 2`), else the real
/// 1-bit BitMap in guest memory. `port` is the window's port for a canvas, 0 for a plain BitMap.
pub fn open_bits<U: GuestUi>(m: &mut Mac<U>, bm_addr: u32) -> Result<Target, Refusal> {
    let port = m.cqd.window_pix.get(&bm_addr).copied().unwrap_or_else(|| bm_addr.wrapping_sub(P_BITS));
    if let Some(bm) = m.qd.canvases.remove(&port) {
        let (r, g, deep) = (port_rect(&m.mem, port), m.windows.get(&port).map(|w| w.rect), bm.depth() > 1);
        let clut = if deep { m.pixmap_clut(bm_addr, bm.depth()).map_err(|e| { Refusal(format!("window {port:#x}: {}", e.0)) }) } else { Ok(alloc::vec::Vec::new()) };
        return match (r, g, clut) {
            (Ok(r), Some(_), Ok(t)) => Ok(Target { port, bm, local0: (i32::from(r.left), i32::from(r.top)), home: Home::Canvas, recorded: false, clut: deep.then_some(t) }),
            (r, _, c) => { m.qd.canvases.insert(port, bm); Err(r.err().or(c.err()).unwrap_or_else(|| Refusal(format!("{port:#x}: a canvas without a window record")))) }
        };
    }
    let (bm, base, row, bounds, clut) = read_bitmap(m, bm_addr)?;
    Ok(Target { port: 0, bm, local0: (bounds.x0, bounds.y0), home: Home::Guest { addr: base, row_bytes: row }, recorded: false, clut })
}

/// Open the destination of `port`'s drawing.
pub fn open<U: GuestUi>(m: &mut Mac<U>, port: u32) -> Result<Target, Refusal> {
    let mut t = open_bits(m, bits_addr(&m.mem, port)?)
        .map_err(|Refusal(e)| Refusal(format!("port {port:#x} has no drawable bitmap ({e}); it is neither a window nor an OpenPort'd/OpenCPort'd port over an indexed (1, 2, 4 or 8 bit) pixel map")))?;
    t.port = port;
    Ok(t)
}

/// Put the destination back; `touched` (PIXEL coordinates) becomes damage for a window canvas.
pub fn finish<U: GuestUi>(m: &mut Mac<U>, t: Target, touched: Rect) -> Result<(), Refusal> {
    match t.home {
        Home::Canvas => {
            if !t.recorded && !touched.is_empty() { super::plane::record_pixels(&mut m.qd.plane, &t, touched); }
            m.qd.canvases.insert(t.port, t.bm);
            m.qd.present.add_damage(t.port, touched);
            Ok(())
        }
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
pub fn clip_region<U: GuestUi>(m: &Mac<U>, t: &Target) -> Result<Region, Refusal> { clip_region_via(m, t.port, t) }

/// Like [`clip_region`], but with the clipRgn/visRgn of `port` (0: none, the whole bitmap) instead of the target's own.
pub fn clip_region_via<U: GuestUi>(m: &Mac<U>, port: u32, t: &Target) -> Result<Region, Refusal> {
    let mut g = Region::from_rect(t.bm.rect());
    if port == 0 { return Ok(g); }
    for off in [P_CLIP, P_VIS] {
        let h = m.mem.read_u32(port.wrapping_add(off)).map_err(Refusal::mem("GrafPort"))?;
        if h == 0 { continue; }
        let mut r = m.read_region(h)?;
        r.translate(-t.local0.0, -t.local0.1);
        g.and_region(&r);
    }
    Ok(g)
}

/// Helper so callers can build a [`Clip`] from a region they hold.
pub fn clip_of<'a>(g: &'a Region, t: &Target) -> Clip<'a> { Clip { rect: t.bm.rect(), region: Some(g) } }
