//! `DrawPicture` for a stored picture: the opcode stream is parsed once ([`super::pict`]) and each bitmap opcode is drawn by the ENGINE'S
//! OWN `CopyBits` (the same clipping, colour mapping, transfer modes and scaling as a program's call): the opcode's PackBits rows are
//! unpacked into a temporary guest BitMap / PixMap (with its colour table as a real `CTabHandle`), `CopyBits` copies it from the opcode's source
//! rectangle to its destination rectangle mapped from the picture frame to `dstRect`, and the temporary is freed.
//!
//! What is not drawn is refused BY NAME with the opcode's number and name, never skipped (the picture stops there; what was drawn before
//! stays). Named limits: the picture's `Clip` region is skipped (the port's own clip applies); a mask region (`BitsRgn`, `PackBitsRgn`,
//! `DirectBitsRgn`) is mapped with the picture (MapRgn: each of its rectangles through the frame -> destination map); transfer modes above 7 (patterns) are refused. Direct pixels (DirectBitsRect /
//! DirectBitsRgn, 16 and 32 bits) are drawn like the others. RGBFgCol / RGBBkCol / OpColor / HiliteColor are stepped over: the port's own
//! pens stay (named: a picture that recolours its own bitmaps is drawn in the port's colours).
use super::copybits::copy_through;
use super::pict::{self, Bits, Op};
use super::port::{bits_addr, raster};
use super::target::current_port;
use crate::cqd::color::Rgb;
use crate::machine::Mac;
use crate::pascal::read_rect;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_raster::{Rect, Region};

/// Draw picture handle `h` into `dst` (a Rect address) in the current port. The log line says what was drawn.
pub(crate) fn play<U: GuestUi>(m: &mut Mac<U>, h: u32, dst_at: u32) -> Result<String, Refusal> {
    let bytes = m.handle_bytes(h).ok_or_else(|| Refusal(format!("DrawPicture: {h:#x} is not a Picture handle")))?;
    let head = pict::header(&bytes).map_err(|e| Refusal(format!("DrawPicture {h:#x}: {e}")))?;
    let dst = read_rect(&m.mem, dst_at)?;
    let (fw, fh) = (i32::from(head.right) - i32::from(head.left), i32::from(head.bottom) - i32::from(head.top));
    let (dw, dh) = (i32::from(dst.right) - i32::from(dst.left), i32::from(dst.bottom) - i32::from(dst.top));
    let port = current_port(m)?;
    let db = bits_addr(&m.mem, port)?;
    let map_x = |x: i16| i32::from(dst.left) + if fw == 0 { 0 } else { (i32::from(x) - i32::from(head.left)) * dw / fw };
    let map_y = |y: i16| i32::from(dst.top) + if fh == 0 { 0 } else { (i32::from(y) - i32::from(head.top)) * dh / fh };
    let (mut at, mut drawn) = (head.body, 0usize);
    loop {
        match pict::next(&bytes, &mut at, head.v2).map_err(|e| Refusal(format!("DrawPicture {h:#x}: malformed picture: {e}")))? {
            Op::Skip => {}
            Op::End => break,
            Op::Unsupported(op, name) => return Err(Refusal(format!("DrawPicture {h:#x}: picture opcode {op:#06x} ({name}) is not interpreted (only bitmap opcodes are drawn; {drawn} drawn before it)"))),
            Op::Bits(b) => {
                let d = Rect::new(map_x(b.dst[1]), map_y(b.dst[0]), map_x(b.dst[3]), map_y(b.dst[2]));
                let map = |x: i32, y: i32| (map_x(x as i16), map_y(y as i16));
                draw_bits(m, h, &b, d, db, &map)?;
                drawn += 1;
            }
            Op::CompressedQuickTime { dst: qt_dst, mode, codec, data, .. } => {
                let img = match m.datatype_host.as_deref() {
                    Some(host) => match host.decode_image(&data, 0, 0) {
                        Some(img) => { m.native_datatype_decodes += 1; Some(img) }
                        None => heirloom_common::datatype::decode_with_fallback(None, &data, 0, 0),
                    },
                    None => heirloom_common::datatype::decode_with_fallback(None, &data, 0, 0),
                };
                if let Some(img) = img {
                    let (w, ht) = (img.width as i16, img.height as i16);
                    let mut raw = alloc::vec::Vec::with_capacity(img.pixels.len() * 4);
                    for px in &img.pixels {
                        // QuickDraw 32-bit direct PixMap is big-endian ARGB (pad/A, R, G, B)
                        raw.extend_from_slice(&px.to_be_bytes());
                    }
                    let bits = Bits {
                        op: 0x8200,
                        pixmap: true,
                        row_bytes: (img.width * 4) as u16,
                        bounds: [0, 0, ht, w],
                        pixel_size: 32,
                        clut: alloc::vec::Vec::new(),
                        src: [0, 0, ht, w],
                        dst: qt_dst,
                        mode,
                        rgn: None,
                        pixels: raw,
                    };
                    let d = Rect::new(map_x(qt_dst[1]), map_y(qt_dst[0]), map_x(qt_dst[3]), map_y(qt_dst[2]));
                    let map = |x: i32, y: i32| (map_x(x as i16), map_y(y as i16));
                    draw_bits(m, h, &bits, d, db, &map)?;
                    drawn += 1;
                } else {
                    let tag = core::str::from_utf8(&codec).unwrap_or("????");
                    return Err(Refusal(format!("DrawPicture {h:#x}: CompressedQuickTime codec '{tag}' ({} bytes) could not be decoded by native datatype host", data.len())));
                }
            }
        }
    }
    let r = dst;
    Ok(if drawn == 0 { format!("[quickdraw] DrawPicture {h:#x} into ({},{},{},{}): no drawing ops", r.top, r.left, r.bottom, r.right) }
       else { format!("[quickdraw] DrawPicture {h:#x} into ({},{},{},{}): {drawn} bitmap(s)", r.top, r.left, r.bottom, r.right) })
}

/// `map`: a picture-frame point to the destination (local coordinates), the same linear map the bitmap's rectangles take.
fn draw_bits<U: GuestUi>(m: &mut Mac<U>, h: u32, b: &Bits, dst: Rect, db: u32, map: &dyn Fn(i32, i32) -> (i32, i32)) -> Result<(), Refusal> {
    // In PICT bitmap opcodes (BitsRect/PackBitsRect), classic QuickDraw ignores bit 3 (the pattern bit:
    // patCopy..notPatBic 8..=15 map onto srcCopy..notSrcBic 0..=7, Inside Macintosh: Imaging with QuickDraw).
    let raw = b.mode & !64;
    let mode = if (8..=15).contains(&raw) { raw & 7 } else { raw };
    if mode > 7 && !(32..=39).contains(&mode) { return Err(Refusal(format!("DrawPicture {h:#x}: bitmap opcode {:#04x} with transfer mode {} (hilite or unsupported mode) is not drawn", b.op, b.mode))); }
    let mask = match &b.rgn {
        None => None,
        Some(r) => {
            // MapRgn: every rectangle of the region through the frame -> destination map (a translation when unscaled)
            let g: Region = super::regions::unpack(r).map_err(|e| Refusal(format!("DrawPicture {h:#x}: mask region: {e}")))?;
            let mut out = Region::new();
            for r in g.rects() { let (a, b) = (map(r.x0, r.y0), map(r.x1, r.y1)); out.or_rect(Rect::new(a.0, a.1, b.0, b.1)); }
            Some(out)
        }
    };
    // the temporary BitMap / PixMap: record, then pixels
    let rec = if b.pixmap { crate::cqd::pixmap::PM_SIZE } else { 14 };
    let block = m.mm.new_ptr(&mut m.mem, &mut m.heap, rec + b.pixels.len() as u32, true).map_err(|e| Refusal(format!("DrawPicture: no memory for a {}-byte bitmap (memFullErr {e})", b.pixels.len())))?;
    let ct = if b.pixmap && b.pixel_size > 1 && b.pixel_size <= 8 { Some(build_ctab(m, b)?) } else { None }; // a direct map has no table
    let result = (|| {
        let e = Refusal::mem("DrawPicture bitmap");
        m.mem.write_u32(block, block + rec).map_err(&e)?;
        m.mem.write_u16(block + 4, b.row_bytes | if b.pixmap { 0x8000 } else { 0 }).map_err(&e)?;
        for (i, v) in b.bounds.iter().enumerate() { m.mem.write_u16(block + 6 + 2 * i as u32, *v as u16).map_err(&e)?; }
        if b.pixmap {
            m.mem.write_u16(block + crate::cqd::pixmap::PM_PIXEL_SIZE, b.pixel_size).map_err(&e)?;
            if let Some(c) = ct { m.mem.write_u32(block + crate::cqd::pixmap::PM_TABLE, c).map_err(&e)?; }
        }
        m.mem.write_bytes(block + rec, &b.pixels).map_err(&e)?;
        let (sr, dr) = (raster(crate::pascal::Rect { top: b.src[0], left: b.src[1], bottom: b.src[2], right: b.src[3] }), dst);
        copy_through(m, "DrawPicture", block, db, sr, dr, mode, mask).map(|_| ())
    })();
    if let Some(c) = ct { let _ = m.dispose_handle(c); }
    let _ = m.mm.dispose_ptr(&mut m.heap, block);
    result
}

/// Blit an ARGB32 (`0xAARRGGBB`) image decoded by `datatype.library` into QuickDraw port `port` at
/// `dst_tlbr = [top, left, bottom, right]` (reusing `draw_bits` -> `CopyBits`).
pub fn draw_argb32_to_port<U: GuestUi>(
    m: &mut Mac<U>,
    port: u32,
    w: u32,
    h: u32,
    pixels: &[u32],
    dst_tlbr: [i16; 4],
) -> Result<(), Refusal> {
    let db = bits_addr(&m.mem, port)?;
    let (wi, hi) = (w.min(i16::MAX as u32) as i16, h.min(i16::MAX as u32) as i16);
    let mut raw = alloc::vec::Vec::with_capacity(pixels.len() * 4);
    for px in pixels {
        raw.extend_from_slice(&px.to_be_bytes());
    }
    let dst = Rect::new(i32::from(dst_tlbr[1]), i32::from(dst_tlbr[0]), i32::from(dst_tlbr[3]), i32::from(dst_tlbr[2]));
    let bits = Bits {
        op: 0x8200,
        pixmap: true,
        row_bytes: (w * 4) as u16,
        bounds: [0, 0, hi, wi],
        pixel_size: 32,
        clut: alloc::vec::Vec::new(),
        src: [0, 0, hi, wi],
        dst: dst_tlbr,
        mode: 0,
        rgn: None,
        pixels: raw,
    };
    let id = |x: i32, y: i32| (x, y);
    draw_bits(m, 0, &bits, dst, db, &id)
}

/// The opcode's colour table as a real `CTabHandle`: entry `value` holds that pixel value's colour.
fn build_ctab<U: GuestUi>(m: &mut Mac<U>, b: &Bits) -> Result<u32, Refusal> {
    let n = b.clut.iter().map(|&(v, _)| usize::from(v) + 1).max().unwrap_or(1).max(1 << b.pixel_size.min(8)).min(256);
    let mut colors = alloc::vec![Rgb::BLACK; n];
    for &(v, [r, g, bl]) in &b.clut { if let Some(c) = colors.get_mut(usize::from(v)) { *c = Rgb { r, g, b: bl }; } }
    let seed = m.unique_seed();
    m.new_ctab(&colors, seed, 0)
}
