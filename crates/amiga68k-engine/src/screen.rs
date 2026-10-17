//! intuition.library public screens: LockPubScreen / UnlockPubScreen over ONE guest `struct Screen`, the "Workbench
//! screen" a CoralOS guest sees. Programs read its geometry, bar height, border sizes and font to lay out their own
//! windows, so these fields are real; the rest of the 346 bytes (ViewPort, RastPort, LayerInfo) is zero until a
//! program that reads them shows up in the census.
//!
//! Values: the AROS m68k oracle's Workbench screen (tests/oracle/screen.bin). They are the guest's view only; the
//! native window the guest's OpenWindow becomes is sized by CoralOS (guestui), not by these numbers.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

pub const LVO_LOCK_PUB_SCREEN: i32 = -510;
pub const LVO_UNLOCK_PUB_SCREEN: i32 = -516;

pub const SCREEN: u32 = 0x6400;
const SCREEN_SIZE: u32 = 346;
const TEXT_ATTR: u32 = 0x6560;
const FONT_NAME: u32 = 0x6570;
const TITLE: u32 = 0x6590;

/// Lay out the Workbench screen (offsets: AROS intuition/screens.h, m68k ABI).
pub fn setup(mem: &mut GuestMem) -> Result<(), Refusal> {
    let m = Refusal::mem;
    for a in SCREEN..SCREEN + SCREEN_SIZE { mem.write_u8(a, 0).map_err(m("Screen"))?; }
    mem.write_u16(SCREEN + 12, 640).map_err(m("Screen.Width"))?;
    mem.write_u16(SCREEN + 14, 256).map_err(m("Screen.Height"))?;
    for (off, v) in [(30, 14u8), (31, 1), (32, 5), (33, 2), (34, 4), (35, 6), (36, 4), (37, 4), (38, 4)] {
        mem.write_u8(SCREEN + off, v).map_err(m("Screen bar/border sizes"))?;
    }
    for (i, b) in b"topaz.font\0".iter().enumerate() { mem.write_u8(FONT_NAME + i as u32, *b).map_err(m("font name"))?; }
    for (i, b) in b"Workbench Screen\0".iter().enumerate() { mem.write_u8(TITLE + i as u32, *b).map_err(m("screen title"))?; }
    mem.write_u32(TEXT_ATTR, FONT_NAME).map_err(m("ta_Name"))?;
    mem.write_u16(TEXT_ATTR + 4, 8).map_err(m("ta_YSize"))?;
    mem.write_u32(SCREEN + 22, TITLE).map_err(m("Screen.Title"))?;
    mem.write_u32(SCREEN + 26, TITLE).map_err(m("Screen.DefaultTitle"))?;
    mem.write_u32(SCREEN + 40, TEXT_ATTR).map_err(m("Screen.Font"))?;
    Ok(())
}

/// LockPubScreen(a0 = name or NULL) -> the screen. NULL and "Workbench" are the one public screen; any other name is
/// a legitimate NULL (no such public screen), as on AmigaOS.
pub fn lock_pub_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    let name = if p == 0 { None } else { Some(a.mem.read_cstr(p).map_err(Refusal::mem("LockPubScreen.name"))?) };
    let s = match name.as_deref() { None | Some("Workbench") => SCREEN, Some(_) => 0 };
    if s != 0 { a.screen_locks += 1; }
    cpu.set_d0(s);
    Ok(format!("[intuition] LockPubScreen {:?} -> {s:#x}", name.unwrap_or_else(|| "NULL".into())))
}

/// UnlockPubScreen(a0 = name, a1 = screen).
pub fn unlock_pub_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = cpu.areg(1);
    if s == 0 { return Ok("[intuition] UnlockPubScreen NULL".into()); }
    if s != SCREEN { return Err(Refusal(format!("UnlockPubScreen {s:#x}: not the public screen"))); }
    a.screen_locks = a.screen_locks.checked_sub(1).ok_or_else(|| Refusal("UnlockPubScreen without LockPubScreen".into()))?;
    Ok(format!("[intuition] UnlockPubScreen (locks {})", a.screen_locks))
}
