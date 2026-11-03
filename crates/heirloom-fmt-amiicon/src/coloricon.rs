//! OS3.5 ColorIcons: the `FORM ICON` IFF that follows a DiskObject's classic part (facts: AROS diskobj35io.c, "Decode35()
//! based on ModifyIcon").
//!
//! * `FACE`: width-1, height-1, flags, aspect, max palette bytes (u16).
//! * `IMAG` (one per state; the first is the normal image): transparent pen, pens-1, flags (bit 0 = has a transparent pen,
//!   bit 1 = has a palette; without one the first image's is used), image format (0 = raw bytes, 1 = run-length), palette
//!   format (likewise), depth, image bytes-1 (u16 BE), palette bytes-1 (u16 BE); then the image bytes, then the palette (RGB).
//!   The run-length form is a bit stream: an 8-bit control `n` (0..=127: the next `n+1` values literal; 129..=255: the next
//!   value repeated `257-n` times; 128: nothing), each value `depth` bits wide.
//! * `ARGB` (OS4): a 10-byte head (its u32 at offset 4 the zlib stream's size, less one in real files), then the stream:
//!   width*height ARGB.
//! * `png ` (AROS): a PNG of the normal image.
use crate::{png, IconImage};
use alloc::vec::Vec;
use heirloom_common::bytes::{be16, be32};

/// The run-length bit stream: `entries` values of `bits` bits.
pub fn unpack(src: &[u8], bits: u32, entries: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(entries);
    let (mut at, mut buf, mut nbits) = (0usize, 0u32, 0u32);
    let mut take = |n: u32| -> Option<u32> {
        while nbits < n {
            buf = (buf << 8) | *src.get(at)? as u32;
            at += 1;
            nbits += 8;
        }
        nbits -= n;
        Some((buf >> nbits) & ((1 << n) - 1))
    };
    while out.len() < entries {
        let c = take(8)?;
        match c {
            0..=127 => {
                for _ in 0..=c {
                    out.push(take(bits)? as u8);
                }
            }
            128 => {}
            _ => {
                let v = take(bits)? as u8;
                out.resize(out.len() + (257 - c) as usize, v);
            }
        }
    }
    out.truncate(entries);
    Some(out)
}

/// The `FORM ICON`'s chunks: `(id, data)`.
fn chunks(b: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    if b.get(0..4) != Some(b"FORM") || b.get(8..12) != Some(b"ICON") {
        return None;
    }
    let end = (be32(b, 4)? as usize + 8).min(b.len());
    let (mut at, mut out) = (12usize, Vec::new());
    while at + 8 <= end {
        let len = be32(b, at + 4)? as usize;
        let data = b.get(at + 8..at + 8 + len)?;
        out.push((b[at..at + 4].try_into().ok()?, data));
        at += 8 + len + (len & 1);
    }
    Some(out)
}

/// The ColorIcon after a DiskObject (`b` = the bytes after its classic part): Ok(None) when there is none.
pub fn decode(b: &[u8]) -> Result<Option<IconImage>, &'static str> {
    // A writer may leave padding before the FORM: look for it, at an even offset, in the first 64 bytes.
    let Some(start) = (0..b.len().min(64)).find(|&i| b[i..].starts_with(b"FORM")) else { return Ok(None) };
    let cs = chunks(&b[start..]).ok_or("a FORM ICON whose chunks run past its end")?;
    let face = cs.iter().find(|(id, _)| id == b"FACE").map(|(_, d)| *d).ok_or("a FORM ICON with no FACE")?;
    let (w, h) = (*face.first().ok_or("FACE")? as u32 + 1, *face.get(1).ok_or("FACE")? as u32 + 1);
    if let Some((_, d)) = cs.iter().find(|(id, _)| id == b"ARGB") {
        // The u32 at offset 4 is the stream's size minus one in the corpus's files (DOpus 5: 6511 for a 6512-byte stream), so the
        // stream is read to the chunk's end; the inflater stops at its own end marker.
        be32(d, 4).ok_or("an ARGB chunk too short")?;
        let argb = d.get(10..).and_then(|s| miniz_oxide::inflate::decompress_to_vec_zlib(s).ok()).ok_or("an ARGB chunk that does not inflate")?;
        if argb.len() == (w * h * 4) as usize {
            let rgba = argb.chunks_exact(4).flat_map(|p| [p[1], p[2], p[3], p[0]]).collect();
            return Ok(Some(IconImage { w, h, rgba, source: "coloricon-argb" }));
        }
    }
    if let Some((_, d)) = cs.iter().find(|(id, _)| id == b"png ") {
        if let Ok(img) = png::decode(d) {
            return Ok(Some(IconImage { source: "coloricon-png", ..img }));
        }
    }
    let imag = cs.iter().find(|(id, _)| id == b"IMAG").map(|(_, d)| *d).ok_or("a FORM ICON with no IMAG")?;
    let bad = "an IMAG chunk cut short or malformed";
    let hd = imag.get(0..10).ok_or(bad)?;
    let (transparent, pens, flags, img_fmt, pal_fmt, depth) = (hd[0], hd[1] as usize + 1, hd[2], hd[3], hd[4], hd[5] as u32);
    let img_len = be16(hd, 6).ok_or(bad)? as usize + 1;
    let pal_len = if flags & 2 != 0 { be16(hd, 8).ok_or(bad)? as usize + 1 } else { 0 };
    if flags & 2 == 0 {
        return Err("a normal IMAG with no palette");
    }
    if !(1..=8).contains(&depth) {
        return Err(bad);
    }
    let body = imag.get(10..10 + img_len + pal_len).ok_or(bad)?;
    let n = (w * h) as usize;
    let pixels = if img_fmt == 0 { body.get(..n).ok_or(bad)?.to_vec() } else { unpack(&body[..img_len], depth, n).ok_or(bad)? };
    let pal_src = &body[img_len..];
    let palette = if pal_fmt == 0 { pal_src.get(..pens * 3).ok_or(bad)?.to_vec() } else { unpack(pal_src, 8, pens * 3).ok_or(bad)? };
    let mut rgba = Vec::with_capacity(n * 4);
    for &p in &pixels {
        let c = palette.get(p as usize * 3..p as usize * 3 + 3).ok_or(bad)?;
        let a = if flags & 1 != 0 && p == transparent { 0 } else { 0xFF };
        rgba.extend_from_slice(&[c[0], c[1], c[2], a]);
    }
    Ok(Some(IconImage { w, h, rgba, source: "coloricon" }))
}
