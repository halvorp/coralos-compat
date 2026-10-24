//! `PixMap` / `PixMapHandle` (Imaging With QuickDraw, "Color QuickDraw Reference"). A PixMap is a BitMap with the high bit
//! of `rowBytes` set and more fields after `bounds`, so `baseAddr`, `rowBytes` and `bounds` sit at the BitMap's offsets and
//! every routine that takes a BitMap pointer takes a PixMap pointer too.
//!
//! ```text
//! baseAddr 0 (Ptr)   rowBytes 4 (word, | $8000)   bounds 6 (Rect)   pmVersion 14   packType 16   packSize 18 (long)
//! hRes 22 (Fixed)    vRes 26 (Fixed)              pixelType 30       pixelSize 32  cmpCount 34    cmpSize 36
//! planeBytes 38 (long)   pmTable 42 (CTabHandle)  pmReserved 46 (long)         = 50 bytes
//! ```
//! Pixels of 1, 2, 4 and 8 bits are packed most-significant-bit first, rows `rowBytes` apart. Deeper (direct) pixel maps are a
//! named refusal: the engine's rasteriser is indexed (one pen number per pixel).
use crate::Refusal;
use alloc::format;
use retro_raster::Bitmap;

pub const PM_BASE: u32 = 0;
pub const PM_ROW: u32 = 4;
pub const PM_BOUNDS: u32 = 6;
pub const PM_VERSION: u32 = 14;
pub const PM_PACK_TYPE: u32 = 16;
pub const PM_PACK_SIZE: u32 = 18;
pub const PM_HRES: u32 = 22;
pub const PM_VRES: u32 = 26;
pub const PM_PIXEL_TYPE: u32 = 30;
pub const PM_PIXEL_SIZE: u32 = 32;
pub const PM_CMP_COUNT: u32 = 34;
pub const PM_CMP_SIZE: u32 = 36;
pub const PM_PLANE_BYTES: u32 = 38;
pub const PM_TABLE: u32 = 42;
pub const PM_SIZE: u32 = 50;
/// `rowBytes` bit 15: this record is a PixMap, not a BitMap.
pub const PIXMAP_FLAG: u16 = 0x8000;
/// 72 dpi as a Fixed.
pub const DPI_72: u32 = 0x0048_0000;

/// Bytes per row of `width` pixels of `depth` bits, rounded up to a whole word (what NewGWorld/NewPixMap use).
pub fn row_bytes(width: i32, depth: u8) -> u32 { ((width as u32 * u32::from(depth) + 15) / 16) * 2 }

/// Unpack `rows` rows of `width` pixels of `depth` (1, 2, 4 or 8) bits from `bytes` (rows `row` bytes apart).
pub fn unpack(bytes: &[u8], row: usize, width: i32, height: i32, depth: u8) -> Result<Bitmap, Refusal> {
    let mut b = Bitmap::new(width, height, depth).ok_or_else(|| Refusal(format!("pixel map {width}x{height}x{depth}: empty, too large, or a depth the rasteriser does not have")))?;
    let per = 8 / usize::from(depth);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let byte = bytes[y * row + x / per];
            let shift = 8 - usize::from(depth) * (x % per + 1);
            b.set(x as i32, y as i32, (byte >> shift) & b.pen_mask());
        }
    }
    Ok(b)
}

/// Pack a bitmap into rows `row` bytes apart (`out` is zeroed first: the row padding is cleared).
pub fn pack(b: &Bitmap, row: usize, out: &mut [u8]) {
    out.fill(0);
    let (depth, per) = (b.depth(), 8 / usize::from(b.depth()));
    for y in 0..b.height() as usize {
        for x in 0..b.width() as usize {
            let shift = 8 - usize::from(depth) * (x % per + 1);
            out[y * row + x / per] |= b.get(x as i32, y as i32).unwrap_or(0) << shift;
        }
    }
}

/// The depths the rasteriser can hold.
pub fn supported_depth(d: u16) -> bool { matches!(d, 1 | 2 | 4 | 8) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_pack_msb_first_at_every_depth_and_round_trip() {
        for depth in [1u8, 2, 4, 8] {
            let mut b = Bitmap::new(5, 3, depth).unwrap();
            let max = b.pen_mask();
            for y in 0..3 { for x in 0..5 { b.set(x, y, ((x * 3 + y * 7 + 1) as u8) & max); } }
            let row = row_bytes(5, depth) as usize;
            let mut buf = alloc::vec![0xEEu8; row * 3];
            pack(&b, row, &mut buf);
            assert_eq!(unpack(&buf, row, 5, 3, depth).unwrap(), b, "depth {depth}");
        }
        let mut b = Bitmap::new(4, 1, 2).unwrap();
        for (x, p) in [1u8, 2, 3, 0].into_iter().enumerate() { b.set(x as i32, 0, p); }
        let mut buf = [0u8; 2];
        pack(&b, 2, &mut buf);
        assert_eq!(buf, [0b01_10_11_00, 0], "2-bit pixels, first pixel in the top bits; padding cleared");
        assert_eq!(row_bytes(5, 8), 6);
        assert_eq!(row_bytes(512, 8), 512);
        assert!(unpack(&[0; 4], 4, 4, 1, 16).is_err() && !supported_depth(16));
    }
}

use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use retro_m68k::Cpu;
use crate::pascal::Rect;
use guestui_abi::GuestUi;

impl<U: GuestUi> Mac<U> {
    /// A new `PixMapHandle` of `depth`-bit chunky pixels over `bounds`, pixels at `base` (0: none yet), `row` bytes a row,
    /// color table `table` (0: none). Resolution 72 dpi, one component of `depth` bits (Imaging With QuickDraw).
    pub fn new_pixmap(&mut self, depth: u8, bounds: Rect, base: u32, row: u32, table: u32) -> Result<u32, Refusal> {
        let (h, p) = self.alloc_handle(PM_SIZE, "PixMap")?;
        let e = Refusal::mem("PixMap");
        self.mem.write_u32(p + PM_BASE, base).map_err(&e)?;
        self.mem.write_u16(p + PM_ROW, PIXMAP_FLAG | row as u16).map_err(&e)?;
        crate::qd::port::write_rect(&mut self.mem, p + PM_BOUNDS, bounds)?;
        self.mem.write_u32(p + PM_HRES, DPI_72).map_err(&e)?;
        self.mem.write_u32(p + PM_VRES, DPI_72).map_err(&e)?;
        for (off, v) in [(PM_PIXEL_SIZE, u16::from(depth)), (PM_CMP_COUNT, 1), (PM_CMP_SIZE, u16::from(depth))] { self.mem.write_u16(p + off, v).map_err(&e)?; }
        self.mem.write_u32(p + PM_TABLE, table).map_err(&e)?;
        Ok(h)
    }
}

/// Register the PixMap traps: `NewPixMap`, `DisposePixMap`, `CopyPixMap`.
pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA03, "NewPixMap", &[], Some(Arg::L), new_pixmap_trap::<U>)?;
    reg(m, 0xAA04, "DisposePixMap", &[Arg::L], None, dispose_pixmap::<U>)?;
    reg(m, 0xAA05, "CopyPixMap", &[Arg::L, Arg::L], None, copy_pixmap::<U>)
}

/// `FUNCTION NewPixMap: PixMapHandle` (Inside Macintosh V-70; Executor qCGrafPort.cpp `C_NewPixMap`): every field comes from the
/// PixMap of `TheGDevice` (so a new map describes the screen's depth, resolution and `baseAddr`), except `pmTable`: a color table
/// of its own is allocated and left uninitialised (one entry, seed unique), to be filled by `CopyPixMap` or the program.
fn new_pixmap_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let gd = m.the_gdevice()?;
    let gp = m.deref_handle(gd, "NewPixMap")?;
    let src_h = m.mem.read_u32(gp + super::gdevice::GD_PMAP).map_err(Refusal::mem("GDevice"))?;
    let src = m.deref_handle(src_h, "NewPixMap: the device's PixMap")?;
    let rec = m.mem.bytes(m.mem.span(src, PM_SIZE).map_err(Refusal::mem("PixMap"))?).map_err(Refusal::mem("PixMap"))?.to_vec();
    let (h, p) = m.alloc_handle(PM_SIZE, "PixMap")?;
    m.mem.write_bytes(p, &rec).map_err(Refusal::mem("PixMap"))?;
    let (ct, cp) = m.alloc_handle(super::ctab::CT_TABLE + super::ctab::SPEC, "NewPixMap color table")?;
    let seed = m.unique_seed();
    m.mem.write_u32(cp + super::ctab::CT_SEED, seed).map_err(Refusal::mem("ColorTable"))?;
    m.mem.write_u32(p + PM_TABLE, ct).map_err(Refusal::mem("PixMap"))?;
    Ok(Done { result: h, log: format!("[quickdraw] NewPixMap -> {h:#x} (fields of the device's PixMap {src_h:#x}, its own color table {ct:#x})") })
}

/// `PROCEDURE DisposePixMap(pm)`: the color table, then the map. NIL is accepted.
fn dispose_pixmap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if h != 0 {
        let p = m.deref_handle(h, "DisposePixMap")?;
        let ct = m.mem.read_u32(p + PM_TABLE).map_err(Refusal::mem("PixMap"))?;
        if ct != 0 && m.mm.block(ct).is_some() { m.dispose_handle(ct).map_err(|e| Refusal(format!("DisposePixMap: color table {ct:#x}: memory manager error {e}")))?; }
        m.dispose_handle(h).map_err(|e| Refusal(format!("DisposePixMap {h:#x}: memory manager error {e}")))?;
    }
    Ok(Done { result: 0, log: format!("[quickdraw] DisposePixMap {h:#x}") })
}

/// `PROCEDURE CopyPixMap(srcPM, dstPM)`: the record, except that `dstPM` keeps its own color table, whose contents become the
/// source table's (Executor `C_CopyPixMap`).
fn copy_pixmap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (sh, dh) = (c.args.get(0), c.args.get(1));
    let (sp, dp) = (m.deref_handle(sh, "CopyPixMap source")?, m.deref_handle(dh, "CopyPixMap destination")?);
    let e = Refusal::mem("PixMap");
    let rec = m.mem.bytes(m.mem.span(sp, PM_SIZE).map_err(&e)?).map_err(&e)?.to_vec();
    let (sct, dct) = (m.mem.read_u32(sp + PM_TABLE).map_err(&e)?, m.mem.read_u32(dp + PM_TABLE).map_err(&e)?);
    m.mem.write_bytes(dp, &rec).map_err(&e)?;
    m.mem.write_u32(dp + PM_TABLE, dct).map_err(&e)?;
    if sct != 0 && dct != 0 {
        let (sb, ssize) = { let b = m.mm.block(sct).ok_or_else(|| Refusal(format!("CopyPixMap: source color table {sct:#x} is not a handle")))?; (b.ptr, b.size) };
        let bytes = m.mem.bytes(m.mem.span(sb, ssize).map_err(&e)?).map_err(&e)?.to_vec();
        m.mm.set_handle_size(&mut m.mem, &mut m.heap, dct, ssize).map_err(|x| Refusal(format!("CopyPixMap: no memory for the color table (memFullErr {x})")))?;
        let db = m.deref_handle(dct, "CopyPixMap destination color table")?;
        m.mem.write_bytes(db, &bytes).map_err(&e)?;
    }
    Ok(Done { result: 0, log: format!("[quickdraw] CopyPixMap {sh:#x} -> {dh:#x}") })
}
