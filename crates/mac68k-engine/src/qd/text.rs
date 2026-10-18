//! Text: TextFont/TextFace/TextMode/TextSize, DrawChar/DrawString/DrawText, CharWidth/StringWidth/TextWidth, GetFontInfo.
//!
//! SUBSTITUTION: Chicago and Geneva are not bundled. EVERY font id and size draws in `retro-raster`'s topaz-8 (the font
//! the Amiga engine's ROM text uses, one renderer for both), and the metrics the program reads back
//! (`StringWidth`, `GetFontInfo`: ascent 7, descent 1, widMax 8, leading 0) are those of what is actually drawn, so the
//! program's own layout stays self-consistent. The font id/size/face the program set are stored in the port as written.
//! `txFace` bold/italic/underline map to the raster font styles; outline, shadow, condense and extend are not drawn.
//! `txMode` selects the stencil like a pen mode ([`super::draw::brush_for`]); the pen is the BASELINE origin and advances.
//! The baseline is a LINE between pixel rows (QuickDraw coordinates are grid points): the last row of a glyph without
//! descender is row `v - 1` (systemless, run as an oracle, and Inside Macintosh agree), unlike the Amiga's baseline ROW,
//! so the shared raster font is drawn at `v - 1` and `GetFontInfo` counts that row into the ascent.
use super::draw::brush_for;
use super::port::*;
use super::target::{self, clip_of, clip_region, current_port};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring_bytes, Arg};
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::font::{draw_text, FSF_BOLD, FSF_ITALIC, FSF_UNDERLINED};

fn style_of(face: u8) -> u8 {
    (if face & 1 != 0 { FSF_BOLD } else { 0 }) | (if face & 2 != 0 { FSF_ITALIC } else { 0 }) | (if face & 4 != 0 { FSF_UNDERLINED } else { 0 })
}

fn text_bytes<U: GuestUi>(m: &Mac<U>, buf: u32, first: u32, count: u32) -> Result<Vec<u8>, Refusal> {
    let e = Refusal::mem("text buffer");
    let (first, count) = (first as u16 as i16, count as u16 as i16);
    if first < 0 || count < 0 { return Err(Refusal(format!("text buffer: negative firstByte {first} or byteCount {count}"))); }
    m.mem.bytes(m.mem.span(buf.wrapping_add(first as u32), count as u32).map_err(&e)?).map_err(&e).map(<[u8]>::to_vec)
}

/// Draw `bytes` at the pen and advance it.
fn draw<U: GuestUi>(m: &mut Mac<U>, bytes: &[u8], name: &str) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    draw_on(m, port, bytes, name)
}

/// [`draw`] on a named port (the engine's own default control look draws into the owner window's port, not the current one).
pub(crate) fn draw_on<U: GuestUi>(m: &mut Mac<U>, port: u32, bytes: &[u8], name: &str) -> Result<Done, Refusal> {
    let pen = read_pen(&m.mem, port)?;
    let (face, tmode) = (m.mem.read_u8(port.wrapping_add(P_TX_FACE)).map_err(Refusal::mem("txFace"))?, m.mem.read_u16(port.wrapping_add(P_TX_MODE)).map_err(Refusal::mem("txMode"))?);
    let width = m.qd.font.text_length(bytes);
    if pen.vis >= 0 && !bytes.is_empty() {
        let (brush, style) = (brush_for(tmode), style_of(face));
        target::with(m, port, |m, t| {
            let clip = clip_region(m, t)?;
            let c = clip_of(&clip, t);
            let touched = draw_text(&mut t.bm, &m.qd.font, bytes, i32::from(pen.h) - t.local0.0, i32::from(pen.v) - t.local0.1 - 1, &brush, style, &c).1;
            // the glyph box the bitmap used: `width` wide, `y_size` tall, its baseline `baseline` rows down from the top
            let (x0, y0) = (i32::from(pen.h) - t.local0.0, i32::from(pen.v) - t.local0.1 - 1 - m.qd.font.baseline as i32);
            let rd = |off: u32| m.mem.read_u16(port.wrapping_add(off)).unwrap_or(0);
            let back = read_pat(&m.mem, port.wrapping_add(P_BK_PAT))?;
            let run = super::plane::Run { text: bytes, box_px: retro_raster::Rect::new(x0, y0, x0 + width as i32, y0 + m.qd.font.y_size as i32), font: rd(P_TX_FONT), face, size: rd(P_TX_SIZE), mode: tmode, back: &back };
            super::plane::record_text(&mut m.qd.plane, t, &run, &clip);
            Ok(touched)
        })?;
    }
    set_pen_loc(&mut m.mem, port, pen.v, pen.h.wrapping_add(width as i16))?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name} {:?} at ({},{}) width {width}", bytes.iter().map(|&b| char::from(b)).collect::<alloc::string::String>(), pen.v, pen.h) })
}

fn draw_char<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { draw(m, &[(c.args.get(0) & 0xFF) as u8], "DrawChar") }
fn draw_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let s = read_pstring_bytes(&m.mem, c.args.get(0))?;
    draw(m, &s, "DrawString")
}
fn draw_text_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let s = text_bytes(m, c.args.get(0), c.args.get(1), c.args.get(2))?;
    draw(m, &s, "DrawText")
}

fn width<U: GuestUi>(m: &Mac<U>, bytes: &[u8], name: &str) -> Result<Done, Refusal> {
    let w = m.qd.font.text_length(bytes);
    Ok(Done { result: w as u32 & 0xFFFF, log: format!("[quickdraw] {name} -> {w}") })
}
fn char_width<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { width(m, &[(c.args.get(0) & 0xFF) as u8], "CharWidth") }
fn string_width<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { width(m, &read_pstring_bytes(&m.mem, c.args.get(0))?, "StringWidth") }
fn text_width<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let s = text_bytes(m, c.args.get(0), c.args.get(1), c.args.get(2))?;
    width(m, &s, "TextWidth")
}

/// FontInfo: ascent, descent, widMax, leading (INTEGERs) of the font that is drawn.
fn get_font_info<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let f = &m.qd.font;
    let v = [f.baseline as i16 + 1, (f.y_size - f.baseline) as i16 - 1, f.x_size as i16, 0]; // the baseline row is the last row above the pen's baseline line
    for (i, x) in v.into_iter().enumerate() { m.mem.write_u16(c.args.get(0) + 2 * i as u32, x as u16).map_err(Refusal::mem("FontInfo"))?; }
    Ok(Done { result: 0, log: format!("[quickdraw] GetFontInfo -> ascent {} descent {} widMax {} leading 0", v[0], v[1], v[2]) })
}

fn set_field<U: GuestUi>(m: &mut Mac<U>, off: u32, v: u16, what: &str) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    m.mem.write_u16(p.wrapping_add(off), v).map_err(Refusal::mem("GrafPort"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] {what} {v} (drawn with the topaz-8 stand-in font)") })
}
fn text_font<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_field(m, P_TX_FONT, c.args.get(0) as u16, "TextFont") }
fn text_mode<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_field(m, P_TX_MODE, c.args.get(0) as u16, "TextMode") }
fn text_size<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_field(m, P_TX_SIZE, c.args.get(0) as u16, "TextSize") }
fn text_face<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = current_port(m)?;
    m.mem.write_u8(p.wrapping_add(P_TX_FACE), (c.args.get(0) & 0xFF) as u8).map_err(Refusal::mem("txFace"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] TextFace {:#04x}", c.args.get(0) & 0xFF) })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use super::reg;
    reg(m, 0xA887, "TextFont", &[Arg::W], None, text_font::<U>)?;
    reg(m, 0xA888, "TextFace", &[Arg::W], None, text_face::<U>)?;
    reg(m, 0xA889, "TextMode", &[Arg::W], None, text_mode::<U>)?;
    reg(m, 0xA88A, "TextSize", &[Arg::W], None, text_size::<U>)?;
    reg(m, 0xA883, "DrawChar", &[Arg::W], None, draw_char::<U>)?;
    reg(m, 0xA884, "DrawString", &[Arg::L], None, draw_string::<U>)?;
    reg(m, 0xA885, "DrawText", &[Arg::L, Arg::W, Arg::W], None, draw_text_trap::<U>)?;
    reg(m, 0xA88D, "CharWidth", &[Arg::W], Some(Arg::W), char_width::<U>)?;
    reg(m, 0xA88C, "StringWidth", &[Arg::L], Some(Arg::W), string_width::<U>)?;
    reg(m, 0xA886, "TextWidth", &[Arg::L, Arg::W, Arg::W], Some(Arg::W), text_width::<U>)?;
    reg(m, 0xA88B, "GetFontInfo", &[Arg::L], None, get_font_info::<U>)
}
