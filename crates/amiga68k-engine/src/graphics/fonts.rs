//! Fonts: a REAL guest `struct TextFont` for topaz 8 (tf_YSize, tf_XSize, tf_Baseline, CharData/CharLoc, tf_Modulo,
//! tf_LoChar/HiChar) at a fixed guest address, so programs that read the metrics directly work; OpenFont/CloseFont/
//! AddFont/RemFont/AskFont/SetFont over the registered fonts; and the parser that turns ANY guest TextFont (a disk font,
//! a program's own) into the renderer's [`Font`].
//!
//! FONT DATA: topaz 8 is the AROS ROM font data (APL; retro-raster/src/topaz8.rs, noted in LICENSES.md and NOTICE).
//! No Amiga ROM font is used. Other sizes/names resolve only from FONTS: through diskfont.library (diskfont.rs).
use super::layout::*;
use crate::{machine::{Amiga, GRAPHICS_BASE}, Refusal};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;
use retro_raster::font::Font;

const FSF_TAGGED: u8 = 0x80;
const FPF_ROMFONT: u8 = 0x01;
const FPF_DESIGNED: u8 = 0x40;

/// Lay the ROM font out in guest memory and register it as the default.
pub fn install_rom_font<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    use retro_raster::topaz8 as t;
    let m = Refusal::mem("ROM font");
    a.mem.zero(ROM_FONT, TF_SIZE).map_err(&m)?;
    for (i, b) in b"topaz.font\0".iter().enumerate() { a.mem.write_u8(ROM_FONT_NAME + i as u32, *b).map_err(&m)?; }
    a.mem.write_bytes(ROM_FONT_DATA, &t::CHAR_DATA).map_err(&m)?;
    for (i, l) in t::CHAR_LOC.iter().enumerate() { a.mem.write_u32(ROM_FONT_LOC + 4 * i as u32, *l).map_err(&m)?; }
    a.mem.write_u8(ROM_FONT + TF_NODE_TYPE, NT_FONT).map_err(&m)?;
    a.mem.write_u32(ROM_FONT + TF_NODE_NAME, ROM_FONT_NAME).map_err(&m)?;
    a.mem.write_u16(ROM_FONT + TF_Y_SIZE, t::Y_SIZE).map_err(&m)?;
    a.mem.write_u8(ROM_FONT + TF_STYLE, FSF_TAGGED).map_err(&m)?;
    a.mem.write_u8(ROM_FONT + TF_FLAGS, FPF_ROMFONT | FPF_DESIGNED).map_err(&m)?;
    a.mem.write_u16(ROM_FONT + TF_X_SIZE, t::X_SIZE).map_err(&m)?;
    a.mem.write_u16(ROM_FONT + TF_BASELINE, t::BASELINE).map_err(&m)?;
    a.mem.write_u16(ROM_FONT + TF_BOLD_SMEAR, t::BOLD_SMEAR).map_err(&m)?;
    a.mem.write_u8(ROM_FONT + TF_LO_CHAR, t::LO_CHAR).map_err(&m)?;
    a.mem.write_u8(ROM_FONT + TF_HI_CHAR, t::HI_CHAR).map_err(&m)?;
    a.mem.write_u32(ROM_FONT + TF_CHAR_DATA, ROM_FONT_DATA).map_err(&m)?;
    a.mem.write_u16(ROM_FONT + TF_MODULO, t::MODULO).map_err(&m)?;
    a.mem.write_u32(ROM_FONT + TF_CHAR_LOC, ROM_FONT_LOC).map_err(&m)?;
    a.mem.write_u32(GRAPHICS_BASE + GB_DEFAULT_FONT, ROM_FONT).map_err(&m)?;
    a.gfx.fonts.clear();
    a.gfx.fonts.push(ROM_FONT);
    a.gfx.rom_font = None;
    Ok(())
}

/// GfxBase->DefaultFont.
pub fn default_font<U: GuestUi>(a: &Amiga<U>) -> Result<u32, Refusal> {
    let f = a.mem.read_u32(GRAPHICS_BASE + GB_DEFAULT_FONT).map_err(Refusal::mem("GfxBase.DefaultFont"))?;
    Ok(if f == 0 { ROM_FONT } else { f })
}

/// The renderer's view of the guest TextFont at `tf`. Validated: a malformed font is a named refusal.
pub fn parse(mem: &GuestMem, tf: u32) -> Result<Font, Refusal> {
    let m = Refusal::mem("TextFont");
    let (y, x, base, smear) = (mem.read_u16(tf + TF_Y_SIZE).map_err(&m)?, mem.read_u16(tf + TF_X_SIZE).map_err(&m)?, mem.read_u16(tf + TF_BASELINE).map_err(&m)?, mem.read_u16(tf + TF_BOLD_SMEAR).map_err(&m)?);
    let (flags, lo, hi) = (mem.read_u8(tf + TF_FLAGS).map_err(&m)?, mem.read_u8(tf + TF_LO_CHAR).map_err(&m)?, mem.read_u8(tf + TF_HI_CHAR).map_err(&m)?);
    let (data, modulo, loc, space, kern) = (mem.read_u32(tf + TF_CHAR_DATA).map_err(&m)?, mem.read_u16(tf + TF_MODULO).map_err(&m)?, mem.read_u32(tf + TF_CHAR_LOC).map_err(&m)?, mem.read_u32(tf + TF_CHAR_SPACE).map_err(&m)?, mem.read_u32(tf + TF_CHAR_KERN).map_err(&m)?);
    if y == 0 || y > 512 || hi < lo || modulo == 0 || data == 0 || loc == 0 {
        return Err(Refusal(format!("TextFont {tf:#x}: not a usable font (YSize {y}, chars {lo}..{hi}, Modulo {modulo}, CharData {data:#x}, CharLoc {loc:#x})")));
    }
    let n = (hi - lo) as usize + 2;
    let bytes = mem.bytes(mem.span(data, modulo as u32 * y as u32).map_err(&m)?).map_err(&m)?.to_vec();
    let locs = (0..n).map(|i| mem.read_u32(loc + 4 * i as u32).map_err(&m)).collect::<Result<_, _>>()?;
    let words = |p: u32| -> Result<Option<alloc::vec::Vec<i16>>, Refusal> {
        if p == 0 { return Ok(None); }
        (0..n).map(|i| mem.read_i16(p + 2 * i as u32).map_err(&m)).collect::<Result<_, _>>().map(Some)
    };
    Ok(Font { y_size: y, x_size: x, baseline: base, bold_smear: smear, flags, lo, hi, modulo, data: bytes, loc: locs, space: words(space)?, kern: words(kern)? })
}

/// The renderer font for the guest TextFont `tf` (the ROM font is parsed once).
pub fn font_at<U: GuestUi>(a: &mut Amiga<U>, tf: u32) -> Result<Rc<Font>, Refusal> {
    if tf == ROM_FONT {
        if let Some(f) = &a.gfx.rom_font { return Ok(f.clone()); }
        let f = Rc::new(parse(&a.mem, tf)?);
        a.gfx.rom_font = Some(f.clone());
        return Ok(f);
    }
    Ok(Rc::new(parse(&a.mem, tf)?))
}

/// The RastPort's font (NULL = the default font): its address and renderer.
pub fn rp_font<U: GuestUi>(a: &mut Amiga<U>, rp: u32) -> Result<(u32, Rc<Font>), Refusal> {
    let mut tf = a.mem.read_u32(rp + RP_FONT).map_err(Refusal::mem("RastPort.Font"))?;
    if tf == 0 { tf = default_font(a)?; }
    Ok((tf, font_at(a, tf)?))
}

fn node_name<U: GuestUi>(a: &Amiga<U>, tf: u32) -> Result<String, Refusal> {
    let p = a.mem.read_u32(tf + TF_NODE_NAME).map_err(Refusal::mem("TextFont name"))?;
    if p == 0 { return Ok(String::new()); }
    a.mem.read_cstr(p).map_err(Refusal::mem("TextFont name"))
}

/// A resident font matching `name` (case-insensitive) and exact `ysize`.
pub fn find_resident<U: GuestUi>(a: &Amiga<U>, name: &str, ysize: u16) -> Result<Option<u32>, Refusal> {
    for &tf in &a.gfx.fonts {
        if node_name(a, tf)?.eq_ignore_ascii_case(name) && a.mem.read_u16(tf + TF_Y_SIZE).map_err(Refusal::mem("TextFont"))? == ysize { return Ok(Some(tf)); }
    }
    Ok(None)
}

fn read_text_attr<U: GuestUi>(a: &Amiga<U>, ta: u32) -> Result<(String, u16), Refusal> {
    let m = Refusal::mem("TextAttr");
    let name = a.mem.read_u32(ta + TA_NAME).map_err(&m)?;
    Ok((a.mem.read_cstr(name).map_err(&m)?, a.mem.read_u16(ta + TA_Y_SIZE).map_err(&m)?))
}

fn bump<U: GuestUi>(a: &mut Amiga<U>, tf: u32, by: i32) -> Result<(), Refusal> {
    let m = Refusal::mem("tf_Accessors");
    let n = a.mem.read_u16(tf + TF_ACCESSORS).map_err(&m)? as i32 + by;
    a.mem.write_u16(tf + TF_ACCESSORS, n.max(0) as u16).map_err(&m)
}

/// OpenFont(a0 = TextAttr) -> a resident font or NULL.
pub fn open_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (name, y) = read_text_attr(a, cpu.areg(0))?;
    let found = find_resident(a, &name, y)?;
    if let Some(tf) = found { bump(a, tf, 1)?; }
    cpu.set_d0(found.unwrap_or(0));
    Ok(format!("[graphics] OpenFont {name:?} {y} -> {:#x}{}", found.unwrap_or(0), if found.is_none() { " (not resident)" } else { "" }))
}

pub fn close_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let tf = cpu.areg(1);
    if tf != 0 { bump(a, tf, -1)?; }
    Ok(format!("[graphics] CloseFont {tf:#x}"))
}

pub fn add_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let tf = cpu.areg(1);
    parse(&a.mem, tf)?; // a font that cannot be rendered is refused at the door
    if !a.gfx.fonts.contains(&tf) { a.gfx.fonts.push(tf); }
    Ok(format!("[graphics] AddFont {tf:#x}"))
}

pub fn rem_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let tf = cpu.areg(1);
    if tf == ROM_FONT { return Err(Refusal("RemFont: the ROM font cannot be removed".into())); }
    a.gfx.fonts.retain(|&f| f != tf);
    Ok(format!("[graphics] RemFont {tf:#x}"))
}

/// SetFont(a1 = rp, a0 = font).
pub fn set_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, tf) = (cpu.areg(1), cpu.areg(0));
    if tf != 0 { parse(&a.mem, tf)?; }
    super::rastport::set_font(a, rp, tf)?;
    Ok(format!("[graphics] SetFont {rp:#x} {tf:#x}"))
}

/// AskFont(a1 = rp, a0 = TextAttr to fill).
pub fn ask_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, ta) = (cpu.areg(1), cpu.areg(0));
    let (tf, _) = rp_font(a, rp)?;
    let m = Refusal::mem("AskFont");
    let name = a.mem.read_u32(tf + TF_NODE_NAME).map_err(&m)?;
    a.mem.write_u32(ta + TA_NAME, name).map_err(&m)?;
    let (y, s, f) = (a.mem.read_u16(tf + TF_Y_SIZE).map_err(&m)?, a.mem.read_u8(tf + TF_STYLE).map_err(&m)?, a.mem.read_u8(tf + TF_FLAGS).map_err(&m)?);
    a.mem.write_u16(ta + TA_Y_SIZE, y).map_err(&m)?;
    a.mem.write_u8(ta + TA_STYLE, s).map_err(&m)?;
    a.mem.write_u8(ta + TA_FLAGS, f).map_err(&m)?;
    Ok(format!("[graphics] AskFont {rp:#x} -> {tf:#x} ysize {y}"))
}

/// FontExtent(a0 = font, a1 = TextExtent): the extent of the whole font's character cell.
pub fn font_extent<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tf, te) = (cpu.areg(0), cpu.areg(1));
    let f = font_at(a, tf)?;
    let max_adv = (f.lo..=f.hi).map(|c| f.advance(c)).max().unwrap_or(0).max(f.x_size as i32);
    super::text::write_extent(a, te, max_adv, retro_raster::Rect::new(0, -(f.baseline as i32), max_adv, f.y_size as i32 - f.baseline as i32), f.y_size as u32)?;
    Ok(format!("[graphics] FontExtent {tf:#x}"))
}
