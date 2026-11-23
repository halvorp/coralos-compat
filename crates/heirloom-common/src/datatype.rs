//! Shared native datatype bridge trait + pure-Rust fallbacks (`BMP`, Amiga `IFF-ILBM`/`PBM`).
//!
//! Engines ALWAYS invoke the attached [`NativeDatatype`] host first (which on CoralOS delegates
//! straight to the out-of-process native amd64 `datatype.library` / `jpeg.datatype` / `gif.datatype`
//! / `webp.datatype` / `svg.datatype` workers via `OsVTable::attach_datatype`), and only fall back
//! to guest-native container decoders (`IFF-ILBM`, `BMP`) when the host returns `None`.
use alloc::vec;
use alloc::vec::Vec;

pub const DT_UNKNOWN: u32 = 0;
pub const DT_PNG: u32 = 1;
pub const DT_WEBP: u32 = 2;
pub const DT_GIF: u32 = 3;
pub const DT_JPEG: u32 = 4;
pub const DT_SVG: u32 = 6;
pub const DT_BMP: u32 = 7;
pub const DT_PDF: u32 = 17;
pub const DT_IFF_ILBM: u32 = 0x494C_424D;

/// Decoded image in `0xAARRGGBB` (ARGB32) row-major order, `width * height` pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// Host-supplied native datatype decoder (backed by CoralOS `datatype.library` on target,
/// or a host decoder in Linux CLI runners and tests).
pub trait NativeDatatype: Send {
    fn identify(&self, data: &[u8]) -> u32 {
        identify_magic(data)
    }
    fn decode_image(&self, data: &[u8], fit_w: u32, fit_h: u32) -> Option<DecodedImage>;
}

/// Identify common image/document formats by magic header without allocating.
pub fn identify_magic(data: &[u8]) -> u32 {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        return DT_PNG;
    }
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return DT_JPEG;
    }
    if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        return DT_GIF;
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return DT_WEBP;
    }
    if data.len() >= 12 && &data[..4] == b"FORM" && (&data[8..12] == b"ILBM" || &data[8..12] == b"PBM ") {
        return DT_IFF_ILBM;
    }
    if data.len() >= 2 && &data[..2] == b"BM" {
        return DT_BMP;
    }
    if data.len() >= 5 && &data[..5] == b"%PDF-" {
        return DT_PDF;
    }
    let trimmed = skip_ws_and_bom(data);
    if trimmed.starts_with(b"<svg") || (trimmed.starts_with(b"<?xml") && trimmed[..trimmed.len().min(512)].windows(4).any(|w| w == b"<svg")) {
        return DT_SVG;
    }
    DT_UNKNOWN
}

fn skip_ws_and_bom(mut s: &[u8]) -> &[u8] {
    if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
        s = &s[3..];
    }
    while let Some(&b) = s.first() {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            s = &s[1..];
        } else {
            break;
        }
    }
    s
}

/// Decode `data` through the native host datatype system first, falling back to built-in
/// Amiga `IFF-ILBM`/`PBM` and `BMP` decoders for legacy guest-native containers.
pub fn decode_with_fallback(host: Option<&dyn NativeDatatype>, data: &[u8], fit_w: u32, fit_h: u32) -> Option<DecodedImage> {
    if let Some(h) = host {
        if let Some(img) = h.decode_image(data, fit_w, fit_h) {
            return Some(img);
        }
    }
    match identify_magic(data) {
        DT_IFF_ILBM => decode_ilbm(data),
        DT_BMP => decode_bmp(data),
        _ => None,
    }
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    crate::bytes::be16(b, at)
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    crate::bytes::be32(b, at)
}

/// Pure-Rust uncompressed Windows `BMP` decoder (1/4/8/24/32 bpp, top-down or bottom-up).
pub fn decode_bmp(data: &[u8]) -> Option<DecodedImage> {
    if data.len() < 54 || &data[..2] != b"BM" {
        return None;
    }
    let pix_off = le32(data, 10)? as usize;
    let dib_size = le32(data, 14)? as usize;
    if dib_size < 40 || pix_off > data.len() {
        return None;
    }
    let w_raw = le32(data, 18)? as i32;
    let h_raw = le32(data, 22)? as i32;
    if w_raw <= 0 || h_raw == 0 || w_raw > 8192 || h_raw.unsigned_abs() > 8192 {
        return None;
    }
    let w = w_raw as usize;
    let h = h_raw.unsigned_abs() as usize;
    let top_down = h_raw < 0;
    let bpp = le16(data, 28)? as usize;
    let compression = le32(data, 30)?;
    if compression != 0 && !(compression == 3 && bpp == 32) {
        return None;
    }
    let pal_off = 14 + dib_size;
    let mut pal = [0xFF00_0000u32; 256];
    if bpp <= 8 {
        let colors = (le32(data, 46)? as usize).min(256);
        let count = if colors == 0 { 1usize << bpp } else { colors };
        for i in 0..count.min(256) {
            let p = pal_off + i * 4;
            if p + 3 <= data.len() {
                pal[i] = 0xFF00_0000 | ((data[p + 2] as u32) << 16) | ((data[p + 1] as u32) << 8) | (data[p] as u32);
            }
        }
    }
    let stride = ((w * bpp + 31) / 32) * 4;
    let mut pixels = vec![0xFF00_0000u32; w.checked_mul(h)?];
    for row in 0..h {
        let src_y = if top_down { row } else { h - 1 - row };
        let row_off = pix_off.checked_add(src_y.checked_mul(stride)?)?;
        let row_bytes = data.get(row_off..row_off.checked_add(stride)?)?;
        let dst = &mut pixels[row * w..(row + 1) * w];
        match bpp {
            32 => {
                for x in 0..w {
                    let o = x * 4;
                    let a = if row_bytes[o + 3] == 0 { 0xFF } else { row_bytes[o + 3] };
                    dst[x] = ((a as u32) << 24) | ((row_bytes[o + 2] as u32) << 16) | ((row_bytes[o + 1] as u32) << 8) | (row_bytes[o] as u32);
                }
            }
            24 => {
                for x in 0..w {
                    let o = x * 3;
                    dst[x] = 0xFF00_0000 | ((row_bytes[o + 2] as u32) << 16) | ((row_bytes[o + 1] as u32) << 8) | (row_bytes[o] as u32);
                }
            }
            8 => {
                for x in 0..w {
                    dst[x] = pal[row_bytes[x] as usize];
                }
            }
            4 => {
                for x in 0..w {
                    let b = row_bytes[x >> 1];
                    let idx = if x & 1 == 0 { b >> 4 } else { b & 0x0F };
                    dst[x] = pal[idx as usize];
                }
            }
            1 => {
                for x in 0..w {
                    let bit = (row_bytes[x >> 3] >> (7 - (x & 7))) & 1;
                    dst[x] = pal[bit as usize];
                }
            }
            _ => return None,
        }
    }
    Some(DecodedImage { width: w as u32, height: h as u32, pixels })
}

/// Pure-Rust Amiga IFF `ILBM` / `PBM ` decoder (1..8 bitplanes, 24-bit ILBM, EHB, HAM6/HAM8, ByteRun1 RLE).
pub fn decode_ilbm(data: &[u8]) -> Option<DecodedImage> {
    if data.len() < 12 || &data[..4] != b"FORM" {
        return None;
    }
    let is_pbm = match &data[8..12] {
        b"ILBM" => false,
        b"PBM " => true,
        _ => return None,
    };
    let form_end = (8usize + be32(data, 4)? as usize).min(data.len());
    let mut pos = 12usize;
    let mut bmhd: Option<(usize, usize, u8, u8, u8)> = None; // (w, h, planes, masking, compression)
    let mut cmap: Vec<u32> = Vec::new();
    let mut camg: u32 = 0;
    let mut body: Option<&[u8]> = None;
    while pos + 8 <= form_end {
        let id = &data[pos..pos + 4];
        let len = be32(data, pos + 4)? as usize;
        let payload_start = pos + 8;
        let payload_end = payload_start.checked_add(len)?;
        if payload_end > form_end {
            break;
        }
        let chunk = &data[payload_start..payload_end];
        match id {
            b"BMHD" if chunk.len() >= 20 => {
                let w = be16(chunk, 0)? as usize;
                let h = be16(chunk, 2)? as usize;
                let planes = chunk[8];
                let masking = chunk[9];
                let compression = chunk[10];
                bmhd = Some((w, h, planes, masking, compression));
            }
            b"CMAP" => {
                cmap.clear();
                for rgb in chunk.chunks_exact(3) {
                    cmap.push(0xFF00_0000 | ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | (rgb[2] as u32));
                }
            }
            b"CAMG" if chunk.len() >= 4 => {
                camg = be32(chunk, 0)?;
            }
            b"BODY" => {
                body = Some(chunk);
            }
            _ => {}
        }
        pos = payload_end + (len & 1);
    }
    let (w, h, planes, masking, compression) = bmhd?;
    let body = body?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 || planes == 0 || (planes > 8 && planes != 24) {
        return None;
    }
    let is_ham = !is_pbm && (camg & 0x0800 != 0) && (planes == 6 || planes == 8);
    let is_ehb = !is_pbm && (camg & 0x0080 != 0) && planes == 6;
    if cmap.is_empty() {
        for i in 0..256u32 {
            cmap.push(0xFF00_0000 | (i << 16) | (i << 8) | i);
        }
    }
    if is_ehb && cmap.len() <= 32 {
        let base = cmap.len();
        cmap.resize(64, 0xFF00_0000);
        for i in 0..base.min(32) {
            let c = cmap[i];
            let r = ((c >> 16) & 0xFF) >> 1;
            let g = ((c >> 8) & 0xFF) >> 1;
            let b = (c & 0xFF) >> 1;
            cmap[32 + i] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }

    let row_bytes = if is_pbm { (w + 1) & !1 } else { ((w + 15) / 16) * 2 };
    let total_planes = if is_pbm { 1usize } else { planes as usize + usize::from(masking == 1) };
    let expected_row = row_bytes.checked_mul(total_planes)?;
    let raw = if compression == 1 {
        unpack_byterun1(body, expected_row.checked_mul(h)?)?
    } else if compression == 0 {
        body.to_vec()
    } else {
        return None;
    };

    let mut pixels = vec![0xFF00_0000u32; w.checked_mul(h)?];
    for y in 0..h {
        let row_off = y * expected_row;
        if row_off + expected_row > raw.len() {
            break;
        }
        let dst = &mut pixels[y * w..(y + 1) * w];
        if is_pbm {
            for x in 0..w {
                let idx = raw[row_off + x] as usize;
                dst[x] = cmap.get(idx).copied().unwrap_or(0xFF00_0000);
            }
        } else if planes == 24 {
            for x in 0..w {
                let byte_i = x >> 3;
                let bit = 7 - (x & 7);
                let mut r = 0u32;
                let mut g = 0u32;
                let mut b = 0u32;
                for p in 0..8 {
                    r |= (((raw[row_off + p * row_bytes + byte_i] >> bit) & 1) as u32) << p;
                    g |= (((raw[row_off + (8 + p) * row_bytes + byte_i] >> bit) & 1) as u32) << p;
                    b |= (((raw[row_off + (16 + p) * row_bytes + byte_i] >> bit) & 1) as u32) << p;
                }
                dst[x] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
            }
        } else {
            let mut prev = cmap.first().copied().unwrap_or(0xFF00_0000);
            for x in 0..w {
                let byte_i = x >> 3;
                let bit = 7 - (x & 7);
                let mut val = 0u32;
                for p in 0..planes as usize {
                    let b = raw[row_off + p * row_bytes + byte_i];
                    val |= (((b >> bit) & 1) as u32) << p;
                }
                if is_ham {
                    let shift = if planes == 6 { 4 } else { 6 };
                    let ctrl = val >> shift;
                    let data_bits = val & ((1 << shift) - 1);
                    let comp = if planes == 6 { (data_bits << 4) | data_bits } else { (data_bits << 2) | (data_bits >> 4) };
                    prev = match ctrl {
                        0 => cmap.get(data_bits as usize).copied().unwrap_or(0xFF00_0000),
                        1 => (prev & 0xFFFF_FF00) | comp,         // modify Blue
                        2 => (prev & 0xFF00_FFFF) | (comp << 16), // modify Red
                        _ => (prev & 0xFFFF_00FF) | (comp << 8),  // modify Green
                    };
                    dst[x] = prev;
                } else {
                    dst[x] = cmap.get(val as usize).copied().unwrap_or(0xFF00_0000);
                }
            }
        }
    }
    Some(DecodedImage { width: w as u32, height: h as u32, pixels })
}

fn unpack_byterun1(src: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(max_out.min(16 * 1024 * 1024));
    let mut i = 0usize;
    while i < src.len() && out.len() < max_out {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let count = (n as usize) + 1;
            let end = i.checked_add(count)?.min(src.len());
            let take = (end - i).min(max_out - out.len());
            out.extend_from_slice(&src[i..i + take]);
            i = end;
        } else if n != -128 {
            let count = ((-n as i32) as usize) + 1;
            let b = *src.get(i)?;
            i += 1;
            let take = count.min(max_out - out.len());
            out.resize(out.len() + take, b);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_datatype_runs_before_builtin_fallback() {
        struct MockHost;
        impl NativeDatatype for MockHost {
            fn decode_image(&self, data: &[u8], _fw: u32, _fh: u32) -> Option<DecodedImage> {
                if identify_magic(data) == DT_PNG {
                    Some(DecodedImage { width: 1, height: 1, pixels: vec![0xFF12_3456] })
                } else {
                    None
                }
            }
        }
        let img = decode_with_fallback(Some(&MockHost), b"\x89PNG\r\n\x1a\n", 0, 0).unwrap();
        assert_eq!(img.pixels, vec![0xFF12_3456]);
    }

    #[test]
    fn decodes_24bit_bmp_and_iff_ilbm() {
        // Minimal 2x1 24-bit bottom-up BMP (red, blue)
        let mut bmp = vec![0u8; 54 + 8];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&2u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&1u32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp[54..60].copy_from_slice(&[0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00]); // BGR: red, blue
        let img = decode_bmp(&bmp).expect("bmp");
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.pixels, vec![0xFFFF_0000, 0xFF00_00FF]);

        // Minimal 2x1 1-plane uncompressed IFF ILBM
        let mut ilbm = Vec::new();
        ilbm.extend_from_slice(b"FORM");
        let body_len = 4 + (8 + 20) + (8 + 6) + (8 + 2);
        ilbm.extend_from_slice(&(body_len as u32).to_be_bytes());
        ilbm.extend_from_slice(b"ILBM");
        ilbm.extend_from_slice(b"BMHD");
        ilbm.extend_from_slice(&20u32.to_be_bytes());
        let mut bmhd = [0u8; 20];
        bmhd[0..2].copy_from_slice(&2u16.to_be_bytes());
        bmhd[2..4].copy_from_slice(&1u16.to_be_bytes());
        bmhd[8] = 1; // 1 plane
        ilbm.extend_from_slice(&bmhd);
        ilbm.extend_from_slice(b"CMAP");
        ilbm.extend_from_slice(&6u32.to_be_bytes());
        ilbm.extend_from_slice(&[0x10, 0x20, 0x30, 0xAA, 0xBB, 0xCC]);
        ilbm.extend_from_slice(b"BODY");
        ilbm.extend_from_slice(&2u32.to_be_bytes());
        ilbm.extend_from_slice(&[0x40, 0x00]); // pixel 0 = 0, pixel 1 = 1
        assert_eq!(identify_magic(&ilbm), DT_IFF_ILBM);
        let img = decode_ilbm(&ilbm).expect("ilbm");
        assert_eq!(img.pixels, vec![0xFF10_2030, 0xFFAA_BBCC]);
    }
}
