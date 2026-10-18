//! Per-start layout of the graphics state: the ROM font, the Workbench screen's RastPort, BitMap, ViewPort ColorMap and
//! DrawInfo, GfxBase fields. Runs from `start_hunk` after the screen block is written.
use super::layout::*;
use super::{bitmaps, fonts, palette, rastport};
use crate::{machine::Amiga, screen, Refusal};
use guestui_abi::GuestUi;

/// DrawInfo pens of a screen with 4 or more colours (AROS intuition_misc.c DriPens4).
const DEFAULT_DRI_PENS: [u16; DRIPEN_COUNT] = [1, 0, 1, 2, 1, 3, 1, 0, 2, 1, 2, 1];
pub const DRI_VERSION_V: u16 = 3;
const DRIF_NEWLOOK: u32 = 1;
const COLORMAP_TYPE_V39: u8 = 2;
const CHECK_MARK: u32 = 0x8F40;
const AMIGA_KEY: u32 = 0x8F80;
const IMAGE_DATA: u32 = 0x8FC0;

/// A small 1-plane Image (ours): `rows` words, width `w`, drawn in the pen PlaneOnOff selects.
fn write_image<U: GuestUi>(a: &mut Amiga<U>, at: u32, data: u32, w: i16, rows: &[u16]) -> Result<(), Refusal> {
    let m = Refusal::mem("DrawInfo image");
    a.mem.zero(at, IM_SIZE).map_err(&m)?;
    a.mem.write_u16(at + 4, w as u16).map_err(&m)?;
    a.mem.write_u16(at + 6, rows.len() as u16).map_err(&m)?;
    a.mem.write_u16(at + IM_DEPTH, 1).map_err(&m)?;
    a.mem.write_u32(at + IM_DATA, data).map_err(&m)?;
    a.mem.write_u8(at + IM_PLANE_PICK, 1).map_err(&m)?;
    for (i, r) in rows.iter().enumerate() { a.mem.write_u16(data + 2 * i as u32, *r).map_err(&m)?; }
    Ok(())
}

/// (Re)write the guest-visible layout. Host-side state (bitmaps, windows, palette, fonts) persists across starts, as the
/// guest heap does; only the fixed structures `screen::setup` zeroes are laid out again.
pub fn setup<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    fonts::install_rom_font(a)?;
    let m = Refusal::mem("screen graphics");
    let scr = screen::SCREEN;
    // The screen's BitMap and RastPort (drawing into it is accepted and not presented: only windows are shown).
    let bm = scr + SC_BITMAP;
    bitmaps::write_struct(a, bm, SCREEN_WIDTH, SCREEN_HEIGHT, SCREEN_DEPTH)?;
    a.gfx.bitmaps.entry(bm).or_insert_with(|| super::state::GBitmap { bm: retro_raster::Bitmap::new(SCREEN_WIDTH, SCREEN_HEIGHT, SCREEN_DEPTH).unwrap(), window: None });
    let rp = scr + SC_RASTPORT;
    rastport::init(a, rp)?;
    a.mem.write_u8(rp + RP_FG_PEN, 1).map_err(&m)?;
    a.mem.write_u32(rp + RP_BITMAP, bm).map_err(&m)?;
    // The ViewPort and its ColorMap.
    let vp = scr + SC_VIEWPORT;
    a.mem.write_u32(vp + VP_COLORMAP, SCREEN_COLORMAP).map_err(&m)?;
    a.mem.write_u16(vp + VP_DWIDTH, SCREEN_WIDTH as u16).map_err(&m)?;
    a.mem.write_u16(vp + VP_DHEIGHT, SCREEN_HEIGHT as u16).map_err(&m)?;
    a.mem.write_u16(vp + VP_MODES, 0x8000).map_err(&m)?;
    a.mem.zero(SCREEN_COLORMAP, CM_SIZE).map_err(&m)?;
    a.mem.write_u8(SCREEN_COLORMAP + 1, COLORMAP_TYPE_V39).map_err(&m)?;
    a.mem.write_u16(SCREEN_COLORMAP + CM_COUNT, 256).map_err(&m)?;
    a.mem.write_u32(SCREEN_COLORMAP + CM_COLOR_TABLE, SCREEN_COLOR_TABLE).map_err(&m)?;
    palette::changed(a)?;
    // The DrawInfo.
    a.mem.zero(SCREEN_DRAWINFO, DRI_SIZE).map_err(&m)?;
    a.mem.write_u16(SCREEN_DRAWINFO + DRI_VERSION, DRI_VERSION_V).map_err(&m)?;
    a.mem.write_u16(SCREEN_DRAWINFO + DRI_NUM_PENS, DRIPEN_COUNT as u16).map_err(&m)?;
    a.mem.write_u32(SCREEN_DRAWINFO + DRI_PENS, SCREEN_DRI_PENS).map_err(&m)?;
    for (i, p) in DEFAULT_DRI_PENS.iter().enumerate() { a.mem.write_u16(SCREEN_DRI_PENS + 2 * i as u32, *p).map_err(&m)?; }
    a.mem.write_u32(SCREEN_DRAWINFO + DRI_FONT, ROM_FONT).map_err(&m)?;
    a.mem.write_u16(SCREEN_DRAWINFO + DRI_DEPTH, SCREEN_DEPTH as u16).map_err(&m)?;
    a.mem.write_u16(SCREEN_DRAWINFO + DRI_RESOLUTION, 44).map_err(&m)?;
    a.mem.write_u16(SCREEN_DRAWINFO + DRI_RESOLUTION + 2, 44).map_err(&m)?;
    a.mem.write_u32(SCREEN_DRAWINFO + DRI_FLAGS, DRIF_NEWLOOK).map_err(&m)?;
    a.mem.write_u32(SCREEN_DRAWINFO + DRI_CHECK_MARK, CHECK_MARK).map_err(&m)?;
    a.mem.write_u32(SCREEN_DRAWINFO + DRI_AMIGA_KEY, AMIGA_KEY).map_err(&m)?;
    // check mark 11x10 and Amiga key 23x10 as one-plane images (drawn by DrawImage in the pen PlaneOnOff supplies).
    write_image(a, CHECK_MARK, IMAGE_DATA, 11, &[0x0020, 0x0020, 0x0040, 0x0040, 0x8080, 0xC100, 0x6200, 0x3400, 0x1800, 0x0800])?;
    write_image(a, AMIGA_KEY, IMAGE_DATA + 32, 16, &[0x03C0, 0x0C30, 0x1008, 0x2004, 0x2004, 0x2814, 0x37EC, 0x1008, 0x0C30, 0x03C0])?;
    Ok(())
}
