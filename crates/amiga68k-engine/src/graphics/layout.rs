//! Guest struct layouts graphics.library hands to programs, and the fixed guest addresses of the objects the engine
//! lays out itself. Offsets are the m68k ABI's (AROS headers, field lists in graphics/{rastport,gfx,text,regions,clip}.h
//! and intuition/{intuition,screens}.h); `tests/guest/gfx_offsets.c` re-derives every one with
//! `m68k-linux-gnu-gcc -m68020 -mno-align-int` and `tests/gfx_layout.rs` compares that table to these constants.

// struct RastPort (100 bytes)
pub const RP_SIZE: u32 = 100;
pub const RP_LAYER: u32 = 0;
pub const RP_BITMAP: u32 = 4;
pub const RP_AREA_PTRN: u32 = 8;
pub const RP_TMP_RAS: u32 = 12;
pub const RP_AREA_INFO: u32 = 16;
pub const RP_MASK: u32 = 24;
pub const RP_FG_PEN: u32 = 25;
pub const RP_BG_PEN: u32 = 26;
pub const RP_AOL_PEN: u32 = 27;
pub const RP_DRAW_MODE: u32 = 28;
pub const RP_AREA_PT_SZ: u32 = 29;
pub const RP_LINPATCNT: u32 = 30;
pub const RP_FLAGS: u32 = 32;
pub const RP_LINE_PTRN: u32 = 34;
pub const RP_CP_X: u32 = 36;
pub const RP_CP_Y: u32 = 38;
pub const RP_MINTERMS: u32 = 40;
pub const RP_PEN_WIDTH: u32 = 48;
pub const RP_PEN_HEIGHT: u32 = 50;
pub const RP_FONT: u32 = 52;
pub const RP_ALGO_STYLE: u32 = 56;
pub const RP_TX_FLAGS: u32 = 57;
pub const RP_TX_HEIGHT: u32 = 58;
pub const RP_TX_WIDTH: u32 = 60;
pub const RP_TX_BASELINE: u32 = 62;
pub const RP_TX_SPACING: u32 = 64;

// struct BitMap (40 bytes)
pub const BM_SIZE: u32 = 40;
pub const BM_BYTES_PER_ROW: u32 = 0;
pub const BM_ROWS: u32 = 2;
pub const BM_FLAGS: u32 = 4;
pub const BM_DEPTH: u32 = 5;
pub const BM_PLANES: u32 = 8;

// struct TextFont (52 bytes) / TextAttr (8 bytes)
pub const TF_SIZE: u32 = 52;
pub const TF_NODE_TYPE: u32 = 8;
pub const TF_NODE_NAME: u32 = 10;
pub const TF_Y_SIZE: u32 = 20;
pub const TF_STYLE: u32 = 22;
pub const TF_FLAGS: u32 = 23;
pub const TF_X_SIZE: u32 = 24;
pub const TF_BASELINE: u32 = 26;
pub const TF_BOLD_SMEAR: u32 = 28;
pub const TF_ACCESSORS: u32 = 30;
pub const TF_LO_CHAR: u32 = 32;
pub const TF_HI_CHAR: u32 = 33;
pub const TF_CHAR_DATA: u32 = 34;
pub const TF_MODULO: u32 = 38;
pub const TF_CHAR_LOC: u32 = 40;
pub const TF_CHAR_SPACE: u32 = 44;
pub const TF_CHAR_KERN: u32 = 48;
pub const TA_SIZE: u32 = 8;
pub const TA_NAME: u32 = 0;
pub const TA_Y_SIZE: u32 = 4;
pub const TA_STYLE: u32 = 6;
pub const TA_FLAGS: u32 = 7;
/// struct TextExtent: te_Width u16 @0, te_Height u16 @2, te_Extent (Rectangle: MinX MinY MaxX MaxY i16) @4. 12 bytes.
pub const TE_SIZE: u32 = 12;

// struct Region (12 bytes) / RegionRectangle (16 bytes) / Rectangle (8 bytes of 4 WORDs)
pub const RG_SIZE: u32 = 12;
pub const RG_BOUNDS: u32 = 0;
pub const RG_FIRST: u32 = 8;
pub const RR_SIZE: u32 = 16;
pub const RR_NEXT: u32 = 0;
pub const RR_PREV: u32 = 4;
pub const RR_BOUNDS: u32 = 8;

// struct Layer (the fields programs read)
pub const LY_SIZE: u32 = 160;
pub const LY_RP: u32 = 12;
pub const LY_BOUNDS: u32 = 16;
pub const LY_FLAGS: u32 = 30;
pub const LY_WINDOW: u32 = 40;
pub const LY_SCROLL_X: u32 = 44;
pub const LY_SCROLL_Y: u32 = 46;
pub const LY_CLIP_REGION: u32 = 126;

// struct Window fields graphics touches (intuition.h)
pub const WD_RPORT: u32 = 50;
pub const WD_BORDER_RPORT: u32 = 58;
pub const WD_WLAYER: u32 = 124;
pub const WD_IFONT: u32 = 128;
pub const WD_GZZ_WIDTH: u32 = 112;
pub const WD_GZZ_HEIGHT: u32 = 114;

// struct Screen fields (intuition/screens.h)
pub const SC_VIEWPORT: u32 = 44;
pub const SC_RASTPORT: u32 = 84;
pub const SC_BITMAP: u32 = 184;
pub const SC_LAYER_INFO: u32 = 224;
// struct ViewPort: ColorMap @4, DWidth @24, DHeight @26, Modes @32
pub const VP_COLORMAP: u32 = 4;
pub const VP_DWIDTH: u32 = 24;
pub const VP_DHEIGHT: u32 = 26;
pub const VP_MODES: u32 = 32;
// struct ColorMap: Flags u8 @0, Type u8 @1, Count u16 @2, ColorTable @4
pub const CM_SIZE: u32 = 52;
pub const CM_COUNT: u32 = 2;
pub const CM_COLOR_TABLE: u32 = 4;

// struct GfxBase: DefaultFont @154
pub const GB_DEFAULT_FONT: u32 = 154;
pub const GB_TEXT_FONTS: u32 = 140;

// struct DrawInfo (intuition/screens.h) 50 bytes
pub const DRI_SIZE: u32 = 50;
pub const DRI_VERSION: u32 = 0;
pub const DRI_NUM_PENS: u32 = 2;
pub const DRI_PENS: u32 = 4;
pub const DRI_FONT: u32 = 8;
pub const DRI_DEPTH: u32 = 12;
pub const DRI_RESOLUTION: u32 = 14;
pub const DRI_FLAGS: u32 = 18;
pub const DRI_CHECK_MARK: u32 = 22;
pub const DRI_AMIGA_KEY: u32 = 26;
pub const DRIPEN_COUNT: usize = 12;

// struct Image / Border / IntuiText (intuition/intuition.h)
pub const IM_SIZE: u32 = 20;
pub const IM_LEFT: u32 = 0;
pub const IM_DEPTH: u32 = 8;
pub const IM_DATA: u32 = 10;
pub const IM_PLANE_PICK: u32 = 14;
pub const IM_PLANE_ON_OFF: u32 = 15;
pub const IM_NEXT: u32 = 16;
pub const BD_SIZE: u32 = 16;
pub const BD_LEFT: u32 = 0;
pub const BD_FRONT_PEN: u32 = 4;
pub const BD_BACK_PEN: u32 = 5;
pub const BD_DRAW_MODE: u32 = 6;
pub const BD_COUNT: u32 = 7;
pub const BD_XY: u32 = 8;
pub const BD_NEXT: u32 = 12;
pub const IT_SIZE: u32 = 20;
pub const IT_FRONT_PEN: u32 = 0;
pub const IT_BACK_PEN: u32 = 1;
pub const IT_DRAW_MODE: u32 = 2;
pub const IT_LEFT: u32 = 4;
pub const IT_TOP: u32 = 6;
pub const IT_FONT: u32 = 8;
pub const IT_TEXT: u32 = 12;
pub const IT_NEXT: u32 = 16;

// RastPort / drawing constants
pub const JAM1: u8 = 0;
pub const JAM2: u8 = 1;
pub const COMPLEMENT: u8 = 2;
pub const INVERSVID: u8 = 4;
pub const FRST_DOT: u16 = 0x0100;
pub const NT_FONT: u8 = 12;

// Fixed guest addresses (free low memory between the gadtools library at 0x7000 and the BOOPSI area at 0xE000).
/// The ROM topaz 8 TextFont, its name, its glyph strip and its CharLoc table.
pub const ROM_FONT: u32 = 0x8000;
pub const ROM_FONT_NAME: u32 = 0x8040;
pub const ROM_FONT_DATA: u32 = 0x8100;
pub const ROM_FONT_LOC: u32 = 0x8700;
/// The screen's ColorMap and its RGB4 ColorTable (256 words).
pub const SCREEN_COLORMAP: u32 = 0x8B00;
pub const SCREEN_COLOR_TABLE: u32 = 0x8C00;
/// The screen's DrawInfo and its pen array.
pub const SCREEN_DRAWINFO: u32 = 0x8E00;
pub const SCREEN_DRI_PENS: u32 = 0x8E40;
pub const SCREEN_DEPTH: u8 = 8;
pub const SCREEN_WIDTH: i32 = 640;
pub const SCREEN_HEIGHT: i32 = 256;

pub fn sx(v: u32) -> i32 { v as u16 as i16 as i32 }
