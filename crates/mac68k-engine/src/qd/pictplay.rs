//! `DrawPicture` for a stored picture: the opcode stream is parsed once ([`super::pict`]) and each bitmap opcode is drawn by the ENGINE'S
//! OWN `CopyBits` (the same clipping, colour mapping, transfer modes and scaling as a program's call): the opcode's PackBits rows are
//! unpacked into a temporary guest BitMap / PixMap (with its colour table as a real `CTabHandle`), `CopyBits` copies it from the opcode's source
//! rectangle to its destination rectangle mapped from the picture frame to `dstRect`, and the temporary is freed.
//!
//! What is not drawn is refused BY NAME with the opcode's number and name, never skipped (the picture stops there; what was drawn before
//! stays). Named limits: the picture's `Clip` region is skipped (the port's own clip applies); a mask region (`BitsRgn`, `PackBitsRgn`) is
//! honoured only when the picture is drawn unscaled; transfer modes above 7 (patterns) and direct (16 / 32-bit) pixels are refused.
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
use guestui_abi::GuestUi;
use retro_raster::{Rect, Region};

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
                let unscaled = (fw, fh) == (dw, dh);
                draw_bits(m, h, &b, d, db, unscaled, (i32::from(dst.left) - i32::from(head.left), i32::from(dst.top) - i32::from(head.top)))?;
                drawn += 1;
            }
        }
    }
    let r = dst;
    Ok(if drawn == 0 { format!("[quickdraw] DrawPicture {h:#x} into ({},{},{},{}): no drawing ops", r.top, r.left, r.bottom, r.right) }
       else { format!("[quickdraw] DrawPicture {h:#x} into ({},{},{},{}): {drawn} bitmap(s)", r.top, r.left, r.bottom, r.right) })
}

fn draw_bits<U: GuestUi>(m: &mut Mac<U>, h: u32, b: &Bits, dst: Rect, db: u32, unscaled: bool, shift: (i32, i32)) -> Result<(), Refusal> {
    let mode = b.mode & !64;
    if mode > 7 { return Err(Refusal(format!("DrawPicture {h:#x}: bitmap opcode {:#04x} with transfer mode {} (pattern and arithmetic modes) is not drawn", b.op, b.mode))); }
    let mask = match &b.rgn {
        None => None,
        Some(r) => {
            if !unscaled { return Err(Refusal(format!("DrawPicture {h:#x}: bitmap opcode {:#04x} with a mask region is drawn only unscaled", b.op))); }
            let mut g: Region = super::regions::unpack(r).map_err(|e| Refusal(format!("DrawPicture {h:#x}: mask region: {e}")))?;
            g.translate(shift.0, shift.1);
            Some(g)
        }
    };
    // the temporary BitMap / PixMap: record, then pixels
    let rec = if b.pixmap { crate::cqd::pixmap::PM_SIZE } else { 14 };
    let block = m.mm.new_ptr(&mut m.mem, &mut m.heap, rec + b.pixels.len() as u32, true).map_err(|e| Refusal(format!("DrawPicture: no memory for a {}-byte bitmap (memFullErr {e})", b.pixels.len())))?;
    let ct = if b.pixmap && b.pixel_size > 1 { Some(build_ctab(m, b)?) } else { None };
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

/// The opcode's colour table as a real `CTabHandle`: entry `value` holds that pixel value's colour.
fn build_ctab<U: GuestUi>(m: &mut Mac<U>, b: &Bits) -> Result<u32, Refusal> {
    let n = b.clut.iter().map(|&(v, _)| usize::from(v) + 1).max().unwrap_or(1).max(1 << b.pixel_size.min(8)).min(256);
    let mut colors = alloc::vec![Rgb::BLACK; n];
    for &(v, [r, g, bl]) in &b.clut { if let Some(c) = colors.get_mut(usize::from(v)) { *c = Rgb { r, g, b: bl }; } }
    let seed = m.unique_seed();
    m.new_ctab(&colors, seed, 0)
}
