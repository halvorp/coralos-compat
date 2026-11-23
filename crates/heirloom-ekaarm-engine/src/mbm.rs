//! Symbian `.mbm` multi-bitmap decoder and built-in 5x7 fallback bitmap font (`fbs.rs` split).
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::fbs::ECOLOR_16MU;

fn color256(idx: u8) -> u32 {
    let i = idx as usize;
    if i < 108 {
        let b = (i % 6) as u32 * 51;
        let g = ((i / 6) % 6) as u32 * 51;
        let r = (i / 36) as u32 * 51;
        (r << 16) | (g << 8) | b
    } else if i >= 148 {
        let j = i - 40;
        let b = (j % 6) as u32 * 51;
        let g = ((j / 6) % 6) as u32 * 51;
        let r = (j / 36) as u32 * 51;
        (r << 16) | (g << 8) | b
    } else {
        let v = ((i - 108) * 6).min(255) as u32;
        (v << 16) | (v << 8) | v
    }
}

/// Decode bitmap `index` from a Symbian `.mbm` multi-bitmap file into `(width, height, is_gray, pixels)`.
pub fn decode_mbm(data: &[u8], index: usize) -> Result<(i32, i32, bool, Vec<u32>), String> {
    let (w, h, mode, pixels) = decode_mbm_mode(data, index)?;
    Ok((w, h, matches!(mode, 1 | 2 | 3 | 4), pixels))
}

/// Decode bitmap `index` from a Symbian `.mbm` multi-bitmap file into `(width, height, display_mode, pixels)`.
pub fn decode_mbm_mode(data: &[u8], index: usize) -> Result<(i32, i32, u32, Vec<u32>), String> {
    let rd32 = |at: usize| -> Result<u32, String> {
        let b = data.get(at..at + 4).ok_or_else(|| String::from("MBM truncated"))?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let uid1 = rd32(0)?;
    let uid2 = rd32(4)?;
    if uid1 != 0x1000_0037 || uid2 != 0x1000_0042 {
        return Err(format!("not a Symbian MBM file (uids {uid1:#x}, {uid2:#x})"));
    }
    let trailer = rd32(16)? as usize;
    let count = rd32(trailer)? as usize;
    if index >= count {
        return Err(format!("MBM bitmap index {index} out of range (count {count})"));
    }
    let off = rd32(trailer + 4 + 4 * index)? as usize;
    let bmp_size = rd32(off)? as usize;
    let hdr_size = rd32(off + 4)? as usize;
    let w = rd32(off + 8)? as i32;
    let h = rd32(off + 12)? as i32;
    let bpp = rd32(off + 24)?;
    let color = rd32(off + 28)?;
    let comp = rd32(off + 36)?;
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
        return Err(format!("MBM invalid dimensions {w}x{h}"));
    }
    let pay_start = off + hdr_size;
    let pay_end = (off + bmp_size).min(data.len());
    let pay = data.get(pay_start..pay_end).ok_or_else(|| String::from("MBM payload out of bounds"))?;
    let row_bytes_uncomp = match bpp {
        1 => ((w as usize + 31) / 32) * 4,
        2 => ((w as usize * 2 + 31) / 32) * 4,
        4 => ((w as usize * 4 + 31) / 32) * 4,
        8 => ((w as usize + 3) / 4) * 4,
        12 | 16 => ((w as usize * 2 + 3) / 4) * 4,
        24 => ((w as usize * 3 + 3) / 4) * 4,
        32 => w as usize * 4,
        other => return Err(format!("MBM unsupported bpp {other}")),
    };
    let min_row = match bpp {
        1 => (w as usize + 7) / 8,
        2 => (w as usize * 2 + 7) / 8,
        4 => (w as usize * 4 + 7) / 8,
        8 => w as usize,
        12 | 16 => w as usize * 2,
        24 => w as usize * 3,
        32 => w as usize * 4,
        _ => row_bytes_uncomp,
    };
    let cap = (row_bytes_uncomp + 12) * h as usize;
    let raw: Vec<u8> = match comp {
        0 => pay.to_vec(),
        1 => {
            // EByteRLECompression
            let mut out = Vec::with_capacity(cap);
            let mut i = 0;
            while i < pay.len() && out.len() < cap {
                let c = pay[i] as i8;
                i += 1;
                if c >= 0 {
                    let n = c as usize + 1;
                    let v = *pay.get(i).unwrap_or(&0);
                    i += 1;
                    for _ in 0..n { out.push(v); }
                } else {
                    let n = (-(c as i32)) as usize;
                    let end = (i + n).min(pay.len());
                    out.extend_from_slice(&pay[i..end]);
                    i = end;
                }
            }
            out
        }
        2 => {
            // ETwelveBitRLECompression: 16-bit LE word, low 12 bits = RGB444, high 4 bits = count - 1
            let mut out = Vec::with_capacity(cap);
            let mut i = 0;
            while i + 1 < pay.len() && out.len() < cap {
                let word = u16::from_le_bytes([pay[i], pay[i + 1]]);
                i += 2;
                let n = ((word >> 12) as usize) + 1;
                let lo = (word & 0xFF) as u8;
                let hi = ((word >> 8) & 0x0F) as u8;
                for _ in 0..n { out.push(lo); out.push(hi); }
            }
            out
        }
        3 => {
            // ESixteenBitRLECompression
            let mut out = Vec::with_capacity(cap);
            let mut i = 0;
            while i < pay.len() && out.len() < cap {
                let c = pay[i] as i8;
                i += 1;
                if c >= 0 {
                    let n = c as usize + 1;
                    if i + 1 >= pay.len() { break; }
                    let (b0, b1) = (pay[i], pay[i + 1]);
                    i += 2;
                    for _ in 0..n { out.push(b0); out.push(b1); }
                } else {
                    let n = (-(c as i32)) as usize * 2;
                    let end = (i + n).min(pay.len());
                    out.extend_from_slice(&pay[i..end]);
                    i = end;
                }
            }
            out
        }
        4 => {
            // ETwentyFourBitRLECompression
            let mut out = Vec::with_capacity(cap);
            let mut i = 0;
            while i < pay.len() && out.len() < cap {
                let c = pay[i] as i8;
                i += 1;
                if c >= 0 {
                    let n = c as usize + 1;
                    if i + 2 >= pay.len() { break; }
                    let (b0, b1, b2) = (pay[i], pay[i + 1], pay[i + 2]);
                    i += 3;
                    for _ in 0..n { out.push(b0); out.push(b1); out.push(b2); }
                } else {
                    let n = (-(c as i32)) as usize * 3;
                    let end = (i + n).min(pay.len());
                    out.extend_from_slice(&pay[i..end]);
                    i = end;
                }
            }
            out
        }
        other => return Err(format!("MBM unsupported compression {other}")),
    };

    let mut pixels = vec![0u32; w as usize * h as usize];
    let stride = if raw.len() >= h as usize * min_row && raw.len() % h as usize == 0 {
        raw.len() / h as usize
    } else {
        row_bytes_uncomp
    };
    for y in 0..h as usize {
        let row = y * stride;
        for x in 0..w as usize {
            let rgb = match bpp {
                1 => {
                    let byte = *raw.get(row + x / 8).unwrap_or(&0);
                    if (byte >> (x & 7)) & 1 != 0 { 0x00FF_FFFF } else { 0x0000_0000 }
                }
                8 => {
                    let v = *raw.get(row + x).unwrap_or(&0);
                    if color == 0 {
                        let g = u32::from(v);
                        (g << 16) | (g << 8) | g
                    } else {
                        color256(v)
                    }
                }
                12 => {
                    let p = row + x * 2;
                    let word = u16::from_le_bytes([*raw.get(p).unwrap_or(&0), *raw.get(p + 1).unwrap_or(&0)]);
                    let r = (((word >> 8) & 0xF) as u32) * 17;
                    let g = (((word >> 4) & 0xF) as u32) * 17;
                    let b = ((word & 0xF) as u32) * 17;
                    (r << 16) | (g << 8) | b
                }
                16 => {
                    let p = row + x * 2;
                    let word = u16::from_le_bytes([*raw.get(p).unwrap_or(&0), *raw.get(p + 1).unwrap_or(&0)]);
                    let r = (((word >> 11) & 0x1F) as u32 * 255) / 31;
                    let g = (((word >> 5) & 0x3F) as u32 * 255) / 63;
                    let b = ((word & 0x1F) as u32 * 255) / 31;
                    (r << 16) | (g << 8) | b
                }
                24 => {
                    let p = row + x * 3;
                    let b = u32::from(*raw.get(p).unwrap_or(&0));
                    let g = u32::from(*raw.get(p + 1).unwrap_or(&0));
                    let r = u32::from(*raw.get(p + 2).unwrap_or(&0));
                    (r << 16) | (g << 8) | b
                }
                32 => {
                    let p = row + x * 4;
                    let b = u32::from(*raw.get(p).unwrap_or(&0));
                    let g = u32::from(*raw.get(p + 1).unwrap_or(&0));
                    let r = u32::from(*raw.get(p + 2).unwrap_or(&0));
                    (r << 16) | (g << 8) | b
                }
                _ => 0,
            };
            pixels[y * w as usize + x] = rgb;
        }
    }
    let mode = match (bpp, color == 0) {
        (1, _) => 1,      // EGray2
        (2, _) => 2,      // EGray4
        (4, true) => 3,   // EGray16
        (4, false) => 5,  // EColor16
        (8, true) => 4,   // EGray256
        (8, false) => 6,  // EColor256
        (12, _) => 10,    // EColor4K
        (16, _) => 7,     // EColor64K
        (24, _) => 8,     // EColor16M
        _ => ECOLOR_16MU, // EColor16MU
    };
    Ok((w, h, mode, pixels))
}

/// Vtable for `CFbsBitmapDevice` and `CFbsScreenDevice`.

pub(crate) fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x06, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
        '*' => [0x00, 0x15, 0x0E, 0x1F, 0x0E, 0x15, 0x00],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '\'' => [0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
        'Ä' | 'ä' => [0x0A, 0x00, 0x0E, 0x11, 0x1F, 0x11, 0x11],
        'Ö' | 'ö' => [0x0A, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E],
        'Ü' | 'ü' => [0x0A, 0x00, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'ß' => [0x0E, 0x11, 0x12, 0x14, 0x12, 0x11, 0x1E],
        ' ' => [0; 7],
        _ => [0; 7],
    }
}

