//! The PICT format as pure functions over bytes (Inside Macintosh V "QuickDraw: Pictures"; Executor's MIT `qPicture.cpp` for the opcode
//! sizes): the header, the opcode stream of version 1 (one-byte opcodes) and version 2 (two-byte, word-aligned, the `$0C00` header
//! opcode), and the bitmap opcodes `BitsRect` $90, `BitsRgn` $91, `PackBitsRect` $98, `PackBitsRgn` $99 with their BitMap or PixMap,
//! colour table, rectangles, mode and PackBits rows. Playback is [`super::pictplay`]; one parser serves it.
//!
//! Opcodes this parser steps over without drawing (they change no pixel): `NOP`, `Clip` ($01: skipped, a named limit of playback), the short
//! and long comments, `DefHilite`, the version and header opcodes, and the pen / text state opcodes that carry no drawing. Every other opcode
//! is reported by NUMBER and NAME so playback can refuse it, never skip it.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The picture's frame and where its opcode stream starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header { pub top: i16, pub left: i16, pub bottom: i16, pub right: i16, pub v2: bool, pub body: usize }

/// `picSize` (2), `picFrame` (8), then `$11 $01` (version 1) or `$0011 $02FF` (version 2).
pub fn header(b: &[u8]) -> Result<Header, String> {
    let r = |i: usize| retro_common::bytes::be16(b, i).map(|v| v as i16).ok_or_else(|| String::from("shorter than its picFrame"));
    let (top, left, bottom, right) = (r(2)?, r(4)?, r(6)?, r(8)?);
    match (b.get(10), b.get(11), retro_common::bytes::be16(b, 10), retro_common::bytes::be16(b, 12)) {
        (Some(0x11), Some(0x01), _, _) => Ok(Header { top, left, bottom, right, v2: false, body: 12 }),
        (_, _, Some(0x0011), Some(0x02FF)) => Ok(Header { top, left, bottom, right, v2: true, body: 14 }),
        _ => Err(format!("version opcode {:02x}{:02x}{:02x}{:02x} is neither $1101 (version 1) nor $001102FF (version 2)", b.get(10).copied().unwrap_or(0), b.get(11).copied().unwrap_or(0), b.get(12).copied().unwrap_or(0), b.get(13).copied().unwrap_or(0))),
    }
}

/// One bitmap opcode, unpacked: rows are `row_bytes` each, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bits {
    pub op: u16,
    pub pixmap: bool,
    pub row_bytes: u16,
    pub bounds: [i16; 4],
    pub pixel_size: u16,
    /// A PixMap's colour table: (value, r, g, b) per entry.
    pub clut: Vec<(u16, [u16; 3])>,
    pub src: [i16; 4],
    pub dst: [i16; 4],
    pub mode: u16,
    /// The mask region record of `BitsRgn` / `PackBitsRgn` (rgnSize included), if the opcode has one.
    pub rgn: Option<Vec<u8>>,
    pub pixels: Vec<u8>,
}

/// What the next opcode is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// An opcode that draws nothing.
    Skip,
    End,
    Bits(Bits),
    /// An opcode this parser cannot step over or playback does not draw: its number and name.
    Unsupported(u16, &'static str),
}

/// The names of the opcodes playback refuses, for the refusal text.
pub fn op_name(op: u16) -> &'static str {
    match op {
        0x02 => "BkPat", 0x03 => "TxFont", 0x07 => "PnSize", 0x08 => "PnMode", 0x09 => "PnPat", 0x0A => "FillPat", 0x0B => "OvSize", 0x0C => "Origin",
        0x0D => "TxSize", 0x20 => "Line", 0x21 => "LineFrom", 0x22 => "ShortLine", 0x23 => "ShortLineFrom", 0x28 => "LongText", 0x29 => "DHText", 0x2A => "DVText",
        0x2B => "DHDVText", 0x30 => "FrameRect", 0x31 => "PaintRect", 0x32 => "EraseRect", 0x33 => "InvertRect", 0x34 => "FillRect", 0x40 => "FrameRRect",
        0x50 => "FrameOval", 0x60 => "FrameArc", 0x70 => "FramePoly", 0x80 => "FrameRgn", 0x9A => "DirectBitsRect", 0x9B => "DirectBitsRgn",
        0x2C => "FontName", 0x15 => "PnLocHFrac", 0x1A => "RGBFgCol", 0x1B => "RGBBkCol", 0x1D => "OpColor", 0x2E => "GlyphState", 0xB0..=0xCF => "an unnamed opcode",
        _ => "an unnamed opcode",
    }
}

/// Unpack `rows` PackBits rows of `row_bytes` each from `b` starting at `at` (packed rows carry a 1-byte count when `row_bytes <= 250`, else 2).
fn unpack_rows(b: &[u8], at: &mut usize, rows: usize, row_bytes: usize, packed: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(rows * row_bytes);
    for r in 0..rows {
        if !packed || row_bytes < 8 {
            // an unpacked opcode, or a row too short to pack: raw bytes (rowBytes < 8 is never packed)
            let chunk = b.get(*at..*at + row_bytes).ok_or_else(|| format!("row {r} runs past the end of the picture"))?;
            out.extend_from_slice(chunk);
            *at += row_bytes;
            continue;
        }
        let n = if row_bytes > 250 { let v = retro_common::bytes::be16(b, *at).ok_or("row count past the end")? as usize; *at += 2; v } else { let v = *b.get(*at).ok_or("row count past the end")? as usize; *at += 1; v };
        let packed_row = b.get(*at..*at + n).ok_or_else(|| format!("packed row {r} runs past the end of the picture"))?;
        *at += n;
        let start = out.len();
        let mut i = 0;
        while i < packed_row.len() && out.len() - start < row_bytes {
            let c = packed_row[i] as i8;
            i += 1;
            if c >= 0 {
                let k = c as usize + 1;
                out.extend_from_slice(packed_row.get(i..i + k).ok_or("a literal run runs past its packed row")?);
                i += k;
            } else {
                let k = (1 - i16::from(c)) as usize;
                let v = *packed_row.get(i).ok_or("a repeat run has no byte")?;
                i += 1;
                out.extend(core::iter::repeat(v).take(k));
            }
        }
        if out.len() - start != row_bytes { return Err(format!("packed row {r} unpacks to {} bytes, not {row_bytes}", out.len() - start)); }
    }
    Ok(out)
}

/// Read the opcode at `*at` and step past it. `Err` = the stream is malformed (named).
pub fn next(b: &[u8], at: &mut usize, v2: bool) -> Result<Op, String> {
    if v2 && *at % 2 == 1 { *at += 1; } // version 2 opcodes are word aligned
    let op = if v2 { let v = retro_common::bytes::be16(b, *at).ok_or("no end opcode")?; *at += 2; v } else { let v = *b.get(*at).ok_or("no end opcode")?; *at += 1; u16::from(v) };
    let skip = |at: &mut usize, n: usize| -> Result<Op, String> { if *at + n > b.len() { Err(format!("opcode {op:#x} runs past the end")) } else { *at += n; Ok(Op::Skip) } };
    match op {
        0x00 | 0x1E => Ok(Op::Skip),
        0xFF => Ok(Op::End), // $FF in version 1, $00FF in version 2
        0x01 => { let n = retro_common::bytes::be16(b, *at).ok_or("clip region has no size")? as usize; skip(at, n) } // Clip: region record
        0x0C00 => skip(at, 24), // v2 header opcode: version, bbox / resolution
        0xA0 => skip(at, 2),
        0xA1 => { let n = retro_common::bytes::be16(b, *at + 2).ok_or("long comment has no size")? as usize; skip(at, 4 + n + if v2 { n & 1 } else { 0 }) }
        0x90 | 0x91 | 0x98 | 0x99 => {
            let packed = op == 0x98 || op == 0x99;
            let has_rgn = op == 0x91 || op == 0x99;
            let rb_raw = retro_common::bytes::be16(b, *at).ok_or("bitmap opcode has no rowBytes")?;
            let pixmap = rb_raw & 0x8000 != 0;
            let row_bytes = rb_raw & 0x3FFF;
            let mut i = *at + 2;
            let rect = |i: &mut usize| -> Result<[i16; 4], String> { let r = [retro_common::bytes::be16(b, *i), retro_common::bytes::be16(b, *i + 2), retro_common::bytes::be16(b, *i + 4), retro_common::bytes::be16(b, *i + 6)]; *i += 8; Ok([r[0].ok_or("rect past the end")? as i16, r[1].ok_or("rect past the end")? as i16, r[2].ok_or("rect past the end")? as i16, r[3].ok_or("rect past the end")? as i16]) };
            let bounds = rect(&mut i)?;
            let (mut pixel_size, mut clut) = (1u16, Vec::new());
            if pixmap {
                // after the bounds: pmVersion 2, packType 2, packSize 4, hRes 4, vRes 4, pixelType 2, pixelSize 2, cmpCount 2, cmpSize 2,
                // planeBytes 4, pmTable 4, pmReserved 4 = 36 bytes (baseAddr, rowBytes and bounds are the 14 the PICT form shortens to 10)
                pixel_size = retro_common::bytes::be16(b, i + 18).ok_or("PixMap past the end")?;
                i += 36;
                // colour table: ctSeed 4, ctFlags 2, ctSize 2, (ctSize + 1) ColorSpecs of 8 bytes
                let ct_size = retro_common::bytes::be16(b, i + 6).ok_or("colour table past the end")? as usize;
                i += 8;
                for _ in 0..=ct_size {
                    let v = retro_common::bytes::be16(b, i).ok_or("colour table past the end")?;
                    clut.push((v, [retro_common::bytes::be16(b, i + 2).ok_or("colour table past the end")?, retro_common::bytes::be16(b, i + 4).ok_or("colour table past the end")?, retro_common::bytes::be16(b, i + 6).ok_or("colour table past the end")?]));
                    i += 8;
                }
            }
            let src = rect(&mut i)?;
            let dst = rect(&mut i)?;
            let mode = retro_common::bytes::be16(b, i).ok_or("mode past the end")?;
            i += 2;
            let rgn = if has_rgn { let n = retro_common::bytes::be16(b, i).ok_or("mask region past the end")? as usize; let r = b.get(i..i + n).ok_or("mask region past the end")?.to_vec(); i += n; Some(r) } else { None };
            if pixel_size > 8 { *at = i; return Ok(Op::Unsupported(op, "bitmap opcode with direct (16 / 32-bit) pixels")); }
            let rows = (i32::from(bounds[2]) - i32::from(bounds[0])).max(0) as usize;
            let pixels = unpack_rows(b, &mut i, rows, usize::from(row_bytes), packed)?;
            *at = i;
            Ok(Op::Bits(Bits { op, pixmap, row_bytes, bounds, pixel_size, clut, src, dst, mode, rgn, pixels }))
        }
        0x9A | 0x9B => Ok(Op::Unsupported(op, op_name(op))),
        other => Ok(Op::Unsupported(other, op_name(other))),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn w(v: &mut Vec<u8>, xs: &[u16]) { for x in xs { v.extend(x.to_be_bytes()); } }

    /// A version-2 picture, frame (10,20)-(12,28): `$0C00` header opcode, a PackBitsRect of an 8-bit 8x2 PixMap with a 2-entry table whose
    /// rows are a repeat run (all 1) and a literal run (0,1,0,1,...), then end.
    pub(crate) fn v2_packbits_picture() -> Vec<u8> {
        let mut b = Vec::new();
        w(&mut b, &[0, 10, 20, 12, 28, 0x0011, 0x02FF, 0x0C00]);
        b.extend([0u8; 24]);
        w(&mut b, &[0x0098, 0x8008, 0, 0, 2, 8]); // opcode, rowBytes (PixMap flag | 8), bounds 0,0,2,8
        w(&mut b, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0]); // the 36 bytes after the bounds: pixelSize 8 is the 10th word (offset 18)
        w(&mut b, &[0, 0, 0, 1]); // ctSeed (long), ctFlags 0, ctSize 1 (two entries)
        w(&mut b, &[0, 0xFFFF, 0xFFFF, 0xFFFF, 1, 0, 0, 0]); // entry 0 white, entry 1 black
        w(&mut b, &[0, 0, 2, 8, 10, 20, 12, 28, 0]); // srcRect, dstRect, mode srcCopy
        b.extend([2, 0xF9, 1]); // row 0: count 2: repeat run (1 - (-7) = 8) of byte 1
        b.extend([9, 7, 0, 1, 0, 1, 0, 1, 0, 1]); // row 1: count 9: literal run of 8 bytes
        b.push(0); // the data is 13 bytes: pad to the next word before the next opcode
        w(&mut b, &[0x00FF]);
        b
    }

    #[test]
    fn the_v2_header_and_a_packbitsrect_unpack_to_their_rows_with_the_colour_table() {
        let b = v2_packbits_picture();
        let h = header(&b).unwrap();
        assert_eq!((h.top, h.left, h.bottom, h.right, h.v2, h.body), (10, 20, 12, 28, true, 14));
        let mut at = h.body;
        assert_eq!(next(&b, &mut at, true).unwrap(), Op::Skip, "the $0C00 header opcode");
        let Op::Bits(bits) = next(&b, &mut at, true).unwrap() else { panic!("a bitmap opcode") };
        assert_eq!((bits.op, bits.pixmap, bits.row_bytes, bits.pixel_size, bits.bounds, bits.src, bits.dst, bits.mode), (0x98, true, 8, 8, [0, 0, 2, 8], [0, 0, 2, 8], [10, 20, 12, 28], 0));
        assert_eq!(bits.clut, [(0, [0xFFFF; 3]), (1, [0, 0, 0])]);
        assert_eq!(bits.pixels, [1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, 0, 1], "a repeat run and a literal run");
        assert_eq!(next(&b, &mut at, true).unwrap(), Op::End);
    }

    #[test]
    fn a_malformed_row_and_an_unknown_opcode_are_named_not_skipped() {
        let mut b = v2_packbits_picture();
        let at_row1 = b.windows(4).position(|w| w == [9, 7, 0, 1]).unwrap();
        b[at_row1] = 99; // row 1's packed count claims 99 bytes
        let (h, mut at) = (header(&b).unwrap(), 14);
        let _ = next(&b, &mut at, true);
        assert!(next(&b, &mut at, true).unwrap_err().contains("past the end"), "a count past the end of the picture");
        // a PaintRect ($0031) is not skipped: it is reported by number and name
        let mut p = Vec::new();
        w(&mut p, &[0, 0, 0, 5, 5, 0x0011, 0x02FF, 0x0031]);
        let mut at = 14;
        assert_eq!(next(&p, &mut at, true).unwrap(), Op::Unsupported(0x31, "PaintRect"));
        assert!(header(&[0; 14]).unwrap_err().contains("neither"), "no version opcode");
        let _ = h;
    }

    #[test]
    fn a_version_1_bitmap_opcode_has_a_bitmap_not_a_pixmap() {
        // v1: $11 $01, BitsRect $90 of a 1-bit 8x1 BitMap (rowBytes 2 unpacked), end
        let mut b = alloc::vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 8, 0x11, 0x01, 0x90];
        w(&mut b, &[2, 0, 0, 1, 8, 0, 0, 1, 8, 0, 0, 1, 8, 1]); // rowBytes, bounds, srcRect, dstRect, mode 1 (srcOr)
        b.extend([0xA5, 0x00, 0xFF]);
        let h = header(&b).unwrap();
        assert!(!h.v2);
        let mut at = h.body;
        let Op::Bits(bits) = next(&b, &mut at, false).unwrap() else { panic!("bits") };
        assert_eq!((bits.pixmap, bits.pixel_size, bits.mode, bits.pixels.as_slice()), (false, 1, 1, &[0xA5u8, 0x00][..]));
        assert_eq!(next(&b, &mut at, false).unwrap(), Op::End);
    }
}
