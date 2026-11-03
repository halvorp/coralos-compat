//! The S60 MIF icon container (facts: EKA2L1 `src/emu/loader/src/mif.cpp` + `include/loader/mif.h`, GPL-3: facts only).
//!
//! All integers little-endian. Header: `B##4`, version (2, or 3 with one more u32), the index's offset, the entry count. The
//! index: `(offset, length)` per entry; entries come in pairs, an icon then its mask (for a vector icon the same data twice).
//! An entry: `C##4`, version, data offset (from the entry), data length, type (0 bitmap, 1 SVG-T, 2 NVG), depth, animated,
//! mask depth; the data may be gzip-wrapped. Only the first icon is read (the app icon is entry 0).
use crate::svgb;
use heirloom_icon::Picture;

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The gzip body (RFC 1952) inflated; None when it is not gzip or does not inflate.
fn gunzip(d: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    if d.len() < 18 || d[0] != 0x1F || d[1] != 0x8B || d[2] != 8 {
        return None;
    }
    let flags = d[3];
    let mut at = 10usize;
    if flags & 4 != 0 {
        at += 2 + u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]) as usize;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            at += d.get(at..)?.iter().position(|&c| c == 0)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    miniz_oxide::inflate::decompress_to_vec(d.get(at..d.len() - 8)?).ok()
}

/// The app icon (entry 0) of a MIF file.
pub fn icon(b: &[u8]) -> Result<Picture, &'static str> {
    if b.get(0..4) != Some(b"B##4") {
        return Err("not a MIF file (no B##4)");
    }
    let bad = "a MIF file cut short or malformed";
    let (index, count) = (le32(b, 8).ok_or(bad)? as usize, le32(b, 12).ok_or(bad)?);
    if count == 0 {
        return Err("a MIF file with no icon");
    }
    let off = le32(b, index).ok_or(bad)? as usize;
    let e = b.get(off..off + 32).ok_or(bad)?;
    if &e[0..4] != b"C##4" {
        return Err(bad);
    }
    let (doff, dlen, ty) = (le32(e, 8).ok_or(bad)? as usize, le32(e, 12).ok_or(bad)? as usize, le32(e, 16).ok_or(bad)?);
    let raw = b.get(off + doff..off + doff + dlen).ok_or(bad)?;
    let unzipped = gunzip(raw);
    let data = unzipped.as_deref().unwrap_or(raw);
    match ty {
        1 if data.starts_with(b"<") || data.starts_with(&[0xEF, 0xBB, 0xBF]) => {
            Ok(Picture::Svg { text: alloc::string::String::from_utf8_lossy(data).into_owned(), source: "mif-svg" })
        }
        1 => svgb::to_svg(data).map(|text| Picture::Svg { text, source: "mif-svgb" }).map_err(|_| "a MIF binary SVG-T this decoder cannot follow"),
        0 => Err("a MIF bitmap entry (this decoder reads the SVG-T entries; a bitmap entry is not read)"),
        2 => Err("a MIF NVG entry (OpenVG commands: not read)"),
        _ => Err(bad),
    }
}
