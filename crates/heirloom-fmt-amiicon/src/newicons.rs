//! NewIcons: an icon picture encoded in a DiskObject's tool types (facts: AROS diskobjNIio.c, "based on ModifyIcon").
//!
//! After the tool type `*** DON'T EDIT THE FOLLOWING LINES!! ***`, the normal image is the lines starting `IM1=` (the selected
//! one `IM2=`, ignored). The first line's 5 characters after `IM1=` are: transparency (`B` = pen 0 transparent, `C` = opaque),
//! width + 0x21, height + 0x21, and the pen count as two 6-bit digits (each + 0x21). The rest of that line and the lines after
//! it are a 7-bit stream: a character below 0xA0 is `c - 0x20`, one below 0xD1 is `c - 0x51`, and one from 0xD1 up is
//! `c - 0xD0` zero groups. Each line restarts the bit count (left-over bits at a line's end are dropped). The stream holds the
//! palette (3 bytes a pen, 8 bits each) - which ends its line - then the pixels, `ceil(log2(pens))` bits each, from the next line.
use crate::IconImage;
use alloc::string::String;
use alloc::vec::Vec;

const MARKER: &str = "*** DON'T EDIT THE FOLLOWING LINES!! ***";

/// Decode `entries` values of `bits` bits from `lines` (each without its `IM1=` prefix), starting at `lines[*line]` offset
/// `skip`; on return `*line` is the line after the last one read.
fn stream(lines: &[&[u8]], line: &mut usize, skip: usize, bits: u32, entries: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(entries);
    let mask = (1u32 << bits) - 1;
    let (mut src, mut at) = (*lines.get(*line)?, skip);
    *line += 1;
    let (mut buf, mut nbits, mut zeros) = (0u32, 0u32, 0u32);
    while out.len() < entries {
        let v = if zeros > 0 {
            zeros -= 1;
            0
        } else {
            if at >= src.len() {
                src = lines.get(*line)?;
                *line += 1;
                at = 0;
                nbits = 0;
            }
            let c = src[at];
            at += 1;
            match c {
                0 => return None,
                c if c < 0xA0 => c.checked_sub(0x20)? as u32,
                c if c < 0xD1 => (c - 0x51) as u32,
                c => {
                    zeros = (c - 0xD1) as u32;
                    0
                }
            }
        };
        buf = (buf << 7 | v) & 0x3FFF;
        nbits += 7;
        while nbits >= bits && out.len() < entries {
            out.push(((buf >> (nbits - bits)) & mask) as u8);
            nbits -= bits;
        }
    }
    Some(out)
}

/// The NewIcon normal image in `tool_types`: Ok(None) when there is none, Err when it is there but cannot be read.
pub fn decode(tool_types: &[String]) -> Result<Option<IconImage>, &'static str> {
    let Some(start) = tool_types.iter().position(|t| t == MARKER) else { return Ok(None) };
    let latin1: Vec<Vec<u8>> = tool_types[start + 1..].iter().map(|t| t.chars().map(|c| c as u32 as u8).collect()).collect();
    let lines: Vec<&[u8]> = latin1.iter().filter_map(|l| l.strip_prefix(b"IM1=")).collect();
    let Some(head) = lines.first() else { return Ok(None) };
    let bad = "a NewIcon whose IM1= data is cut short or malformed";
    if head.len() < 5 {
        return Err(bad);
    }
    let d = |c: u8| (c as u32).checked_sub(0x21).ok_or(bad);
    let (w, h) = (d(head[1])?, d(head[2])?);
    let pens = (d(head[3])? << 6 | d(head[4])?) as usize;
    if w == 0 || h == 0 || pens == 0 || pens > 256 {
        return Err(bad);
    }
    let transparent = head[0] == b'B';
    let mut line = 0;
    let palette = stream(&lines, &mut line, 5, 8, pens * 3).ok_or(bad)?;
    let bits = (1..=8).find(|b| 1usize << b >= pens).unwrap_or(8);
    let pixels = if line < lines.len() { stream(&lines, &mut line, 0, bits, (w * h) as usize).ok_or(bad)? } else { return Err(bad) };
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for &p in &pixels {
        let i = p as usize * 3;
        let c = palette.get(i..i + 3).ok_or(bad)?;
        rgba.extend_from_slice(&[c[0], c[1], c[2], if transparent && p == 0 { 0 } else { 0xFF }]);
    }
    Ok(Some(IconImage { w, h, rgba, source: "newicon" }))
}
