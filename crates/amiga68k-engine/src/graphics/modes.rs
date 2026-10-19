//! graphics.library display modes: the one mode the engine's single screen has, 0x29000 (PAL_MONITOR_ID | HIRES_KEY: the
//! "Workbench mode" the AROS m68k oracle reports for DOpus's screen, tests/oracle/dopus5/trace/1-startup.txt:1347 GetVPModeID ->
//! 0x29000, GetDisplayInfoData(DTAG_DISP, 0x38 bytes) -> 0x38).
//!
//! GetVPModeID(a0 = ViewPort) -> 0x29000 (INVALID_ID, -1, for NULL). GetDisplayInfoData(a0 = handle or NULL, a1 = buffer,
//! d0 = size, d1 = tagID, d2 = displayID) copies up to `size` bytes of the info record and returns the count, 0 for a mode the
//! engine does not have. DTAG_DISP (0x80000000), DTAG_DIMS (0x80001000) and DTAG_NAME (0x80003000) are filled; the other record kinds
//! (MNTR, VEC) are named refusals until a program asks (a MonitorInfo points at a MonitorSpec the engine does not model). struct DisplayInfo is 56 bytes (NDK graphics/displayinfo.h): QueryHeader{StructID 0, DisplayID 4,
//! SkipID 8, Length 12}, NotAvailable 16 (UWORD), PropertyFlags 18, Resolution 22 (x, y), PixelSpeed 26, NumStdSprites 28,
//! PaletteRange 30, SpriteResolution 32, RedBits 40, GreenBits 41, BlueBits 42. The timing fields (resolution 22x44, pixel
//! speed 70 ns, 8 sprites) are those of a PAL high-resolution mode, representative and NOT verified against the oracle (the trace logs
//! arguments, not the buffer); the colour fields are the screen's truth: 8 bits per gun, a 256-pen palette.
use alloc::format;
use alloc::string::String;
use super::layout::{SCREEN_DEPTH, SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const INVALID_MODE: u32 = 0xFFFF_FFFF;
pub const WORKBENCH_MODE: u32 = 0x29000;
const INVALID_ID: u32 = 0xFFFF_FFFF;
const DTAG_DISP: u32 = 0x8000_0000;
const DTAG_DIMS: u32 = 0x8000_1000;
const DTAG_NAME: u32 = 0x8000_3000;
const DIMENSIONINFO_SIZE: usize = 88;
const NAMEINFO_SIZE: usize = 56;
const DISPLAYNAMELEN: usize = 32;
/// DIPF_IS_WB: the mode is the Workbench screen's.
pub const DIPF_IS_WB: u32 = 1 << 8;
const DISPLAYINFO_SIZE: usize = 56;

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("InitTmpRas", init_tmp_ras::<U>).named("GetVPModeID", get_vp_mode_id::<U>).named("NextDisplayInfo", next_display_info::<U>).named("FindDisplayInfo", find_display_info::<U>).named("GetDisplayInfoData", get_display_info_data::<U>).named("BestModeIDA", super::modes_best::best_mode_ida::<U>)
}

fn get_vp_mode_id<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let vp = cpu.areg(0);
    let id = if vp == 0 { INVALID_ID } else { WORKBENCH_MODE };
    cpu.set_d0(id);
    Ok(format!("[graphics] GetVPModeID {vp:#x} -> {id:#x}"))
}

/// NextDisplayInfo(d0 = last ID or INVALID_ID to begin) -> the next mode ID of the display database, INVALID_ID at the end. The database is the
/// one mode the engine's single screen has ([`WORKBENCH_MODE`]): the first call answers it, the call after it answers INVALID_ID; an ID that is
/// not in the database ends the iteration too (graphics/nextdisplayinfo.c walks its list from the entry it finds for `last_ID`).
fn next_display_info<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let last = cpu.dreg(0);
    let next = if last == INVALID_ID { WORKBENCH_MODE } else { INVALID_ID };
    cpu.set_d0(next);
    Ok(format!("[graphics] NextDisplayInfo {last:#x} -> {next:#x}"))
}

/// FindDisplayInfo(d0 = ID) -> a DisplayInfoHandle, NULL when the database has no such mode. A handle is opaque to programs; here it is the
/// mode ID itself (non-zero), which `GetDisplayInfoData` accepts back.
fn find_display_info<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let id = cpu.dreg(0);
    let h = if id == WORKBENCH_MODE { id } else { 0 };
    cpu.set_d0(h);
    Ok(format!("[graphics] FindDisplayInfo {id:#x} -> {h:#x}"))
}

/// The one display mode's record of kind `tag`, or None for a kind not modelled. Honest about CoralOS's one screen (graphics/layout.rs:
/// SCREEN_WIDTH x SCREEN_HEIGHT, SCREEN_DEPTH planes, no overscan, 8 bits per colour gun because the host palette is 8-bit RGB).
/// Layouts: NDK graphics/displayinfo.h with m68k -mno-align-int: DimensionInfo = QueryHeader 16, MaxDepth 16, MinRasterWidth 18,
/// MinRasterHeight 20, MaxRasterWidth 22, MaxRasterHeight 24, Nominal 26, MaxOScan 34, VideoOScan 42, TxtOScan 50, StdOScan 58 (each a
/// Rectangle of four WORDs MinX MinY MaxX MaxY), pad 66, reserved 80 = 88 bytes; NameInfo = QueryHeader 16, Name[32] 16, reserved 48
/// = 56 bytes (sizes and offsets checked with m68k-linux-gnu-gcc -m68020 -mno-align-int).
pub(super) fn record(tag: u32, id: u32) -> Option<alloc::vec::Vec<u8>> {
    let size = match tag { DTAG_DISP => DISPLAYINFO_SIZE, DTAG_DIMS => DIMENSIONINFO_SIZE, DTAG_NAME => NAMEINFO_SIZE, _ => return None };
    let mut rec = alloc::vec![0u8; size];
    let (w, h) = (SCREEN_WIDTH as u16, SCREEN_HEIGHT as u16);
    rec[0..4].copy_from_slice(&tag.to_be_bytes()); rec[4..8].copy_from_slice(&id.to_be_bytes());
    rec[8..12].copy_from_slice(&INVALID_ID.to_be_bytes()); rec[12..16].copy_from_slice(&((size / 4) as u32).to_be_bytes());
    let put16 = |rec: &mut [u8], o: usize, v: u16| rec[o..o + 2].copy_from_slice(&v.to_be_bytes());
    match tag {
        DTAG_DISP => {
            rec[18..22].copy_from_slice(&DIPF_IS_WB.to_be_bytes());
            for (o, v) in [(22usize, 22u16), (24, 44), (26, 70), (28, 8), (30, 256), (32, 140), (34, 280)] { put16(&mut rec, o, v); }
            rec[40] = 8; rec[41] = 8; rec[42] = 8;
        }
        DTAG_DIMS => {
            for (o, v) in [(16usize, SCREEN_DEPTH as u16), (18, w), (20, h), (22, w), (24, h)] { put16(&mut rec, o, v); }
            for r in 0..5 { for (k, v) in [0, 0, w - 1, h - 1].into_iter().enumerate() { put16(&mut rec, 26 + 8 * r + 2 * k, v); } }
        }
        _ => {
            let name = alloc::format!("CoralOS {w}x{h}");
            rec[16..16 + name.len().min(DISPLAYNAMELEN - 1)].copy_from_slice(&name.as_bytes()[..name.len().min(DISPLAYNAMELEN - 1)]);
        }
    }
    Some(rec)
}

fn get_display_info_data<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (handle, buf, size, tag) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as usize, cpu.dreg(1));
    let id = if handle != 0 { handle } else { cpu.dreg(2) }; // a handle (FindDisplayInfo) names the mode; without one the ID does
    let Some(rec) = record(tag, id) else { return Err(Refusal(format!("GetDisplayInfoData: record kind {tag:#x} is not modelled (only DTAG_DISP, DTAG_DIMS, DTAG_NAME)"))) };
    if id != WORKBENCH_MODE { cpu.set_d0(0); return Ok(format!("[graphics] GetDisplayInfoData mode {id:#x}: no such mode -> 0")); }
    let n = size.min(rec.len());
    a.mem.write_bytes(buf, &rec[..n]).map_err(Refusal::mem("DisplayInfo buffer"))?;
    cpu.set_d0(n as u32);
    Ok(format!("[graphics] GetDisplayInfoData {tag:#x} mode {id:#x} -> {n} bytes"))
}

/// InitTmpRas(a0 = TmpRas, a1 = buffer, d0 = size): struct TmpRas { BYTE *RasPtr; LONG Size; } (NDK graphics/gfx.h). The engine
/// draws without a scratch raster (areas and flood fills are computed host-side), so the buffer is only recorded.
pub fn init_tmp_ras<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tr, buf, size) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    a.mem.write_u32(tr, buf).map_err(Refusal::mem("TmpRas.RasPtr"))?;
    a.mem.write_u32(tr + 4, size).map_err(Refusal::mem("TmpRas.Size"))?;
    cpu.set_d0(tr);
    Ok(format!("[graphics] InitTmpRas {tr:#x} buffer {buf:#x} size {size}"))
}
