//! The PNG icon (OS4 / AROS: the whole `.info` is a PNG, the icon's DiskObject data in an `icOn` chunk; a second PNG after the
//! first, the selected image, is ignored). The decoder reads what icons use: non-interlaced, colour types 0 (grey), 2 (RGB),
//! 3 (palette, with `tRNS`), 4 (grey + alpha) and 6 (RGBA) at 8 bits per sample, palette images at 1, 2, 4 or 8 bits, and
//! 16-bit samples (their high byte). Anything else is refused by name (PNG specification, ISO/IEC 15948).
use crate::IconImage;
use alloc::vec::Vec;
use heirloom_common::bytes::be32;

pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// The first PNG in `b` to straight RGBA.
pub fn decode(b: &[u8]) -> Result<IconImage, &'static str> {
    if !b.starts_with(&SIGNATURE) {
        return Err("not a PNG");
    }
    let bad = "a PNG icon cut short or malformed";
    let (mut at, mut ihdr, mut plte, mut trns, mut idat) = (8usize, None, &[][..], &[][..], Vec::new());
    loop {
        let len = be32(b, at).ok_or(bad)? as usize;
        let id = b.get(at + 4..at + 8).ok_or(bad)?;
        let data = b.get(at + 8..at + 8 + len).ok_or(bad)?;
        match id {
            b"IHDR" => ihdr = Some(data),
            b"PLTE" => plte = data,
            b"tRNS" => trns = data,
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let h = ihdr.filter(|h| h.len() >= 13).ok_or(bad)?;
    let (w, ht, depth, ctype, interlace) = (be32(h, 0).ok_or(bad)?, be32(h, 4).ok_or(bad)?, h[8] as usize, h[9], h[12]);
    if interlace != 0 {
        return Err("an interlaced PNG icon (not read)");
    }
    if w == 0 || ht == 0 || w > 1024 || ht > 1024 {
        return Err(bad);
    }
    let channels = match ctype { 0 => 1, 2 => 3, 3 => 1, 4 => 2, 6 => 4, _ => return Err(bad) };
    if !(depth == 8 || depth == 16 || (ctype == 3 && matches!(depth, 1 | 2 | 4))) {
        return Err("a PNG icon of a bit depth this decoder does not read");
    }
    let bpp_bits = channels * depth;
    let stride = (w as usize * bpp_bits).div_ceil(8);
    let step = bpp_bits.div_ceil(8); // the filter's byte distance
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).map_err(|_| "a PNG icon whose image data does not inflate")?;
    if raw.len() < (stride + 1) * ht as usize {
        return Err(bad);
    }
    let mut prev = alloc::vec![0u8; stride];
    let mut rgba = Vec::with_capacity((w * ht * 4) as usize);
    for y in 0..ht as usize {
        let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let mut cur = line[1..].to_vec();
        for i in 0..stride {
            let a = if i >= step { cur[i - step] } else { 0 };
            let (up, ul) = (prev[i], if i >= step { prev[i - step] } else { 0 });
            cur[i] = cur[i].wrapping_add(match line[0] {
                0 => 0,
                1 => a,
                2 => up,
                3 => ((a as u16 + up as u16) / 2) as u8,
                4 => paeth(a, up, ul),
                _ => return Err(bad),
            });
        }
        for x in 0..w as usize {
            let s = |k: usize| -> u8 {
                if depth == 16 { cur[(x * channels + k) * 2] } else { cur[x * channels + k] }
            };
            let px = match ctype {
                0 => [s(0), s(0), s(0), 0xFF],
                2 => [s(0), s(1), s(2), 0xFF],
                4 => [s(0), s(0), s(0), s(1)],
                6 => [s(0), s(1), s(2), s(3)],
                _ => {
                    let idx = if depth == 8 { cur[x] as usize } else {
                        let per = 8 / depth;
                        ((cur[x / per] >> (8 - depth * (x % per + 1))) & ((1 << depth) - 1) as u8) as usize
                    };
                    let c = plte.get(idx * 3..idx * 3 + 3).ok_or(bad)?;
                    [c[0], c[1], c[2], *trns.get(idx).unwrap_or(&0xFF)]
                }
            };
            rgba.extend_from_slice(&px);
        }
        prev = cur;
    }
    Ok(IconImage { w, h: ht, rgba, source: "png" })
}
