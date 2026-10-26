//! `FontMetrics` ($A835) and the Font Manager's global width table (Inside Macintosh: Text, "Font Manager", FMetricRec and
//! WidthTable; Inside Macintosh Vol IV p. 32).
//!
//! `FMetricRec` (20 bytes): ascent, descent, leading, widMax as FIXED, then `wTabHandle`, a handle to the global width table.
//! The metrics are those of the font [`super::text`] actually draws (the same record `GetFontInfo` returns, [`super::text::font_info`]),
//! so a program that sizes its lines from either sees one font. The table is the real layout, built in ONE place (here):
//!
//! | offset | field | value |
//! |--------|-------|-------|
//! | 0 | `tabData[0..256]` Fixed | the advance of each character, as `CharWidth` returns it ([`heirloom_raster::font::Font::advance`]) |
//! | 1024 | `tabFont` Handle | 0: no FONT resource backs the stand-in font |
//! | 1028 | `sExtra` Fixed | the port's `spExtra`, added to the space entry as the Font Manager does |
//! | 1032 | `style` LongInt | 0: `text_length` adds no style extra |
//! | 1036 | `fID`, `fSize`, `face`, `device` | the port's txFont, txSize, txFace; device 0 (the screen) |
//! | 1044 | `inNumer`, `inDenom` Point | 1/1 (v, h) |
//! | 1052 | `aFID`, `fHand` | txFont (the substitute is drawn for every id); 0 |
//! | 1058 | `usedFam` Boolean, `aFace` Byte | false (no fractional widths); the face bits drawn (bold, italic, underline) |
//! | 1060 | `vOutput`, `hOutput`, `vFactor`, `hFactor` | 256 (1.0 in 8.8): the stand-in font is never scaled |
//! | 1068 | `aSize`, `tabSize` | the drawn font's height; 1072 |
//!
//! The table is allocated once, on the first call, and rebuilt on each call for the current port's font (the real Font Manager
//! keeps one global table for the font last used). `WidthPtr` ($B10) and `WidthTabHandle` ($B2A) name it.
use super::port::*;
use super::target::current_port;
use super::text::font_info;
use crate::lowmem::{WIDTH_PTR, WIDTH_TAB_HANDLE};
use crate::machine::{Call, Done, Mac};
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::font::Font;

/// `sizeof(WidthTable)`, also stored in its `tabSize` field.
pub const WIDTH_TABLE_SIZE: u32 = 1072;
const FIXED_ONE: u32 = 1 << 16;
const SPACE: usize = b' ' as usize;

/// The bytes of the global width table for `font` drawn as (`fid`, `size`, `face`) with `space_extra` (Fixed) on the space.
pub fn width_table(font: &Font, fid: u16, size: u16, face: u8, space_extra: u32) -> Vec<u8> {
    let mut t = Vec::with_capacity(WIDTH_TABLE_SIZE as usize);
    let (w, l) = (|t: &mut Vec<u8>, v: u16| t.extend_from_slice(&v.to_be_bytes()), |t: &mut Vec<u8>, v: u32| t.extend_from_slice(&v.to_be_bytes()));
    for c in 0..=255u8 {
        let fixed = (font.advance(c) as u32) << 16;
        l(&mut t, if c as usize == SPACE { fixed.wrapping_add(space_extra) } else { fixed });
    }
    l(&mut t, 0); // tabFont
    l(&mut t, space_extra);
    l(&mut t, 0); // style
    for v in [fid, size, u16::from(face), 0] { w(&mut t, v); } // fID, fSize, face, device
    for v in [1, 1, 1, 1] { w(&mut t, v); } // inNumer (v, h), inDenom (v, h)
    w(&mut t, fid); // aFID
    l(&mut t, 0); // fHand
    t.push(0); // usedFam
    t.push(face & 7); // aFace: the soft styles drawn (bold, italic, underline)
    for _ in 0..4 { w(&mut t, 256); } // vOutput, hOutput, vFactor, hFactor
    w(&mut t, font.y_size); // aSize
    w(&mut t, WIDTH_TABLE_SIZE as u16);
    debug_assert_eq!(t.len(), WIDTH_TABLE_SIZE as usize);
    t
}

/// `FontMetrics(VAR theMetrics: FMetricRec)`.
pub(super) fn font_metrics<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rec = c.args.get(0);
    let port = current_port(m)?;
    let rd = |m: &Mac<U>, off: u32| m.mem.read_u16(port.wrapping_add(off)).map_err(Refusal::mem("GrafPort"));
    let (fid, size) = (rd(m, P_TX_FONT)?, rd(m, P_TX_SIZE)?);
    let face = m.mem.read_u8(port.wrapping_add(P_TX_FACE)).map_err(Refusal::mem("txFace"))?;
    let sp_extra = m.mem.read_u32(port.wrapping_add(P_SP_EXTRA)).map_err(Refusal::mem("spExtra"))?;
    let table = width_table(&m.qd.font, fid, size, face, sp_extra);
    if m.qd.width_table == 0 || m.mm.block(m.qd.width_table).is_none() {
        m.qd.width_table = m.mm.new_handle(&mut m.mem, &mut m.heap, WIDTH_TABLE_SIZE, true).map_err(|e| Refusal(format!("FontMetrics: no room for the width table (memory error {e})")))?;
    }
    let (h, ptr) = (m.qd.width_table, m.mm.block(m.qd.width_table).map_or(0, |b| b.ptr));
    let e = Refusal::mem("FontMetrics");
    m.mem.write_bytes(ptr, &table).map_err(&e)?;
    m.mem.write_u32(WIDTH_PTR, ptr).map_err(&e)?;
    m.mem.write_u32(WIDTH_TAB_HANDLE, h).map_err(&e)?;
    let [ascent, descent, wid_max, leading] = font_info(&m.qd.font);
    for (i, v) in [ascent, descent, leading, wid_max].into_iter().enumerate() {
        m.mem.write_u32(rec.wrapping_add(4 * i as u32), (v as u32).wrapping_mul(FIXED_ONE)).map_err(&e)?;
    }
    m.mem.write_u32(rec.wrapping_add(16), h).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[quickdraw] FontMetrics -> ascent {ascent} descent {descent} leading {leading} widMax {wid_max}, width table {h:#x}") })
}
