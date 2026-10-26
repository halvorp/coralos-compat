//! graphics.library (and the layers.library / intuition drawing calls riding on it): raster SEMANTICS, not Amiga
//! hardware (04-AMIGA-ENGINE.md s11). A program's windows get a content canvas; its RastPort calls draw into host
//! bitmaps through `heirloom-raster`; the damage reaches the UI once per engine step as `GuestUi::present_content`.
//!
//! Layouts qualified: CHUNKY bitmaps of 1..=8 bits (see `bitmaps.rs` for what that means for planar programs). Fonts:
//! topaz 8 resident (AROS ROM font data, APL), others from FONTS: via diskfont.library. Everything not implemented
//! stays a NAMED `unimplemented` refusal (the census runner keys on it).
pub mod layout;
mod areafill;
mod areas;
pub(crate) mod bitmaps;
mod bltpattern;
mod blits;
pub mod content;
mod diskfont;
mod flood;
pub(crate) mod modes;
mod modes_best;
mod draw;
pub(crate) mod drawinfo;
pub mod fonts;
pub(crate) mod palette;
mod videocontrol;
mod pixelarrays;
pub mod rastport;
pub mod record;
pub(crate) mod regions;
mod setup;
pub mod state;
pub mod target;
mod text;
mod windows;
pub use drawinfo::draw_image_at;
pub use regions::{mark_damaged, release_damage};
pub use windows::resize as resize_window;

pub use setup::setup;
pub use windows::{close as close_window_surface, open as open_window_surface};

use crate::libtable::LibraryBuilder;
use heirloom_ui_abi::GuestUi;

/// topaz 8: every glyph is 8 pixels wide (gt_menus lays menu items out with it).
pub const TOPAZ8_WIDTH: u32 = 8;

/// graphics.library's implemented vectors.
pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("PolyDraw", draw::poly_draw::<U>).named("InitArea", areas::init_area::<U>).named("AreaMove", areas::area_move::<U>).named("AreaDraw", areas::area_draw::<U>).named("AreaEllipse", areas::area_ellipse::<U>).named("AreaEnd", areas::area_end::<U>).named("BltPattern", bltpattern::blt_pattern::<U>).with(modes::register::<U>).with(videocontrol::register::<U>).with(pixelarrays::register::<U>).named("Flood", flood::flood::<U>).named("InitRastPort", rastport::init_rast_port::<U>)
        .named("SetAPen", rastport::set_a_pen::<U>)
        .named("SetBPen", rastport::set_b_pen::<U>)
        .named("SetDrMd", rastport::set_dr_md::<U>)
        .named("SetABPenDrMd", rastport::set_abpen_drmd::<U>)
        .named("GetAPen", rastport::get_a_pen::<U>)
        .named("GetBPen", rastport::get_b_pen::<U>)
        .named("GetDrMd", rastport::get_dr_md::<U>)
        .named("GetOutlinePen", rastport::get_outline_pen::<U>)
        .named("SetWriteMask", rastport::set_write_mask::<U>)
        .named("SetMaxPen", rastport::set_max_pen::<U>)
        .named("SetRPAttrsA", rastport::set_rp_attrs::<U>)
        .named("GetRPAttrsA", rastport::get_rp_attrs::<U>)
        .named("Move", rastport::move_::<U>)
        .named("Draw", draw::draw::<U>)
        .named("RectFill", draw::rect_fill::<U>)
        .named("EraseRect", draw::erase_rect::<U>)
        .named("SetRast", draw::set_rast::<U>)
        .named("ClearEOL", draw::clear_eol::<U>)
        .named("ClearScreen", draw::clear_screen::<U>)
        .named("ReadPixel", draw::read_pixel::<U>)
        .named("WritePixel", draw::write_pixel::<U>)
        .named("ScrollRaster", draw::scroll_raster::<U>)
        .named("ScrollRasterBF", draw::scroll_raster::<U>)
        .named("BltBitMap", blits::blt_bit_map::<U>)
        .named("BltBitMapRastPort", blits::blt_bit_map_rast_port::<U>)
        .named("ClipBlit", blits::clip_blit::<U>)
        .named("BltMaskBitMapRastPort", blits::blt_mask_bit_map_rast_port::<U>)
        .named("BltTemplate", blits::blt_template::<U>)
        .named("BltClear", bitmaps::blt_clear::<U>)
        .named("AllocBitMap", bitmaps::alloc_bit_map::<U>)
        .named("FreeBitMap", bitmaps::free_bit_map::<U>)
        .named("InitBitMap", bitmaps::init_bit_map::<U>)
        .named("AllocRaster", bitmaps::alloc_raster::<U>)
        .named("FreeRaster", bitmaps::free_raster::<U>)
        .named("GetBitMapAttr", bitmaps::get_bit_map_attr::<U>)
        .named("Text", text::text::<U>)
        .named("TextLength", text::text_length::<U>)
        .named("TextExtent", text::text_extent::<U>)
        .named("TextFit", text::text_fit::<U>)
        .named("AskSoftStyle", text::ask_soft_style::<U>)
        .named("SetSoftStyle", text::set_soft_style::<U>)
        .named("OpenFont", fonts::open_font::<U>)
        .named("CloseFont", fonts::close_font::<U>)
        .named("AddFont", fonts::add_font::<U>)
        .named("RemFont", fonts::rem_font::<U>)
        .named("SetFont", fonts::set_font::<U>)
        .named("AskFont", fonts::ask_font::<U>)
        .named("FontExtent", fonts::font_extent::<U>)
        .named("NewRegion", regions::new_region::<U>)
        .named("DisposeRegion", regions::dispose_region::<U>)
        .named("ClearRegion", regions::clear_region::<U>)
        .named("OrRectRegion", regions::or_rect_region::<U>)
        .named("AndRectRegion", regions::and_rect_region::<U>)
        .named("XorRectRegion", regions::xor_rect_region::<U>)
        .named("ClearRectRegion", regions::clear_rect_region::<U>)
        .named("OrRegionRegion", regions::or_region_region::<U>)
        .named("AndRegionRegion", regions::and_region_region::<U>)
        .named("XorRegionRegion", regions::xor_region_region::<U>)
        .named("ClearRegionRegion", regions::clear_region_region::<U>)
        .named("IsPointInRegion", regions::is_point_in_region::<U>)
        .named("SetRGB4", palette::set_rgb4::<U>)
        .named("SetRGB32", palette::set_rgb32::<U>)
        .named("LoadRGB4", palette::load_rgb4::<U>)
        .named("LoadRGB32", palette::load_rgb32::<U>)
        .named("GetRGB4", palette::get_rgb4::<U>)
        .named("GetRGB32", palette::get_rgb32::<U>)
        .named("ObtainPen", palette::obtain_pen::<U>)
        .named("ReleasePen", palette::release_pen::<U>)
        .named("ObtainBestPenA", palette::obtain_best_pen::<U>)
        .named("FindColor", palette::find_color::<U>)
        .named("WaitBlit", drawinfo::true_noop::<U>)
        .named("WaitTOF", drawinfo::true_noop::<U>)
        .named("OwnBlitter", drawinfo::true_noop::<U>)
        .named("DisownBlitter", drawinfo::true_noop::<U>)
}

/// intuition.library's drawing, refresh and pointer calls.
pub fn register_intuition<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("GetScreenDrawInfo", drawinfo::get_screen_draw_info::<U>)
        .named("FreeScreenDrawInfo", drawinfo::free_screen_draw_info::<U>)
        .named("DrawImage", drawinfo::draw_image::<U>)
        .named("DrawImageState", drawinfo::draw_image_state::<U>)
        .named("EraseImage", drawinfo::erase_image::<U>)
        .named("PointInImage", drawinfo::point_in_image::<U>)
        .named("DrawBorder", drawinfo::draw_border::<U>)
        .named("PrintIText", drawinfo::print_itext::<U>)
        .named("IntuiTextLength", drawinfo::intui_text_length::<U>)
        .named("ObtainGIRPort", drawinfo::obtain_gi_rport::<U>)
        .named("ReleaseGIRPort", drawinfo::log_only::<U>)
        .named("BeginRefresh", regions::begin_refresh::<U>)
        .named("EndRefresh", regions::end_refresh::<U>)
        .named("RefreshWindowFrame", drawinfo::log_only::<U>)
        .named("DisplayBeep", drawinfo::log_only::<U>)
        .named("ClearPointer", drawinfo::log_only::<U>)
        .named("SetPointer", drawinfo::log_only::<U>)
        .named("SetWindowPointerA", drawinfo::log_only::<U>)
        .named("ScrollWindowRaster", drawinfo::scroll_window_raster::<U>)
        .named("GetScreenData", drawinfo::get_screen_data::<U>)
}

/// layers.library, opened on demand: clip regions are real; locking is a no-op (tasks switch only after a library call, so a lock is never contended).
pub fn layers_library<U: GuestUi>(base: u32) -> LibraryBuilder<U> {
    LibraryBuilder::new("layers.library", base)
        .named("InstallClipRegion", regions::install_clip_region::<U>)
        .named("LockLayer", drawinfo::true_noop::<U>)
        .named("UnlockLayer", drawinfo::true_noop::<U>)
        .named("LockLayers", drawinfo::true_noop::<U>)
        .named("UnlockLayers", drawinfo::true_noop::<U>)
        .named("LockLayerInfo", drawinfo::true_noop::<U>)
        .named("UnlockLayerInfo", drawinfo::true_noop::<U>)
        .named("BeginUpdate", drawinfo::true_noop::<U>)
        .named("EndUpdate", drawinfo::true_noop::<U>)
}

/// diskfont.library, opened on demand.
pub fn diskfont_library<U: GuestUi>(base: u32) -> LibraryBuilder<U> {
    LibraryBuilder::new("diskfont.library", base).named("OpenDiskFont", diskfont::open_disk_font::<U>)
}
