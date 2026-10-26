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
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_68k::Cpu;

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
    for (i, b) in b"guteform.font\0".iter().enumerate() { mem.write_u8(FONT_NAME + i as u32, *b).map_err(m("font name"))?; }
    for (i, b) in b"Workbench Screen\0".iter().enumerate() { mem.write_u8(TITLE + i as u32, *b).map_err(m("screen title"))?; }
    mem.write_u32(TEXT_ATTR, FONT_NAME).map_err(m("ta_Name"))?;
    mem.write_u16(TEXT_ATTR + 4, heirloom_theme::uifont::Y_SIZE).map_err(m("ta_YSize"))?;
    mem.write_u32(SCREEN + 22, TITLE).map_err(m("Screen.Title"))?;
    mem.write_u32(SCREEN + 26, TITLE).map_err(m("Screen.DefaultTitle"))?;
    mem.write_u32(SCREEN + 40, TEXT_ATTR).map_err(m("Screen.Font"))?;
    // IntuitionBase.ActiveScreen / FirstScreen (readable between LockIBase and UnlockIBase): the one screen. Offsets: Library 34 + View 18 +
    // ActiveWindow = 52 (m68k, -mno-align-int, intuition/intuitionbase.h). Programs seed their preferences from ActiveScreen (Jano's Editor).
    mem.write_u32(crate::machine::INTUITION_BASE + 56, SCREEN).map_err(m("IntuitionBase.ActiveScreen"))?;
    mem.write_u32(crate::machine::INTUITION_BASE + 60, SCREEN).map_err(m("IntuitionBase.FirstScreen"))?;
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

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("LockPubScreenList", lock_pub_screen_list::<U>).named("UnlockPubScreenList", unlock_pub_screen_list::<U>).named("LockIBase", lock_ibase::<U>).named("UnlockIBase", unlock_ibase::<U>).named("ScreenToFront", |a, c| screen_depth(a, c, "ScreenToFront")).named("ScreenToBack", |a, c| screen_depth(a, c, "ScreenToBack")).named("QueryOverscan", query_overscan::<U>)
}

/// QueryOverscan(a0 = displayid, a1 = Rectangle*, d0.w = oscantype) -> TRUE when the mode exists and the type is known
/// (rom/intuition/queryoverscan.c:73-105): the rectangle is the matching one of the mode's DimensionInfo (DTAG_DIMS), OSCAN_TEXT 1 ->
/// TxtOScan, STANDARD 2 -> StdOScan, MAX 3 -> MaxOScan, VIDEO 4 -> VideoOScan. An unknown mode or type answers FALSE and leaves the
/// rectangle alone. DimensionInfo offsets: MaxOScan 34, VideoOScan 42, TxtOScan 50, StdOScan 58 (graphics/modes.rs).
fn query_overscan<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (id, rect, ty) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as u16);
    let Some(dims) = crate::graphics::modes::record(crate::graphics::modes::DTAG_DIMS, id).filter(|_| id == crate::graphics::modes::WORKBENCH_MODE) else {
        cpu.set_d0(0);
        return Ok(format!("[intuition] QueryOverscan mode {id:#x}: no such mode -> FALSE"));
    };
    let off = match ty { 1 => 50, 2 => 58, 3 => 34, 4 => 42, _ => { cpu.set_d0(0); return Ok(format!("[intuition] QueryOverscan type {ty}: not OSCAN_TEXT/STANDARD/MAX/VIDEO -> FALSE")); } };
    a.mem.write_bytes(rect, &dims[off..off + 8]).map_err(Refusal::mem("QueryOverscan rectangle"))?;
    cpu.set_d0(1);
    Ok(format!("[intuition] QueryOverscan mode {id:#x} type {ty} -> TRUE"))
}

/// ScreenToFront / ScreenToBack(a0 = screen): there is one screen, so its depth order cannot change; the call is accepted for
/// that screen and a named refusal for any other pointer.
fn screen_depth<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu, name: &str) -> Result<String, Refusal> {
    let s = cpu.areg(0);
    if s != SCREEN { return Err(Refusal(format!("{name} {s:#x}: not the screen (there is one screen)"))); }
    Ok(format!("[intuition] {name} (the only screen: nothing to reorder)"))
}

/// LockIBase(d0 = dontknow) -> a lock token for UnlockIBase. Tasks switch only after a library call, so the lock never has to
/// exclude anybody; it is counted so an UnlockIBase without a LockIBase is a named refusal.
fn lock_ibase<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    a.ibase_locks += 1;
    cpu.set_d0(a.ibase_locks);
    Ok(format!("[intuition] LockIBase -> token {}", a.ibase_locks))
}

fn unlock_ibase<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let token = cpu.areg(0);
    a.ibase_locks = a.ibase_locks.checked_sub(1).ok_or_else(|| Refusal("UnlockIBase without LockIBase".into()))?;
    Ok(format!("[intuition] UnlockIBase {token:#x}"))
}

/// LockPubScreenList() -> the list of public screens: a real List with the one PubScreenNode, "Workbench" (struct PubScreenNode,
/// NDK intuition/screens.h: Node 14 bytes, psn_Screen 14, psn_Flags 18, psn_Size 20, psn_VisitorCount 22, psn_SigTask 24,
/// psn_SigBit 28; 30 bytes) with psn_Flags 0, psn_VisitorCount = the current LockPubScreen count at
/// each call. Built on first use in the engine's arena.
fn lock_pub_screen_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if a.pub_list == 0 {
        let m = Refusal::mem("PubScreenList");
        let blk = crate::fs::alloc_zeroed_sys(a, 14 + 30 + 16)?.ok_or_else(|| Refusal("LockPubScreenList: no memory (NOMEM)".into()))?;
        let (list, node, name) = (blk, blk + 14, blk + 44);
        a.mem.write_bytes(name, b"Workbench\0").map_err(&m)?;
        crate::execlists::new_list(&mut a.mem, list, 0)?;
        a.mem.write_u8(node + 8, 2).map_err(&m)?; // ln_Type NT_USER-ish: a plain node
        a.mem.write_u32(node + 10, name).map_err(&m)?;
        a.mem.write_u32(node + 14, SCREEN).map_err(&m)?;
        crate::execlists::add_tail(&mut a.mem, list, node)?;
        a.pub_list = list;
    }
    a.mem.write_u16(a.pub_list + 14 + 22, a.screen_locks as u16).map_err(Refusal::mem("psn_VisitorCount"))?;
    cpu.set_d0(a.pub_list);
    Ok(format!("[intuition] LockPubScreenList -> {:#x}", a.pub_list))
}

fn unlock_pub_screen_list<U: GuestUi>(_: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> { Ok("[intuition] UnlockPubScreenList".into()) }
