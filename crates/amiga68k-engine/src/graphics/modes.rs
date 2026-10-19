//! graphics.library display modes: the one mode the engine's single screen has, 0x29000 (PAL_MONITOR_ID | HIRES_KEY: the
//! "Workbench mode" the AROS m68k oracle reports for DOpus's screen, tests/oracle/dopus5/trace/1-startup.txt:1347 GetVPModeID ->
//! 0x29000, GetDisplayInfoData(DTAG_DISP, 0x38 bytes) -> 0x38).
//!
//! GetVPModeID(a0 = ViewPort) -> 0x29000 (INVALID_ID, -1, for NULL). GetDisplayInfoData(a0 = handle or NULL, a1 = buffer,
//! d0 = size, d1 = tagID, d2 = displayID) copies up to `size` bytes of the info record and returns the count, 0 for a mode the
//! engine does not have. Only DTAG_DISP (0x80000000) is filled; the other record kinds (DIMS, MNTR, NAME, VEC) are named
//! refusals until a program asks. struct DisplayInfo is 56 bytes (NDK graphics/displayinfo.h): QueryHeader{StructID 0, DisplayID 4,
//! SkipID 8, Length 12}, NotAvailable 16 (UWORD), PropertyFlags 18, Resolution 22 (x, y), PixelSpeed 26, NumStdSprites 28,
//! PaletteRange 30, SpriteResolution 32, RedBits 40, GreenBits 41, BlueBits 42. The values are those of a PAL non-interlaced
//! high-resolution mode (resolution 22x44, pixel speed 70 ns, 8 sprites, 4 bits per gun, 4096 colours); they are
//! representative, NOT verified against the oracle (the trace logs arguments, not the buffer).
use alloc::format;
use alloc::string::String;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const WORKBENCH_MODE: u32 = 0x29000;
const INVALID_ID: u32 = 0xFFFF_FFFF;
const DTAG_DISP: u32 = 0x8000_0000;
const DISPLAYINFO_SIZE: usize = 56;

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("InitTmpRas", init_tmp_ras::<U>).named("GetVPModeID", get_vp_mode_id::<U>).named("NextDisplayInfo", next_display_info::<U>).named("FindDisplayInfo", find_display_info::<U>).named("GetDisplayInfoData", get_display_info_data::<U>)
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

fn get_display_info_data<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (handle, buf, size, tag) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as usize, cpu.dreg(1));
    let id = if handle != 0 { handle } else { cpu.dreg(2) }; // a handle (FindDisplayInfo) names the mode; without one the ID does
    if tag != DTAG_DISP { return Err(Refusal(format!("GetDisplayInfoData: record kind {tag:#x} is not modelled (only DTAG_DISP)"))); }
    if id != WORKBENCH_MODE { cpu.set_d0(0); return Ok(format!("[graphics] GetDisplayInfoData mode {id:#x}: no such mode -> 0")); }
    let mut rec = [0u8; DISPLAYINFO_SIZE];
    let mut put32 = |o: usize, v: u32| rec[o..o + 4].copy_from_slice(&v.to_be_bytes());
    put32(0, DTAG_DISP); put32(4, id); put32(8, INVALID_ID); put32(12, (DISPLAYINFO_SIZE / 4) as u32);
    for (o, v) in [(22usize, 22u16), (24, 44), (26, 70), (28, 8), (30, 4096), (32, 140), (34, 280)] { rec[o..o + 2].copy_from_slice(&v.to_be_bytes()); }
    rec[40] = 4; rec[41] = 4; rec[42] = 4;
    let n = size.min(DISPLAYINFO_SIZE);
    a.mem.write_bytes(buf, &rec[..n]).map_err(Refusal::mem("DisplayInfo buffer"))?;
    cpu.set_d0(n as u32);
    Ok(format!("[graphics] GetDisplayInfoData DTAG_DISP mode {id:#x} -> {n} bytes"))
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
