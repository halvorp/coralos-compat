//! The EPOC multi-bitmap file (`.mbm`): a colour bitmap and its mask (facts: EKA2L1 `src/emu/loader/src/mbm.cpp`,
//! `include/loader/mbm.h`, `src/emu/common/src/runlen.cpp`; GPL-3: facts only - checked against the SDK's real .mbm files by
//! tests/real_icons.rs).
//!
//! All integers little-endian. Header: UIDs `0x10000037` (direct file store) and `0x10000042` (multi-bitmap), a third UID, a
//! checksum, then the trailer's offset; the trailer is a count and one header offset per bitmap. A bitmap header (40 bytes):
//! total size (header included), header size, width, height, width and height in twips, bits per pixel, colour flag (0 grey,
//! 1 colour, 2 colour with alpha), palette entries, compression (0 none, 1..=4 run-length over 8, 12, 16 and 24-bit values).
//! Each scan line is padded to 4 bytes; a 12-bit pixel takes 16 bits. Pixels narrower than a byte are packed least
//! significant bits first. RLE: a signed control byte, `n >= 0` = the next value `n+1` times, `n < 0` = `-n` literal values;
//! the 12-bit form packs the repeat count in each u16's top nibble instead.
//!
//! An icon file holds (icon, mask) pairs at several sizes: the largest pair is taken, its mask's grey level is the alpha. The
//! 256-colour palette (8-bit colour) is not built in: such a bitmap is refused by name.
use heirloom_icon::IconImage;
use alloc::vec::Vec;

/// Bytes per scan line: padded to 4 bytes, a 12-bit pixel stored in 16 bits.
fn stride(w: u32, bpp: u32) -> usize {
    ((w * if bpp == 12 { 16 } else { bpp }).div_ceil(32) * 4) as usize
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// One bitmap: geometry, depth, colour flag and its decompressed scan lines.
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub bpp: u32,
    pub color: u32,
    pub data: Vec<u8>,
}

fn unrle(src: &[u8], unit: usize, out_len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(out_len);
    let mut at = 0;
    while out.len() < out_len && at < src.len() {
        let n = src[at] as i8;
        at += 1;
        if n >= 0 {
            let v = src.get(at..at + unit)?;
            at += unit;
            for _ in 0..=n {
                out.extend_from_slice(v);
            }
        } else {
            let k = (-(n as i32)) as usize * unit;
            out.extend_from_slice(src.get(at..at + k)?);
            at += k;
        }
    }
    out.resize(out_len, 0);
    Some(out)
}

fn unrle12(src: &[u8], out_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(out_len);
    for c in src.chunks_exact(2) {
        let v = u16::from_le_bytes([c[0], c[1]]);
        for _ in 0..=(v >> 12) {
            out.extend_from_slice(&(v & 0x0FFF).to_le_bytes());
        }
        if out.len() >= out_len {
            break;
        }
    }
    out.resize(out_len, 0);
    out
}

/// Every bitmap of an MBM file.
pub fn bitmaps(b: &[u8]) -> Result<Vec<Bitmap>, &'static str> {
    if le32(b, 0) != Some(0x1000_0037) || le32(b, 4) != Some(0x1000_0042) {
        return Err("not a multi-bitmap file (UIDs 0x10000037 / 0x10000042)");
    }
    let bad = "a multi-bitmap file cut short or malformed";
    let trailer = le32(b, 16).ok_or(bad)? as usize;
    let count = le32(b, trailer).ok_or(bad)? as usize;
    let mut out = Vec::new();
    for i in 0..count.min(256) {
        let at = le32(b, trailer + 4 + 4 * i).ok_or(bad)? as usize;
        let h = b.get(at..at + 40).ok_or(bad)?;
        let f = |o: usize| le32(h, o).unwrap_or(0);
        let (total, hlen, w, ht, bpp, color, comp) = (f(0) as usize, f(4) as usize, f(8), f(12), f(24), f(28), f(36));
        if w == 0 || ht == 0 || w > 8192 || ht > 8192 || hlen < 40 || total < hlen {
            return Err(bad);
        }
        let need = stride(w, bpp) * ht as usize;
        let src = b.get(at + hlen..at + total).ok_or(bad)?;
        let data = match comp {
            0 => src.get(..need).ok_or(bad)?.to_vec(),
            1 => unrle(src, 1, need).ok_or(bad)?,
            2 => unrle12(src, need),
            3 => unrle(src, 2, need).ok_or(bad)?,
            4 => unrle(src, 3, need).ok_or(bad)?,
            _ => return Err("a bitmap compression this decoder does not read"),
        };
        out.push(Bitmap { w, h: ht, bpp, color, data });
    }
    Ok(out)
}

impl Bitmap {
    /// The pixel at (x, y) as RGBA (a grey bitmap as grey, opaque).
    pub fn px(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let row = &self.data[y as usize * stride(self.w, self.bpp)..];
        let x = x as usize;
        Some(match self.bpp {
            1 | 2 | 4 => {
                let per = 8 / self.bpp as usize;
                let v = (row[x / per] >> ((x % per) * self.bpp as usize)) & ((1u16 << self.bpp) - 1) as u8;
                let g = (v as u32 * 255 / ((1 << self.bpp) - 1)) as u8;
                [g, g, g, 0xFF]
            }
            8 if self.color == 0 => [row[x], row[x], row[x], 0xFF],
            12 => {
                let v = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]);
                [((v >> 8) & 0xF) as u8 * 17, ((v >> 4) & 0xF) as u8 * 17, (v & 0xF) as u8 * 17, 0xFF]
            }
            16 => {
                let v = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]);
                let (r, g, b) = ((v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F);
                [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8, 0xFF]
            }
            24 => [row[3 * x + 2], row[3 * x + 1], row[3 * x], 0xFF],
            32 => [row[4 * x + 2], row[4 * x + 1], row[4 * x], if self.color == 2 { row[4 * x + 3] } else { 0xFF }],
            _ => return None,
        })
    }
}

/// The icon of an MBM file: the largest (colour, mask) pair, else the largest bitmap alone.
pub fn icon(b: &[u8]) -> Result<IconImage, &'static str> {
    let bms = bitmaps(b)?;
    let mut best: Option<(usize, Option<usize>)> = None;
    let area = |i: usize| bms[i].w * bms[i].h;
    for i in 0..bms.len() {
        let mask = bms.get(i + 1).filter(|m| m.w == bms[i].w && m.h == bms[i].h && m.color == 0 && m.bpp <= 8).map(|_| i + 1);
        if best.is_none_or(|(j, jm)| (mask.is_some(), area(i)) > (jm.is_some(), area(j))) {
            best = Some((i, mask));
        }
    }
    let (i, mask) = best.ok_or("a multi-bitmap file with no bitmap")?;
    let c = &bms[i];
    let mut rgba = Vec::with_capacity((c.w * c.h * 4) as usize);
    for y in 0..c.h {
        for x in 0..c.w {
            let mut p = c.px(x, y).ok_or("an 8-bit colour bitmap (the 256-colour palette is not built in)")?;
            if let Some(m) = mask {
                p[3] = bms[m].px(x, y).map_or(0xFF, |q| q[0]);
            }
            rgba.extend_from_slice(&p);
        }
    }
    Ok(IconImage { w: c.w, h: c.h, rgba, source: if mask.is_some() { "mbm+mask" } else { "mbm" } })
}
