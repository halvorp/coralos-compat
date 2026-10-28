//! Text(), TextLength(), TextExtent(), TextFit() and the soft-style calls, over the RastPort's guest `struct TextFont`.
use super::layout::*;
use super::{fonts, rastport, target};
use crate::{machine::Amiga, Refusal};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::{font::draw_text, Rect};

const FS_ALGO: u8 = 0x07; // FSF_UNDERLINED | FSF_BOLD | FSF_ITALIC

fn read_str<U: GuestUi>(a: &Amiga<U>, p: u32, n: usize, dir: i32) -> Result<Vec<u8>, Refusal> {
    (0..n as i64).map(|i| a.mem.read_u8((p as i64 + i * dir as i64) as u32).map_err(Refusal::mem("text string"))).collect()
}

/// Text(a1 = rp, a0 = string, d0 = count): draw at the cursor, the cursor moves by the string's advance.
pub fn text<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, s, n) = (cpu.areg(1), cpu.areg(0), cpu.dreg(0) as u16 as usize);
    let bytes = read_str(a, s, n, 1)?;
    let st = rastport::read(a, rp)?;
    let (_, font) = fonts::rp_font(a, rp)?;
    let (brush, algo, (x, y)) = (st.brush(), st.algo, st.cp);
    let adv = font.text_length(&bytes);
    let prim = super::record::text(&st, &font, &bytes, (x, y));
    target::paint_as(a, rp, "Text", prim, |bm, clip, (ox, oy)| draw_text(bm, &font, &bytes, x - ox, y - oy, &brush, algo, clip).1)?;
    rastport::set_cp(a, rp, x + adv, y)?;
    Ok(format!("[graphics] Text {:?} at {x},{y} (+{adv})", String::from_utf8_lossy(&bytes[..bytes.len().min(48)])))
}

/// TextLength(a1 = rp, a0 = string, d0 = count) -> the advance in pixels.
pub fn text_length<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, s, n) = (cpu.areg(1), cpu.areg(0), cpu.dreg(0) as u16 as usize);
    let bytes = read_str(a, s, n, 1)?;
    let (_, font) = fonts::rp_font(a, rp)?;
    let w = font.text_length(&bytes);
    cpu.set_d0(w as u32);
    Ok(format!("[graphics] TextLength {:?} ({n} chars) -> {w}", String::from_utf8_lossy(&bytes[..bytes.len().min(48)])))
}

/// Fill a guest TextExtent: width, height and the inclusive extent rectangle.
pub fn write_extent<U: GuestUi>(a: &mut Amiga<U>, te: u32, width: i32, r: Rect, height: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("TextExtent");
    a.mem.write_u16(te, width as u16).map_err(&m)?;
    a.mem.write_u16(te + 2, height as u16).map_err(&m)?;
    for (i, v) in [r.x0, r.y0, r.x1 - 1, r.y1 - 1].into_iter().enumerate() { a.mem.write_u16(te + 4 + 2 * i as u32, v as u16).map_err(&m)?; }
    Ok(())
}

/// TextExtent(a1 = rp, a0 = string, d0 = count, a2 = TextExtent).
pub fn text_extent<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, s, n, te) = (cpu.areg(1), cpu.areg(0), cpu.dreg(0) as u16 as usize, cpu.areg(2));
    let bytes = read_str(a, s, n, 1)?;
    let (_, font) = fonts::rp_font(a, rp)?;
    let ex = font.extent(&bytes, rastport::read(a, rp)?.algo);
    write_extent(a, te, ex.width, ex.rect, font.y_size as u32)?;
    cpu.set_d0(ex.width as u32);
    Ok(format!("[graphics] TextExtent {n} chars -> width {}", ex.width))
}

/// TextFit(a1 = rp, a0 = string, d0 = strLen, a2 = extent out, a3 = constraining extent, d1 = direction,
/// d2 = bit width, d3 = bit height) -> how many characters fit.
pub fn text_fit<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, s, n, te, ce) = (cpu.areg(1), cpu.areg(0), cpu.dreg(0) as u16 as usize, cpu.areg(2), cpu.areg(3));
    let (dir, mut w, mut h) = (sx(cpu.dreg(1)), cpu.dreg(2) as u16 as i32, cpu.dreg(3) as u16 as i32);
    let m = Refusal::mem("TextFit");
    if ce != 0 {
        let r = |o| a.mem.read_i16(ce + 4 + o).map(i32::from).map_err(&m);
        w = r(4)? - r(0)? + 1;
        h = r(6)? - r(2)? + 1;
    }
    let bytes = read_str(a, s, n, if dir < 0 { -1 } else { 1 })?;
    let (_, font) = fonts::rp_font(a, rp)?;
    let fit = if font.y_size as i32 > h { 0 } else { font.fit(&bytes, w, false) };
    if te != 0 {
        let ex = font.extent(&bytes[..fit], rastport::read(a, rp)?.algo);
        write_extent(a, te, ex.width, ex.rect, font.y_size as u32)?;
    }
    cpu.set_d0(fit as u32);
    Ok(format!("[graphics] TextFit {n} chars in {w}x{h} dir {dir} -> {fit}"))
}

/// AskSoftStyle(a1 = rp): the algorithmic styles the font does not already have.
pub fn ask_soft_style<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rp = cpu.areg(1);
    let (tf, _) = fonts::rp_font(a, rp)?;
    let style = a.mem.read_u8(tf + TF_STYLE).map_err(Refusal::mem("tf_Style"))?;
    let v = !style & FS_ALGO;
    cpu.set_d0(v as u32);
    Ok(format!("[graphics] AskSoftStyle -> {v:#x}"))
}

/// SetSoftStyle(a1 = rp, d0 = style, d1 = enable) -> the style now in effect.
pub fn set_soft_style<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, style, enable) = (cpu.areg(1), cpu.dreg(0) as u8, cpu.dreg(1) as u8);
    let (tf, _) = fonts::rp_font(a, rp)?;
    let fstyle = a.mem.read_u8(tf + TF_STYLE).map_err(Refusal::mem("tf_Style"))?;
    let allowed = !fstyle & FS_ALGO & enable;
    let cur = a.mem.read_u8(rp + RP_ALGO_STYLE).map_err(Refusal::mem("AlgoStyle"))?;
    let new = (cur & !allowed) | (style & allowed);
    a.mem.write_u8(rp + RP_ALGO_STYLE, new).map_err(Refusal::mem("AlgoStyle"))?;
    cpu.set_d0((new | fstyle) as u32);
    Ok(format!("[graphics] SetSoftStyle {rp:#x} style={style:#x} enable={enable:#x} -> {new:#x}"))
}
