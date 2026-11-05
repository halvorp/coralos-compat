//! Color icons: `GetCIcon(id)`, `PlotCIcon(theRect, theIcon)`, `DisposeCIcon(theIcon)` (Imaging With QuickDraw, "Color QuickDraw", the
//! `'cicn'` resource and the CIcon record).
//!
//! ```text
//! CIcon:  iconPMap 0 (PixMap, 50)   iconMask 50 (BitMap, 14)   iconBMap 64 (BitMap, 14)   iconData 78 (Handle)   iconMaskData 82 ...
//! 'cicn': the same 82-byte header, then the mask's bits, the 1-bit image's bits, the colour table (ColorTable), the pixel image
//! ```
//! `GetCIcon` makes a CIconHandle holding the header and the two bitmaps' bits (their `baseAddr`s point at them), a `CTabHandle` for
//! `pmTable` and a handle of the pixels for `iconData` (`iconPMap.baseAddr` stays NIL: the pixels are named by the handle, as in the ROM).
//! `PlotCIcon` draws the colour image into `thePort` through the mask with `CopyMask` (the same blit, clipping and depth conversion as a
//! program's own CopyMask); a rectangle of another size than the icon is drawn with `CopyBits` scaled and WITHOUT the mask (CopyMask
//! does not scale here), named in the log. A missing `'cicn'` is NIL with QDError resNotFound (-192).
use crate::cpu::Cpu;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;

const PMAP: u32 = 0;
const MASK: u32 = 50;
const BMAP: u32 = 64;
const DATA: u32 = 78;
const HEADER: u32 = 82;
const BM_ROW: u32 = 4;
const BM_BOUNDS: u32 = 6;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA1E, "GetCIcon", &[Arg::W], Some(Arg::L), get_cicon::<U>)?;
    reg(m, 0xAA1F, "PlotCIcon", &[Arg::P, Arg::L], None, plot_cicon::<U>)?;
    reg(m, 0xAA25, "DisposeCIcon", &[Arg::L], None, dispose_cicon::<U>)
}

fn be16(b: &[u8], at: usize) -> Option<u16> { Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?])) }
/// Bytes of the image a BitMap / PixMap header at `at` in `b` describes: (rowBytes & $3FFF) x the bounds' height.
fn image_len(b: &[u8], at: usize) -> Option<usize> {
    let row = usize::from(be16(b, at + BM_ROW as usize)? & 0x3FFF);
    let (top, bottom) = (be16(b, at + BM_BOUNDS as usize)? as i16, be16(b, at + BM_BOUNDS as usize + 4)? as i16);
    Some(row * usize::try_from(i32::from(bottom) - i32::from(top)).ok()?)
}

fn get_cicon<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(r) = m.get_resource(*b"cicn", id, false) else {
        m.cqd.qd_error = -192;
        return Ok(Done { result: 0, log: format!("[quickdraw] GetCIcon {id}: no 'cicn' (NIL, QDError -192)") });
    };
    let b = m.handle_bytes(r).unwrap_or_default();
    let bad = |why: &str| Refusal(format!("GetCIcon {id}: the 'cicn' is malformed ({why})"));
    let (mask_len, bmap_len) = (image_len(&b, MASK as usize).ok_or_else(|| bad("mask header"))?, image_len(&b, BMAP as usize).ok_or_else(|| bad("bitmap header"))?);
    let ct_at = HEADER as usize + mask_len + bmap_len;
    let ct_size = usize::from(be16(&b, ct_at + 6).ok_or_else(|| bad("colour table header"))?) + 1;
    let pix_at = ct_at + 8 + 8 * ct_size;
    let pix_len = image_len(&b, PMAP as usize).ok_or_else(|| bad("pixel map header"))?;
    if b.len() < pix_at + pix_len { return Err(bad(&format!("{} bytes, {} needed", b.len(), pix_at + pix_len))); }
    let (h, p) = m.alloc_handle(HEADER + (mask_len + bmap_len) as u32, "CIcon")?;
    let (ct, ctp) = m.alloc_handle((8 + 8 * ct_size) as u32, "CIcon pmTable")?;
    let (data, dp) = m.alloc_handle(pix_len as u32, "CIcon iconData")?;
    let e = Refusal::mem("CIcon");
    m.mem.write_bytes(p, &b[..ct_at]).map_err(&e)?;
    m.mem.write_bytes(ctp, &b[ct_at..pix_at]).map_err(&e)?;
    m.mem.write_bytes(dp, &b[pix_at..pix_at + pix_len]).map_err(&e)?;
    m.mem.write_u32(p + PMAP, 0).map_err(&e)?;
    m.mem.write_u32(p + PMAP + crate::cqd::pixmap::PM_TABLE, ct).map_err(&e)?;
    m.mem.write_u32(p + MASK, p + HEADER).map_err(&e)?;
    m.mem.write_u32(p + BMAP, p + HEADER + mask_len as u32).map_err(&e)?;
    m.mem.write_u32(p + DATA, data).map_err(&e)?;
    m.cqd.qd_error = 0;
    Ok(Done { result: h, log: format!("[quickdraw] GetCIcon {id} -> {h:#x} ({pix_len} pixel bytes, {ct_size} colours)") })
}

fn plot_cicon<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (rect, icon) = (c.args.get(0), c.args.get(1));
    let p = m.deref_handle(icon, "PlotCIcon CIconHandle")?;
    let e = Refusal::mem("CIcon");
    let data = m.mem.read_u32(p + DATA).map_err(&e)?;
    let dp = m.deref_handle(data, "PlotCIcon iconData")?;
    // the pixels' address is set for the draw (the ROM locks iconData and points iconPMap.baseAddr at it)
    m.mem.write_u32(p + PMAP, dp).map_err(&e)?;
    m.mem.write_u32(p + MASK, p + HEADER).map_err(&e)?;
    let port = crate::qd::target::current_port(m)?;
    let dst = crate::qd::port::bits_addr(&m.mem, port)?;
    let r = crate::pascal::read_rect(&m.mem, rect)?;
    let ib = crate::pascal::read_rect(&m.mem, p + PMAP + BM_BOUNDS)?;
    let same = (r.right - r.left, r.bottom - r.top) == (ib.right - ib.left, ib.bottom - ib.top);
    let (op, args, how) = if same {
        (0xA817u16, [p + PMAP, p + MASK, dst, p + PMAP + BM_BOUNDS, p + MASK + BM_BOUNDS, rect], "through its mask")
    } else {
        (0xA8EC, [p + PMAP, dst, p + PMAP + BM_BOUNDS, rect, 0, 0], "scaled, without its mask (CopyMask does not scale)")
    };
    m.pascal_call(op, &args).map_err(|e| Refusal(format!("PlotCIcon: {e:?}")))?;
    m.mem.write_u32(p + PMAP, 0).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[quickdraw] PlotCIcon {icon:#x} into ({},{},{},{}) {how}", r.top, r.left, r.bottom, r.right) })
}

fn dispose_cicon<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let p = m.deref_handle(h, "DisposeCIcon")?;
    let e = Refusal::mem("CIcon");
    let (ct, data) = (m.mem.read_u32(p + PMAP + crate::cqd::pixmap::PM_TABLE).map_err(&e)?, m.mem.read_u32(p + DATA).map_err(&e)?);
    for x in [ct, data, h] { if x != 0 { let _ = m.dispose_handle(x); } }
    Ok(Done { result: 0, log: format!("[quickdraw] DisposeCIcon {h:#x}") })
}
