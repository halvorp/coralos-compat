//! Symbian Font & Bitmap Server (`fbscli.dll`), BitGDI (`bitgdi.dll`), GDI fonts (`gdi.dll`),
//! and Direct Screen Access (`CDirectScreenAccess`, `ws32.dll`).
//!
//! Real bitmaps (created via `CFbsBitmap::Create` or decoded from Symbian `.mbm` multi-bitmap files)
//! are stored in `FbsState` as `0x00RRGGBB` pixel arrays. `CFbsBitGc` draws directly onto its target
//! bitmap (fills, rects, lines, `BitBlt`, `BitBltMasked`, `DrawBitmap`, `DrawBitmapMasked`, clipping),
//! and `CFbsScreenDevice::Update()` presents the screen bitmap through `GuestUi::present_plane`.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;

/// Standard Symbian `TDisplayMode::EColor16MU` (gdi.h:312).
pub const ECOLOR_16MU: u32 = 11;

/// `CDirectScreenAccess` member offsets after `CActive` (28 bytes: vptr 0, iStatus 4, iFlags 8, iLink 12..24, iSpare 24).
pub const DSA_I_GC: u32 = 0x1C;
pub const DSA_I_SCREEN_DEVICE: u32 = 0x20;
pub const DSA_I_DRAWING_REGION: u32 = 0x24;
pub const DSA_BYTES: u32 = 0x60;

/// One host-side raster bitmap backing a `CFbsBitmap` handle.
#[derive(Debug, Clone)]
pub struct FbsBitmap {
    pub width: i32,
    pub height: i32,
    pub display_mode: u32,
    pub is_gray: bool,
    pub pixels: Vec<u32>, // 0x00RRGGBB
    /// Guest-mapped scanline buffer allocated lazily by `CFbsBitmap::DataAddress()`.
    pub data_addr: Option<u32>,
}

impl FbsBitmap {
    pub fn new(width: i32, height: i32, display_mode: u32) -> Self {
        let w = width.max(0);
        let h = height.max(0);
        let is_gray = matches!(display_mode, 1 | 2 | 3 | 4);
        Self {
            width: w,
            height: h,
            display_mode,
            is_gray,
            pixels: vec![0x00FF_FFFF; (w as usize).saturating_mul(h as usize)],
            data_addr: None,
        }
    }
    pub fn scanline_bytes(&self) -> usize {
        let w = self.width.max(0) as usize;
        // Symbian TDisplayMode (gdi.h:287): 1=EGray2, 2=EGray4, 3=EGray16, 4=EGray256,
        // 5=EColor16, 6=EColor256, 7=EColor64K (16bpp), 8=EColor16M (24bpp), 10=EColor4K (12bpp/16bit), 11/12=32bpp.
        match self.display_mode {
            1 => ((w + 31) / 32) * 4,
            2 => ((w * 2 + 31) / 32) * 4,
            3 | 5 => ((w * 4 + 31) / 32) * 4,
            4 | 6 => ((w + 3) / 4) * 4,
            7 | 10 => ((w * 2 + 3) / 4) * 4,
            8 => ((w * 3 + 3) / 4) * 4,
            _ => w * 4, // EColor16MU (11), EColor16MA (12), etc.
        }
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height { return None; }
        self.pixels.get(y as usize * self.width as usize + x as usize).copied()
    }
    #[inline]
    pub fn set(&mut self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height { return; }
        if let Some(p) = self.pixels.get_mut(y as usize * self.width as usize + x as usize) {
            *p = rgb & 0x00FF_FFFF;
        }
    }
}

#[derive(Debug, Clone)]
pub struct FbsDeviceState {
    pub bitmap_handle: u32,
    pub is_screen: bool,
}

#[derive(Debug, Clone)]
pub struct FbsGcState {
    pub device_addr: u32,
    pub bitmap_handle: u32,
    pub origin: (i32, i32),
    pub clip_rect: Option<[i32; 4]>, // [x0, y0, x1, y1]
    pub pen_color: u32,
    pub pen_style: u32,
    pub brush_color: u32,
    pub brush_style: u32,
    pub draw_mode: u32,
    pub pos: (i32, i32),
}

impl FbsGcState {
    pub(crate) fn new(device_addr: u32, bitmap_handle: u32) -> Self {
        Self {
            device_addr,
            bitmap_handle,
            origin: (0, 0),
            clip_rect: None,
            pen_color: 0x0000_0000,
            pen_style: 1, // ESolidPen
            brush_color: 0x00FF_FFFF,
            brush_style: 0, // ENullBrush
            draw_mode: 32, // EDrawModePEN
            pos: (0, 0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DsaState {
    pub screen_bmp_handle: u32,
    pub screen_dev_addr: u32,
    pub gc_addr: u32,
    pub region_addr: u32,
}

pub use crate::audio::AudioOutState;

#[derive(Default)]
pub struct FbsState {
    pub next_handle: u32,
    pub bitmaps: BTreeMap<u32, FbsBitmap>,
    /// Shared screen bitmap handle + `CWindowGc` guest address returned by `SystemGc()`.
    pub system_gc: Option<(u32, u32)>,
}

impl FbsState {
    pub fn insert_bitmap(&mut self, bmp: FbsBitmap) -> u32 {
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        let id = self.next_handle;
        self.bitmaps.insert(id, bmp);
        id
    }
}

/// Symbian 256-color palette entry (6x6x6 cube + grays, EColor256).
pub use crate::mbm::{decode_mbm, decode_mbm_mode};

pub(crate) const DEV_SLOTS_TOTAL: usize = 36;
pub(crate) const DEV_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN10CFbsDeviceD1Ev"),
    (1, "_ZN10CFbsDeviceD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZNK10CFbsDevice11DisplayModeEv"),
    (4, "_ZNK10CFbsDevice12SizeInPixelsEv"),
    (5, "_ZNK10CFbsDevice11SizeInTwipsEv"),
    (6, "_ZN10CFbsDevice13CreateContextERP16CGraphicsContext"),
    (13, "_ZNK10CFbsDevice8GetPixelER4TRgbRK6TPoint"),
    (14, "_ZNK10CFbsDevice11GetScanLineER5TDes8RK6TPointi12TDisplayMode"),
    (15, "_ZN10CFbsDevice7AddFileERK7TDesC16Ri"),
    (16, "_ZN10CFbsDevice10RemoveFileEi"),
    (17, "_ZN10CFbsDevice22GetNearestFontInPixelsERP5CFontRK9TFontSpec"),
    (18, "_ZN10CFbsDevice36GetNearestFontToDesignHeightInPixelsERP5CFontRK9TFontSpec"),
    (19, "_ZN10CFbsDevice33GetNearestFontToMaxHeightInPixelsERP5CFontRK9TFontSpeci"),
    (20, "_ZN10CFbsDevice18FontHeightInPixelsEii"),
    (21, "_ZN10CFbsDevice13CreateContextERP9CFbsBitGc"),
    (22, "_ZN10CFbsDevice12DrawingBeginEi"),
    (23, "_ZN10CFbsDevice10DrawingEndEi"),
    (24, "_ZN10CFbsDevice7SetBitsEv"),
    (25, "_ZNK10CFbsDevice12CancelSpriteEv"),
    (26, "_ZNK10CFbsDevice10HideSpriteEv"),
    (27, "_ZNK10CFbsDevice10HideSpriteERK5TRectPK7TRegion"),
    (28, "_ZNK10CFbsDevice10ShowSpriteEP11TSpriteBase"),
    (29, "_ZNK10CFbsDevice10ShowSpriteEP11TSpriteBaseRK5TRectPK7TRegion"),
];

/// Vtable for `CFbsBitGc` (inherits `CBase` 0..2, `CGraphicsContext` 3..53, `CBitmapContext` 54..65).
pub(crate) const GC_SLOTS_TOTAL: usize = 70;
pub(crate) const GC_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN9CFbsBitGcD1Ev"),
    (1, "_ZN9CFbsBitGcD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZNK9CFbsBitGc6DeviceEv"),
    (4, "_ZN9CFbsBitGc9SetOriginERK6TPoint"),
    (5, "_ZN9CFbsBitGc11SetDrawModeEN16CGraphicsContext9TDrawModeE"),
    (6, "_ZN9CFbsBitGc15SetClippingRectERK5TRect"),
    (7, "_ZN9CFbsBitGc18CancelClippingRectEv"),
    (8, "_ZN9CFbsBitGc5ResetEv"),
    (9, "_ZN9CFbsBitGc7UseFontEPK5CFont"),
    (10, "_ZN9CFbsBitGc11DiscardFontEv"),
    (11, "_ZN9CFbsBitGc17SetUnderlineStyleE14TFontUnderline"),
    (12, "_ZN9CFbsBitGc21SetStrikethroughStyleE18TFontStrikethrough"),
    (13, "_ZN9CFbsBitGc20SetWordJustificationEii"),
    (14, "_ZN9CFbsBitGc20SetCharJustificationEii"),
    (15, "_ZN9CFbsBitGc11SetPenColorERK4TRgb"),
    (16, "_ZN9CFbsBitGc11SetPenStyleEN16CGraphicsContext9TPenStyleE"),
    (17, "_ZN9CFbsBitGc10SetPenSizeERK5TSize"),
    (18, "_ZN9CFbsBitGc13SetBrushColorERK4TRgb"),
    (19, "_ZN9CFbsBitGc13SetBrushStyleEN16CGraphicsContext11TBrushStyleE"),
    (20, "_ZN9CFbsBitGc14SetBrushOriginERK6TPoint"),
    (23, "_ZN9CFbsBitGc6MoveToERK6TPoint"),
    (24, "_ZN9CFbsBitGc6MoveByERK6TPoint"),
    (25, "_ZN9CFbsBitGc4PlotERK6TPoint"),
    (27, "_ZN9CFbsBitGc8DrawLineERK6TPointS2_"),
    (28, "_ZN9CFbsBitGc10DrawLineToERK6TPoint"),
    (29, "_ZN9CFbsBitGc10DrawLineByERK6TPoint"),
    (33, "_ZN9CFbsBitGc11DrawEllipseERK5TRect"),
    (34, "_ZN9CFbsBitGc8DrawRectERK5TRect"),
    (35, "_ZN9CFbsBitGc13DrawRoundRectERK5TRectRK5TSize"),
    (38, "_ZN9CFbsBitGc10DrawBitmapERK6TPointPK10CFbsBitmap"),
    (39, "_ZN9CFbsBitGc10DrawBitmapERK5TRectPK10CFbsBitmap"),
    (40, "_ZN9CFbsBitGc10DrawBitmapERK5TRectPK10CFbsBitmapS2_"),
    (41, "_ZN9CFbsBitGc16DrawBitmapMaskedERK5TRectPK10CFbsBitmapS2_S5_i"),
    (43, "_ZN9CFbsBitGc8DrawTextERK7TDesC16RK6TPoint"),
    (44, "_ZN9CFbsBitGc8DrawTextERK7TDesC16RK5TRectiN16CGraphicsContext10TTextAlignEi"),
    (48, "_ZN9CFbsBitGc17SetClippingRegionERK7TRegion"),
    (49, "_ZN9CFbsBitGc20CancelClippingRegionEv"),
    (54, "_ZN9CFbsBitGc5ClearEv"),
    (55, "_ZN9CFbsBitGc5ClearERK5TRect"),
    (56, "_ZN9CFbsBitGc8CopyRectERK6TPointRK5TRect"),
    (57, "_ZN9CFbsBitGc6BitBltERK6TPointPK10CFbsBitmap"),
    (58, "_ZN9CFbsBitGc6BitBltERK6TPointPK10CFbsBitmapRK5TRect"),
    (59, "_ZN9CFbsBitGc12BitBltMaskedERK6TPointPK10CFbsBitmapRK5TRectS5_i"),
    (60, "_ZN9CFbsBitGc8SetFadedEi"),
    (61, "_ZN9CFbsBitGc19SetFadingParametersEhh"),
    (62, "_ZN9CFbsBitGc17AlphaBlendBitmapsERK6TPointPK10CFbsBitmapRK5TRectS5_S2_"),
];

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let [r0, r1, r2, r3] = [p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]];
    let refuse = |m: String| Some(NativeOutcome::Refused(format!("NEXT-NEEDED fbs {name}: {m}")));
    Some(match name {
        // CFbsBitmap* CEikonEnv::CreateBitmapL(const TDesC16& aFileName, TInt aId, TBool aShowRunning = ETrue)
        "_ZN9CEikonEnv13CreateBitmapLERK7TDesC16i" | "_ZN9CEikonEnv13CreateBitmapLERK7TDesC16ii" => {
            let Ok(units) = crate::desc::read16(p, r1) else { return refuse(format!("bad filename at {r1:#x}")) };
            let raw_path = crate::desc::to_utf8(&units);
            let priv_dir = format!("C:\\private\\{:08x}\\", p.uid3);
            let path = crate::fs::resolve_drive_path(&raw_path, &priv_dir);
            let idx = r2 as usize;
            let bytes = p.host.drive.as_ref().and_then(|d| {
                d.read(&path).ok()
                    .or_else(|| d.read(&format!("{priv_dir}{}", raw_path.trim_start_matches(['\\', '/']))).ok())
                    .or_else(|| {
                        // Strip drive letter (e.g. "C:\bdim.mbm") and check private dir fallback
                        let tail = raw_path.rsplit(['\\', '/']).next().unwrap_or(&raw_path);
                        d.read(&format!("{priv_dir}{tail}")).ok()
                    })
            }).map(|b| b.to_vec());
            let bmp = match bytes.and_then(|b| decode_mbm_mode(&b, idx).ok()) {
                Some((w, h, display_mode, pixels)) => FbsBitmap {
                    width: w, height: h, display_mode, is_gray: matches!(display_mode, 1 | 2 | 3 | 4), pixels, data_addr: None,
                },
                None => return refuse(format!("CreateBitmapL({path}, {idx}) failed to load MBM")),
            };
            let handle = p.nat.fbs.insert_bitmap(bmp);
            let Ok(obj) = p.plain_object("CFbsBitmap", 0x20) else { return refuse("no heap for CFbsBitmap".into()) };
            let _ = p.obj_set(obj, HostObj::FbsBitmap(handle));
            p.cpu.regs[0] = obj;
            NativeOutcome::Return
        }
        // TDisplayMode CEikonEnv::DefaultDisplayMode() const
        "_ZNK9CEikonEnv18DefaultDisplayModeEv" => {
            p.cpu.regs[0] = ECOLOR_16MU;
            NativeOutcome::Return
        }
        // TFontSpec::TFontSpec(const TDesC16& aTypefaceName, TInt aHeight)
        "_ZN9TFontSpecC1ERK7TDesC16i" | "_ZN9TFontSpecC2ERK7TDesC16i" => {
            // Write iHeight at offset 56 (matching screen.rs:62)
            let _ = p.wr(r0 + 56, r2);
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        // CFont basic metrics (matching our 5x7 + 1px gap = 6x12 font)
        "_ZNK5CFont14AscentInPixelsEv" => { p.cpu.regs[0] = 10; NativeOutcome::Return }
        "_ZNK5CFont20MaxCharWidthInPixelsEv" | "_ZNK5CFont17CharWidthInPixelsE5TChar" => { p.cpu.regs[0] = 6; NativeOutcome::Return }
        "_ZNK5CFont17TextWidthInPixelsERK7TDesC16" => {
            let len = crate::desc::read16(p, r1).map(|u| u.len() as u32).unwrap_or(0);
            p.cpu.regs[0] = len * 6;
            NativeOutcome::Return
        }
        // CFont::ExtendedFunction(TUid aFunctionId, TAny* aParam) const (gdi.h:1253-1257)
        "_ZNK5CFont16ExtendedFunctionE4TUidPv" => {
            p.cpu.regs[0] = match r1 {
                0x1020_498E => 10, // KFontCapitalAscent
                0x1020_4B10 => 10, // KFontMaxAscent
                0x1020_4B11 => 2,  // KFontStandardDescent
                0x1020_5AFC => 2,  // KFontMaxDescent
                0x1020_4B12 => 14, // KFontLineGap
                _ => (-5i32) as u32, // KErrNotSupported
            };
            NativeOutcome::Return
        }
        // CFbsBitmap::CFbsBitmap()
        "_ZN10CFbsBitmapC1Ev" | "_ZN10CFbsBitmapC2Ev" => {
            if let Ok(vptr) = p.build_vtable("CFbsBitmap", &slots_with_gaps(4, &[
                (0, "_ZN10CFbsBitmapD1Ev"),
                (1, "_ZN10CFbsBitmapD0Ev"),
                (2, "_ZN5CBase10Extension_EjRPvS0_"),
            ])) {
                let _ = p.wr(r0, vptr);
            }
            let h = p.nat.fbs.insert_bitmap(FbsBitmap::new(0, 0, ECOLOR_16MU));
            let _ = p.obj_set(r0, HostObj::FbsBitmap(h));
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        "_ZN10CFbsBitmapD1Ev" | "_ZN10CFbsBitmapD2Ev" | "_ZN10CFbsBitmapD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") {
                if let Some(heap) = p.heap.as_mut() { let _ = heap.free(r0); }
            }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        // TInt CFbsBitmap::Create(const TSize& aSizeInPixels, TDisplayMode aDispMode)
        "_ZN10CFbsBitmap6CreateERK5TSize12TDisplayMode" => {
            let Ok((w, h)) = p.rd_point(r1) else { return refuse(format!("bad TSize at {r1:#x}")) };
            let bmp = FbsBitmap::new(w, h, r2);
            let handle = p.nat.fbs.insert_bitmap(bmp);
            let _ = p.obj_set(r0, HostObj::FbsBitmap(handle));
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // TInt CFbsBitmap::Load(const TDesC16& aFileName, TInt32 aId, TBool aShareIfLoaded)
        "_ZN10CFbsBitmap4LoadERK7TDesC16li" | "_ZN10CFbsBitmap4LoadERK7TDesC16lii" => {
            let Ok(units) = crate::desc::read16(p, r1) else { return refuse(format!("bad filename at {r1:#x}")) };
            let raw_path = crate::desc::to_utf8(&units);
            let priv_dir = format!("C:\\private\\{:08x}\\", p.uid3);
            let path = crate::fs::resolve_drive_path(&raw_path, &priv_dir);
            let idx = r2 as usize;
            let bytes = p.host.drive.as_ref().and_then(|d| {
                d.read(&path).ok()
                    .or_else(|| {
                        let tail = raw_path.rsplit(['\\', '/']).next().unwrap_or(&raw_path);
                        d.read(&format!("{priv_dir}{tail}")).ok()
                    })
            });
            match bytes {
                Some(b) => match decode_mbm_mode(&b, idx) {
                    Ok((w, h, display_mode, pixels)) => {
                        let bmp = FbsBitmap { width: w, height: h, display_mode, is_gray: matches!(display_mode, 1 | 2 | 3 | 4), pixels, data_addr: None };
                        let handle = p.nat.fbs.insert_bitmap(bmp);
                        let _ = p.obj_set(r0, HostObj::FbsBitmap(handle));
                        p.cpu.regs[0] = 0;
                    }
                    Err(_) => { p.cpu.regs[0] = (-6i32) as u32; } // KErrCorrupt / KErrArgument
                },
                None => { p.cpu.regs[0] = (-1i32) as u32; } // KErrNotFound
            }
            NativeOutcome::Return
        }
        // TInt CFbsBitmap::Duplicate(TInt aHandle)
        "_ZN10CFbsBitmap9DuplicateEi" => {
            if p.nat.fbs.bitmaps.contains_key(&r1) {
                let _ = p.obj_set(r0, HostObj::FbsBitmap(r1));
                p.cpu.regs[0] = 0;
            } else {
                p.cpu.regs[0] = (-1i32) as u32;
            }
            NativeOutcome::Return
        }
        // TInt CFbsBitmap::Handle() const
        "_ZNK10CFbsBitmap6HandleEv" => {
            p.cpu.regs[0] = p.bitmap_handle_of(r0).unwrap_or(0);
            NativeOutcome::Return
        }
        // TDisplayMode CFbsBitmap::DisplayMode() const
        "_ZNK10CFbsBitmap11DisplayModeEv" => {
            p.cpu.regs[0] = p.bitmap_handle_of(r0)
                .and_then(|id| p.nat.fbs.bitmaps.get(&id))
                .map(|b| b.display_mode)
                .unwrap_or(ECOLOR_16MU);
            NativeOutcome::Return
        }
        // void CFbsBitmap::LockHeap(TBool) const / LockHeapLC(TBool) const
        "_ZNK10CFbsBitmap8LockHeapEi" | "_ZNK10CFbsBitmap10LockHeapLCEi" => NativeOutcome::Return,
        // TUint32* CFbsBitmap::DataAddress() const: lazily map scanlines into guest heap and sync pixels -> guest.
        "_ZNK10CFbsBitmap11DataAddressEv" => {
            let Some(id) = p.bitmap_handle_of(r0) else { p.cpu.regs[0] = 0; return Some(NativeOutcome::Return); };
            let Some((w, h, mode, stride, existing, pixels)) = p.nat.fbs.bitmaps.get(&id).map(|b| {
                (b.width.max(0) as usize, b.height.max(0) as usize, b.display_mode, b.scanline_bytes(), b.data_addr, b.pixels.clone())
            }) else { p.cpu.regs[0] = 0; return Some(NativeOutcome::Return); };
            let total = (stride * h).max(4);
            let addr = match existing {
                Some(a) => a,
                None => match p.alloc_zeroed(total as u32) {
                    Some(a) => {
                        if let Some(b) = p.nat.fbs.bitmaps.get_mut(&id) { b.data_addr = Some(a); }
                        a
                    }
                    None => { p.cpu.regs[0] = 0; return Some(NativeOutcome::Return); }
                },
            };
            let mut raw = vec![0u8; stride * h];
            for y in 0..h {
                let row = y * stride;
                for x in 0..w {
                    let px = pixels[y * w + x];
                    let (r, g, b) = (((px >> 16) & 0xFF) as u8, ((px >> 8) & 0xFF) as u8, (px & 0xFF) as u8);
                    match mode {
                        1 => { if r > 127 { raw[row + x / 8] |= 1 << (x & 7); } }
                        2 => { raw[row + x / 4] |= (r >> 6) << ((x & 3) * 2); }
                        3 | 5 => { raw[row + x / 2] |= (r >> 4) << ((x & 1) * 4); }
                        4 | 6 => { raw[row + x] = r; }
                        7 => {
                            // EColor64K (16-bit RGB565)
                            let rgb565 = (((r as u16) >> 3) << 11) | (((g as u16) >> 2) << 5) | ((b as u16) >> 3);
                            raw[row + x * 2] = rgb565 as u8;
                            raw[row + x * 2 + 1] = (rgb565 >> 8) as u8;
                        }
                        8 => {
                            // EColor16M (24-bit BGR)
                            raw[row + x * 3] = b; raw[row + x * 3 + 1] = g; raw[row + x * 3 + 2] = r;
                        }
                        10 => {
                            // EColor4K (12-bit RGB444 in 16-bit LE)
                            let rgb444 = (((r as u16) >> 4) << 8) | (((g as u16) >> 4) << 4) | ((b as u16) >> 4);
                            raw[row + x * 2] = rgb444 as u8;
                            raw[row + x * 2 + 1] = (rgb444 >> 8) as u8;
                        }
                        _ => {
                            raw[row + x * 4] = b; raw[row + x * 4 + 1] = g; raw[row + x * 4 + 2] = r; raw[row + x * 4 + 3] = 0xFF;
                        }
                    }
                }
            }
            if !raw.is_empty() { let _ = p.mem.vm.write_bytes(addr, &raw); }
            p.cpu.regs[0] = addr;
            NativeOutcome::Return
        }
        // void CFbsBitmap::UnlockHeap(TBool) const: sync any guest-modified DataAddress() scanlines back to host pixels.
        "_ZNK10CFbsBitmap10UnlockHeapEi" => {
            if let Some(id) = p.bitmap_handle_of(r0) {
                if let Some((w, h, mode, stride, Some(addr))) = p.nat.fbs.bitmaps.get(&id).map(|b| {
                    (b.width.max(0) as usize, b.height.max(0) as usize, b.display_mode, b.scanline_bytes(), b.data_addr)
                }) {
                    let mut raw = vec![0u8; stride * h];
                    if !raw.is_empty() && p.mem.vm.read_bytes(addr, &mut raw).is_ok() {
                        if let Some(bmp) = p.nat.fbs.bitmaps.get_mut(&id) {
                            for y in 0..h {
                                let row = y * stride;
                                for x in 0..w {
                                    let px = match mode {
                                        1 => if (raw[row + x / 8] >> (x & 7)) & 1 != 0 { 0x00FF_FFFF } else { 0 },
                                        2 => { let v = (((raw[row + x / 4] >> ((x & 3) * 2)) & 3) as u32) * 85; (v << 16) | (v << 8) | v }
                                        3 | 5 => { let v = (((raw[row + x / 2] >> ((x & 1) * 4)) & 15) as u32) * 17; (v << 16) | (v << 8) | v }
                                        4 | 6 => { let v = u32::from(raw[row + x]); (v << 16) | (v << 8) | v }
                                        7 => {
                                            // EColor64K (16-bit RGB565)
                                            let word = u16::from_le_bytes([raw[row + x * 2], raw[row + x * 2 + 1]]);
                                            let r = (((word >> 11) & 0x1F) as u32 * 255) / 31;
                                            let g = (((word >> 5) & 0x3F) as u32 * 255) / 63;
                                            let b = ((word & 0x1F) as u32 * 255) / 31;
                                            (r << 16) | (g << 8) | b
                                        }
                                        8 => {
                                            // EColor16M (24-bit BGR)
                                            (u32::from(raw[row + x * 3 + 2]) << 16) | (u32::from(raw[row + x * 3 + 1]) << 8) | u32::from(raw[row + x * 3])
                                        }
                                        10 => {
                                            // EColor4K (12-bit RGB444)
                                            let word = u16::from_le_bytes([raw[row + x * 2], raw[row + x * 2 + 1]]);
                                            let r = (((word >> 8) & 0xF) as u32) * 17;
                                            let g = (((word >> 4) & 0xF) as u32) * 17;
                                            let b = ((word & 0xF) as u32) * 17;
                                            (r << 16) | (g << 8) | b
                                        }
                                        _ => (u32::from(raw[row + x * 4 + 2]) << 16) | (u32::from(raw[row + x * 4 + 1]) << 8) | u32::from(raw[row + x * 4]),
                                    };
                                    bmp.pixels[y * w + x] = px;
                                }
                            }
                        }
                    }
                }
            }
            NativeOutcome::Return
        }
        // TSize CFbsBitmap::SizeInPixels() const (hidden return pointer in r0, this in r1)
        "_ZNK10CFbsBitmap12SizeInPixelsEv" => {
            let (w, h) = p.bitmap_handle_of(r1)
                .and_then(|id| p.nat.fbs.bitmaps.get(&id))
                .map(|b| (b.width, b.height))
                .unwrap_or((0, 0));
            let _ = p.wr(r0, w as u32);
            let _ = p.wr(r0 + 4, h as u32);
            NativeOutcome::Return
        }
        // void CFbsBitmap::GetPixel(TRgb& aColor, const TPoint& aPixel) const
        "_ZNK10CFbsBitmap8GetPixelER4TRgbRK6TPoint" => {
            let (x, y) = p.rd_point(r2).unwrap_or((0, 0));
            let px = p.bitmap_handle_of(r0)
                .and_then(|id| p.nat.fbs.bitmaps.get(&id))
                .and_then(|b| b.get(x, y))
                .unwrap_or(0);
            let _ = p.wr(r1, px);
            NativeOutcome::Return
        }
        // CFbsBitmapDevice* CFbsBitmapDevice::NewL(CFbsBitmap* aFbsBitmap)
        "_ZN16CFbsBitmapDevice4NewLEP10CFbsBitmap" => {
            let h = p.bitmap_handle_of(r0).unwrap_or(0);
            match p.make_fbs_device(h, false) {
                Ok(dev) => { p.cpu.regs[0] = dev; NativeOutcome::Return }
                Err(m) => return refuse(m),
            }
        }
        // TInt CFbsDevice::CreateContext(CFbsBitGc*& aGc) / CreateContext(CGraphicsContext*&)
        "_ZN10CFbsDevice13CreateContextERP9CFbsBitGc" | "_ZN10CFbsDevice13CreateContextERP16CGraphicsContext" => {
            let h = match p.obj_get(r0) { Some(HostObj::FbsDevice(d)) => d.bitmap_handle, _ => 0 };
            match p.make_fbs_gc(r0, h) {
                Ok(gc) => {
                    let _ = p.wr(r1, gc);
                    p.cpu.regs[0] = 0;
                    NativeOutcome::Return
                }
                Err(m) => return refuse(m),
            }
        }
        "_ZN10CFbsDeviceD1Ev" | "_ZN10CFbsDeviceD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") { if let Some(heap) = p.heap.as_mut() { let _ = heap.free(r0); } }
            NativeOutcome::Return
        }
        "_ZNK10CFbsDevice11DisplayModeEv" => { p.cpu.regs[0] = ECOLOR_16MU; NativeOutcome::Return }
        "_ZNK10CFbsDevice12SizeInPixelsEv" => {
            let h = match p.obj_get(r1) { Some(HostObj::FbsDevice(d)) => d.bitmap_handle, _ => 0 };
            let (w, h_) = p.nat.fbs.bitmaps.get(&h).map(|b| (b.width, b.height)).unwrap_or_else(|| p.screen_size());
            let _ = p.wr(r0, w as u32);
            let _ = p.wr(r0 + 4, h_ as u32);
            NativeOutcome::Return
        }
        "_ZNK10CFbsDevice11SizeInTwipsEv" => {
            let _ = p.wr(r0, 2235);
            let _ = p.wr(r0 + 4, 3973);
            NativeOutcome::Return
        }
        // void CFbsScreenDevice::Update() / Update(const TRegion&)
        "_ZN16CFbsScreenDevice6UpdateEv" | "_ZN16CFbsScreenDevice6UpdateERK7TRegion" => {
            let h = match p.obj_get(r0) { Some(HostObj::FbsDevice(d)) => Some(d.bitmap_handle), _ => None };
            if let Some(h) = h { let _ = p.present_screen_bitmap(h); }
            NativeOutcome::Return
        }
        "_ZN16CFbsScreenDevice14SetAutoUpdateEi" => NativeOutcome::Return,
        "_ZN10CFbsDevice22GetNearestFontInPixelsERP5CFontRK9TFontSpec"
        | "_ZN10CFbsDevice36GetNearestFontToDesignHeightInPixelsERP5CFontRK9TFontSpec"
        | "_ZN10CFbsDevice33GetNearestFontToMaxHeightInPixelsERP5CFontRK9TFontSpeci" => {
            let height = p.rd(r2 + 56).unwrap_or(12);
            match p.plain_object("CFont", 0x20) {
                Ok(f) => {
                    let _ = p.obj_set(f, HostObj::Font(height * 6));
                    let _ = p.wr(r1, f);
                    p.cpu.regs[0] = 0;
                    NativeOutcome::Return
                }
                Err(m) => return refuse(m),
            }
        }
        "_ZN10CFbsDevice7AddFileERK7TDesC16Ri" => { let _ = p.wr(r2, 1); p.cpu.regs[0] = 0; NativeOutcome::Return }
        "_ZN10CFbsDevice10RemoveFileEi"
        | "_ZN10CFbsDevice12DrawingBeginEi"
        | "_ZN10CFbsDevice10DrawingEndEi"
        | "_ZN10CFbsDevice7SetBitsEv"
        | "_ZNK10CFbsDevice12CancelSpriteEv"
        | "_ZNK10CFbsDevice10ShowSpriteEP11TSpriteBase"
        | "_ZNK10CFbsDevice10ShowSpriteEP11TSpriteBaseRK5TRectPK7TRegion" => NativeOutcome::Return,
        "_ZNK10CFbsDevice10HideSpriteEv" | "_ZNK10CFbsDevice10HideSpriteERK5TRectPK7TRegion" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        "_ZN10CFbsDevice18FontHeightInPixelsEii" => { p.cpu.regs[0] = 12; NativeOutcome::Return }
        "_ZNK10CFbsDevice8GetPixelER4TRgbRK6TPoint" => {
            let h = match p.obj_get(r0) { Some(HostObj::FbsDevice(d)) => d.bitmap_handle, _ => 0 };
            let (x, y) = p.rd_point(r2).unwrap_or((0, 0));
            let px = p.nat.fbs.bitmaps.get(&h).and_then(|b| b.get(x, y)).unwrap_or(0);
            let _ = p.wr(r1, px);
            NativeOutcome::Return
        }
        "_ZNK10CFbsDevice11GetScanLineER5TDes8RK6TPointi12TDisplayMode" => NativeOutcome::Return,

        // CWindowGc& CCoeControl::SystemGc() const / CCoeEnv::SystemGc() const
        "_ZNK11CCoeControl8SystemGcEv" | "_ZNK7CCoeEnv8SystemGcEv" => {
            match p.ensure_system_gc() {
                Ok((_, gc)) => { p.cpu.regs[0] = gc; NativeOutcome::Return }
                Err(m) => return refuse(m),
            }
        }
        // CDirectScreenAccess::NewL(RWsSession&, CWsScreenDevice&, RWindowBase&, MDirectScreenAccess&)
        "_ZN19CDirectScreenAccess4NewLER10RWsSessionR15CWsScreenDeviceR11RWindowBaseR19MDirectScreenAccess" => {
            let Some(dsa) = p.alloc_zeroed(DSA_BYTES) else { return refuse("no heap for CDirectScreenAccess".into()) };
            let (sw, sh) = p.screen_size();
            let screen_bmp_handle = p.nat.fbs.insert_bitmap(FbsBitmap::new(sw, sh, ECOLOR_16MU));
            let Ok(screen_dev_addr) = p.make_fbs_device(screen_bmp_handle, true) else { return refuse("screen dev".into()) };
            let Ok(gc_addr) = p.make_fbs_gc(screen_dev_addr, screen_bmp_handle) else { return refuse("screen gc".into()) };
            // Simple RRegion placeholder (count=1, rect=(0,0,360,640))
            let Some(region_addr) = p.alloc_zeroed(32) else { return refuse("region".into()) };
            let _ = p.wr(dsa + DSA_I_GC, gc_addr);
            let _ = p.wr(dsa + DSA_I_SCREEN_DEVICE, screen_dev_addr);
            let _ = p.wr(dsa + DSA_I_DRAWING_REGION, region_addr);
            let _ = p.obj_set(dsa, HostObj::Dsa(DsaState { screen_bmp_handle, screen_dev_addr, gc_addr, region_addr }));
            p.cpu.regs[0] = dsa;
            NativeOutcome::Return
        }
        "_ZN19CDirectScreenAccess6StartLEv" => NativeOutcome::Return,
        "_ZN10RWsSession5FlushEv" => NativeOutcome::Return,

        // CFbsBitGc methods
        "_ZN9CFbsBitGcD1Ev" | "_ZN9CFbsBitGcD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") { if let Some(heap) = p.heap.as_mut() { let _ = heap.free(r0); } }
            NativeOutcome::Return
        }
        "_ZNK9CFbsBitGc6DeviceEv" => {
            p.cpu.regs[0] = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => g.device_addr, _ => 0 };
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc9SetOriginERK6TPoint" => {
            if let Ok(pt) = p.rd_point(r1) { if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.origin = pt; } }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc11SetDrawModeEN16CGraphicsContext9TDrawModeE" => {
            if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.draw_mode = r1; }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc15SetClippingRectERK5TRect" => {
            if let Ok(rc) = p.rd_rect(r1) { if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.clip_rect = Some(rc); } }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc18CancelClippingRectEv" | "_ZN9CFbsBitGc20CancelClippingRegionEv" => {
            if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.clip_rect = None; }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc17SetClippingRegionEPK7TRegion" | "_ZN9CFbsBitGc17SetClippingRegionERK7TRegion" => {
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc5ResetEv" => {
            if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) {
                let (d, b) = (g.device_addr, g.bitmap_handle);
                *g = FbsGcState::new(d, b);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc7UseFontEPK5CFont"
        | "_ZN9CFbsBitGc11DiscardFontEv"
        | "_ZN9CFbsBitGc17SetUnderlineStyleE14TFontUnderline"
        | "_ZN9CFbsBitGc21SetStrikethroughStyleE18TFontStrikethrough"
        | "_ZN9CFbsBitGc20SetWordJustificationEii"
        | "_ZN9CFbsBitGc20SetCharJustificationEii"
        | "_ZN9CFbsBitGc10SetPenSizeERK5TSize"
        | "_ZN9CFbsBitGc14SetBrushOriginERK6TPoint"
        | "_ZN9CFbsBitGc8SetFadedEi"
        | "_ZN9CFbsBitGc19SetFadingParametersEhh" => NativeOutcome::Return,
        "_ZN9CFbsBitGc11SetPenColorERK4TRgb" => {
            if let Ok(c) = p.rd_rgb(r1) { if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.pen_color = c; } }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc11SetPenStyleEN16CGraphicsContext9TPenStyleE" => {
            if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.pen_style = r1; }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc13SetBrushColorERK4TRgb" => {
            if let Ok(c) = p.rd_rgb(r1) { if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.brush_color = c; } }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc13SetBrushStyleEN16CGraphicsContext11TBrushStyleE" => {
            if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.brush_style = r1; }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc5ClearEv" => {
            let col = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => g.brush_color, _ => 0x00FF_FFFF };
            p.gc_fill_rect(r0, None, col);
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc5ClearERK5TRect" => {
            let col = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => g.brush_color, _ => 0x00FF_FFFF };
            if let Ok(rc) = p.rd_rect(r1) { p.gc_fill_rect(r0, Some(rc), col); }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc11DrawEllipseERK5TRect" => {
            if let Ok(rc) = p.rd_rect(r1) {
                let (dst_h, origin, clip, bs, bc, ps, pc) = match p.obj_get(r0) {
                    Some(HostObj::FbsGc(g)) => (g.bitmap_handle, g.origin, g.clip_rect, g.brush_style, g.brush_color, g.pen_style, g.pen_color),
                    _ => return Some(NativeOutcome::Return),
                };
                if let Some(dst) = p.nat.fbs.bitmaps.get_mut(&dst_h) {
                    let x0 = rc[0] + origin.0;
                    let y0 = rc[1] + origin.1;
                    let x1 = rc[2] + origin.0;
                    let y1 = rc[3] + origin.1;
                    let w = (x1 - x0) as f32;
                    let h = (y1 - y0) as f32;
                    if w > 0.0 && h > 0.0 {
                        let rx = w * 0.5;
                        let ry = h * 0.5;
                        let cx = x0 as f32 + rx;
                        let cy = y0 as f32 + ry;
                        let (mut bx0, mut by0, mut bx1, mut by1) = (x0.max(0), y0.max(0), x1.min(dst.width), y1.min(dst.height));
                        if let Some(c) = clip {
                            bx0 = bx0.max(c[0]); by0 = by0.max(c[1]); bx1 = bx1.min(c[2]); by1 = by1.min(c[3]);
                        }
                        for py in by0..by1 {
                            let dy = (py as f32 + 0.5 - cy) / ry;
                            for px in bx0..bx1 {
                                let dx = (px as f32 + 0.5 - cx) / rx;
                                let d2 = dx * dx + dy * dy;
                                if d2 <= 1.0 {
                                    let border = ps != 0 && d2 >= 0.72;
                                    if border {
                                        dst.set(px, py, pc);
                                    } else if bs != 0 {
                                        dst.set(px, py, bc);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc8DrawRectERK5TRect" | "_ZN9CFbsBitGc13DrawRoundRectERK5TRectRK5TSize" => {
            if let Ok(rc) = p.rd_rect(r1) {
                let (bs, bc, ps, pc) = match p.obj_get(r0) {
                    Some(HostObj::FbsGc(g)) => (g.brush_style, g.brush_color, g.pen_style, g.pen_color),
                    _ => (0, 0, 0, 0),
                };
                if bs != 0 { p.gc_fill_rect(r0, Some(rc), bc); }
                if ps != 0 {
                    p.gc_fill_rect(r0, Some([rc[0], rc[1], rc[2], rc[1] + 1]), pc);
                    p.gc_fill_rect(r0, Some([rc[0], rc[3] - 1, rc[2], rc[3]]), pc);
                    p.gc_fill_rect(r0, Some([rc[0], rc[1], rc[0] + 1, rc[3]]), pc);
                    p.gc_fill_rect(r0, Some([rc[2] - 1, rc[1], rc[2], rc[3]]), pc);
                }
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc6MoveToERK6TPoint" => {
            if let Ok(pt) = p.rd_point(r1) { if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.pos = pt; } }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc6MoveByERK6TPoint" => {
            if let Ok(pt) = p.rd_point(r1) { if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.pos.0 += pt.0; g.pos.1 += pt.1; } }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc4PlotERK6TPoint" => {
            if let Ok((x, y)) = p.rd_point(r1) {
                let pc = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => g.pen_color, _ => 0 };
                p.gc_fill_rect(r0, Some([x, y, x + 1, y + 1]), pc);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc8DrawLineERK6TPointS2_" => {
            if let (Ok((x0, y0)), Ok((x1, y1))) = (p.rd_point(r1), p.rd_point(r2)) {
                let pc = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => g.pen_color, _ => 0 };
                let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
                for i in 0..=steps {
                    let x = x0 + (x1 - x0) * i / steps;
                    let y = y0 + (y1 - y0) * i / steps;
                    p.gc_fill_rect(r0, Some([x, y, x + 1, y + 1]), pc);
                }
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc10DrawLineToERK6TPoint" | "_ZN9CFbsBitGc10DrawLineByERK6TPoint" => {
            if let Ok(pt) = p.rd_point(r1) {
                let (pos, pc) = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => (g.pos, g.pen_color), _ => ((0, 0), 0) };
                let (x1, y1) = if name.ends_with("ByERK6TPoint") { (pos.0 + pt.0, pos.1 + pt.1) } else { pt };
                let steps = (x1 - pos.0).abs().max((y1 - pos.1).abs()).max(1);
                for i in 0..=steps {
                    let x = pos.0 + (x1 - pos.0) * i / steps;
                    let y = pos.1 + (y1 - pos.1) * i / steps;
                    p.gc_fill_rect(r0, Some([x, y, x + 1, y + 1]), pc);
                }
                if let Some(HostObj::FbsGc(g)) = p.obj_get(r0) { g.pos = (x1, y1); }
            }
            NativeOutcome::Return
        }
        // BitBlt(const TPoint& aPoint, const CFbsBitmap* aBitmap)
        "_ZN9CFbsBitGc6BitBltERK6TPointPK10CFbsBitmap"
        | "_ZN9CFbsBitGc10DrawBitmapERK6TPointPK10CFbsBitmap" => {
            if let Ok(pt) = p.rd_point(r1) {
                let _ = p.gc_blit(r0, pt, r2, None, None, false);
            }
            NativeOutcome::Return
        }
        // BitBlt(const TPoint& aPoint, const CFbsBitmap* aBitmap, const TRect& aRect)
        "_ZN9CFbsBitGc6BitBltERK6TPointPK10CFbsBitmapRK5TRect" => {
            if let (Ok(pt), Ok(sr)) = (p.rd_point(r1), p.rd_rect(r3)) {
                let _ = p.gc_blit(r0, pt, r2, Some(sr), None, false);
            }
            NativeOutcome::Return
        }
        // BitBltMasked(const TPoint& aPoint, const CFbsBitmap* aBitmap, const TRect& aSourceRect, const CFbsBitmap* aMaskBitmap, TBool aInvertMask)
        "_ZN9CFbsBitGc12BitBltMaskedERK6TPointPK10CFbsBitmapRK5TRectS5_i" => {
            let sp = p.cpu.regs[13];
            let mask_bmp = p.rd(sp).unwrap_or(0);
            let invert = p.rd(sp + 4).unwrap_or(0) != 0;
            if let (Ok(pt), Ok(sr)) = (p.rd_point(r1), p.rd_rect(r3)) {
                let _ = p.gc_blit(r0, pt, r2, Some(sr), Some(mask_bmp), invert);
            }
            NativeOutcome::Return
        }
        // DrawBitmap(const TRect& aDestRect, const CFbsBitmap* aSource[, const TRect& aSourceRect])
        "_ZN9CFbsBitGc10DrawBitmapERK5TRectPK10CFbsBitmap" => {
            if let Ok(dr) = p.rd_rect(r1) {
                let _ = p.gc_draw_bitmap_scaled(r0, dr, r2, None, None, false);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc10DrawBitmapERK5TRectPK10CFbsBitmapS2_" => {
            if let (Ok(dr), Ok(sr)) = (p.rd_rect(r1), p.rd_rect(r3)) {
                let _ = p.gc_draw_bitmap_scaled(r0, dr, r2, Some(sr), None, false);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc16DrawBitmapMaskedERK5TRectPK10CFbsBitmapS2_S5_i" => {
            let sp = p.cpu.regs[13];
            let mask_bmp = p.rd(sp).unwrap_or(0);
            let invert = p.rd(sp + 4).unwrap_or(0) != 0;
            if let (Ok(dr), Ok(sr)) = (p.rd_rect(r1), p.rd_rect(r3)) {
                let _ = p.gc_draw_bitmap_scaled(r0, dr, r2, Some(sr), Some(mask_bmp), invert);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc8CopyRectERK6TPointRK5TRect" => {
            if let (Ok((dx, dy)), Ok(rc)) = (p.rd_point(r1), p.rd_rect(r2)) {
                let dst_h = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => g.bitmap_handle, _ => 0 };
                if let Some(snap) = p.nat.fbs.bitmaps.get(&dst_h).cloned() {
                    if let Some(dst) = p.nat.fbs.bitmaps.get_mut(&dst_h) {
                        for y in rc[1]..rc[3] {
                            for x in rc[0]..rc[2] {
                                if let Some(px) = snap.get(x, y) {
                                    dst.set(x + dx, y + dy, px);
                                }
                            }
                        }
                    }
                }
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc8DrawTextERK7TDesC16RK6TPoint" => {
            if let (Ok(units), Ok((x, y))) = (crate::desc::read16(p, r1), p.rd_point(r2)) {
                let s = crate::desc::to_utf8(&units);
                p.gc_draw_text(r0, &s, x, y);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc8DrawTextERK7TDesC16RK5TRectiN16CGraphicsContext10TTextAlignEi" => {
            let sp = p.cpu.regs[13];
            let align = p.rd(sp).unwrap_or(0);
            let margin = p.rd(sp + 4).unwrap_or(0) as i32;
            let baseline = r3 as i32;
            if let (Ok(units), Ok(rc)) = (crate::desc::read16(p, r1), p.rd_rect(r2)) {
                let s = crate::desc::to_utf8(&units);
                let (bs, bc) = match p.obj_get(r0) { Some(HostObj::FbsGc(g)) => (g.brush_style, g.brush_color), _ => (0, 0) };
                if bs != 0 { p.gc_fill_rect(r0, Some(rc), bc); }
                let text_w = s.chars().count() as i32 * 6;
                let tx = match align {
                    1 => rc[0] + ((rc[2] - rc[0]) - text_w) / 2, // ECenter
                    2 => rc[2] - text_w - margin,                // ERight
                    _ => rc[0] + margin,                         // ELeft
                };
                p.gc_draw_text(r0, &s, tx, rc[1] + baseline);
            }
            NativeOutcome::Return
        }
        "_ZN9CFbsBitGc17AlphaBlendBitmapsERK6TPointPK10CFbsBitmapRK5TRectS5_S2_" => {
            let sp = p.cpu.regs[13];
            let alpha_bmp = p.rd(sp).unwrap_or(0);
            if let (Ok(pt), Ok(sr)) = (p.rd_point(r1), p.rd_rect(r3)) {
                let _ = p.gc_blit(r0, pt, r2, Some(sr), Some(alpha_bmp), true);
            }
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        _ => return None,
    })
}
