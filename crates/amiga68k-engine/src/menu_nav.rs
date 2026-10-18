//! Menu numbers and the menu strip's live state: ItemAddress, OnMenu / OffMenu, and the MENUPICK a native menu command
//! becomes.
//!
//! A menu number packs three fields (intuition/intuition.h): MENUNUM = bits 0-4, ITEMNUM = bits 5-10, SUBNUM = bits
//! 11-15; NOMENU 0x1F, NOITEM 0x3F, NOSUB 0x1F and MENUNULL 0xFFFF mean "none". The strip is the GUEST's own struct
//! chain (a window's `menu_strips` entry); everything here walks it with checked reads, bounded, so a cyclic or wild
//! chain is a named refusal, never a hang. Offsets: Menu.NextMenu 0, Flags 12, FirstItem 18; MenuItem.NextItem 0, Flags
//! 12, MutualExclude 14, SubItem 28, NextSelect 32 (m68k gcc -mno-align-int over the AROS field lists).
use alloc::string::String;
use alloc::format;
use crate::{idcmp::{self, IDCMP_MENUPICK, MENUNULL}, machine::Amiga, menu, run::Delivered, Refusal};
use guestui_abi::{CallbackToken, GuestUi};
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

pub const LVO_ITEM_ADDRESS: i32 = -144;
pub const LVO_OFF_MENU: i32 = -180;
pub const LVO_ON_MENU: i32 = -192;
pub const LVO_RESET_MENU_STRIP: i32 = -702;

const MN_FLAGS: u32 = 12;
const MN_FIRST_ITEM: u32 = 18;
const MI_FLAGS: u32 = 12;
const MI_EXCLUDE: u32 = 14;
const MI_SUB_ITEM: u32 = 28;
const MI_NEXT_SELECT: u32 = 32;
const MENUENABLED: u16 = 0x0001;
const CHECKIT: u16 = 0x0001;
const MENUTOGGLE: u16 = 0x0008;
const ITEMENABLED: u16 = 0x0010;
const CHECKED: u16 = 0x0100;
const NOMENU: u16 = 0x1F;
const NOITEM: u16 = 0x3F;
const NOSUB: u16 = 0x1F;

/// What a menu number names in a strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target { Menu(u32), Item(u32), Sub(u32) }

/// A resolved menu number: the target, the Menu it is under, its parent item (subs), and the head of the list it is in.
#[derive(Debug, Clone, Copy)]
pub struct Resolved { pub target: Target, pub menu: u32, pub parent: Option<u32>, pub list_head: u32 }

impl Resolved {
    pub fn address(&self) -> u32 { match self.target { Target::Menu(p) | Target::Item(p) | Target::Sub(p) => p } }
    /// The flags word and the bit that means "enabled" for this target.
    fn enable_bit(&self) -> (u32, u16) {
        match self.target { Target::Menu(p) => (p + MN_FLAGS, MENUENABLED), Target::Item(p) | Target::Sub(p) => (p + MI_FLAGS, ITEMENABLED) }
    }
}

/// The `n`th node (0-based) of the NextXxx-at-offset-0 chain starting at `head`.
fn nth(mem: &GuestMem, head: u32, n: u16) -> Result<Option<u32>, Refusal> {
    let mut at = head;
    for _ in 0..n {
        if at == 0 { return Ok(None); }
        at = mem.read_u32(at).map_err(Refusal::mem("menu chain"))?;
    }
    Ok(if at == 0 { None } else { Some(at) })
}

/// Resolve `code` in `strip`; None when it names nothing the strip has (ItemAddress returns NULL then).
pub fn resolve(mem: &GuestMem, strip: u32, code: u16) -> Result<Option<Resolved>, Refusal> {
    let (m, i, s) = (code & 0x1F, (code >> 5) & 0x3F, (code >> 11) & 0x1F);
    if code == MENUNULL || m == NOMENU { return Ok(None); }
    let Some(menu) = nth(mem, strip, m)? else { return Ok(None) };
    if i == NOITEM { return Ok(Some(Resolved { target: Target::Menu(menu), menu, parent: None, list_head: 0 })); }
    let head = mem.read_u32(menu + MN_FIRST_ITEM).map_err(Refusal::mem("Menu.FirstItem"))?;
    let Some(item) = nth(mem, head, i)? else { return Ok(None) };
    if s == NOSUB { return Ok(Some(Resolved { target: Target::Item(item), menu, parent: None, list_head: head })); }
    let sub_head = mem.read_u32(item + MI_SUB_ITEM).map_err(Refusal::mem("MenuItem.SubItem"))?;
    Ok(nth(mem, sub_head, s)?.map(|sub| Resolved { target: Target::Sub(sub), menu, parent: Some(item), list_head: sub_head }))
}

fn enabled(mem: &GuestMem, r: &Resolved) -> Result<bool, Refusal> {
    let flag = |at: u32, bit: u16| mem.read_u16(at).map(|f| f & bit != 0).map_err(Refusal::mem("menu flags"));
    let mut on = flag(r.menu + MN_FLAGS, MENUENABLED)?;
    if let Some(p) = r.parent { on &= flag(p + MI_FLAGS, ITEMENABLED)?; }
    if !matches!(r.target, Target::Menu(_)) { on &= flag(r.address() + MI_FLAGS, ITEMENABLED)?; }
    Ok(on)
}

/// ItemAddress(a0 = menu strip, d0 = menu number) -> the Menu / MenuItem / sub-item it names, or NULL.
pub fn item_address<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (strip, code) = (cpu.areg(0), cpu.dreg(0) as u16);
    let at = if strip == 0 { 0 } else { resolve(&a.mem, strip, code)?.map_or(0, |r| r.address()) };
    cpu.set_d0(at);
    Ok(format!("[intuition] ItemAddress strip={strip:#x} number={code:#06x} -> {at:#x}"))
}

/// Tell the native side the strip changed (a check mark moved, an item was enabled or disabled).
fn resync<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    let (Some(&strip), Some(&native)) = (a.menu_strips.get(&win), a.windows.get(&win)) else { return Ok(()) };
    let model = menu::decode_menu_strip(&a.mem, strip)?;
    a.ui.set_menu_strip(native, model).map_err(Refusal::ui)
}

fn on_off<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, on: bool) -> Result<String, Refusal> {
    let (win, code) = (cpu.areg(0), cpu.dreg(0) as u16);
    let name = if on { "OnMenu" } else { "OffMenu" };
    let strip = *a.menu_strips.get(&win).ok_or_else(|| Refusal(format!("{name}: window {win:#x} has no menu strip")))?;
    let r = resolve(&a.mem, strip, code)?.ok_or_else(|| Refusal(format!("{name}: menu number {code:#06x} names nothing in the strip")))?;
    let (at, bit) = r.enable_bit();
    let f = a.mem.read_u16(at).map_err(Refusal::mem("menu flags"))?;
    a.mem.write_u16(at, if on { f | bit } else { f & !bit }).map_err(Refusal::mem("menu flags"))?;
    resync(a, win)?;
    Ok(format!("[intuition] {name} {win:#x} number={code:#06x}"))
}

pub fn on_menu<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { on_off(a, cpu, true) }
pub fn off_menu<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { on_off(a, cpu, false) }

/// Selecting a check item: MENUTOGGLE flips CHECKED; otherwise it becomes checked and every sibling its MutualExclude
/// names loses its check. True when any flag changed.
fn apply_check(mem: &mut GuestMem, r: &Resolved) -> Result<bool, Refusal> {
    let at = r.address() + MI_FLAGS;
    let f = mem.read_u16(at).map_err(Refusal::mem("MenuItem.Flags"))?;
    if f & CHECKIT == 0 { return Ok(false); }
    let mut changed = false;
    let new = if f & MENUTOGGLE != 0 { f ^ CHECKED } else { f | CHECKED };
    if new != f { mem.write_u16(at, new).map_err(Refusal::mem("MenuItem.Flags"))?; changed = true; }
    if f & MENUTOGGLE == 0 {
        let exclude = mem.read_u32(r.address() + MI_EXCLUDE).map_err(Refusal::mem("MenuItem.MutualExclude"))?;
        for k in (0..32u16).filter(|k| exclude & (1 << k) != 0) {
            let Some(sib) = nth(mem, r.list_head, k)? else { continue };
            if sib == r.address() { continue; }
            let sf = mem.read_u16(sib + MI_FLAGS).map_err(Refusal::mem("MenuItem.Flags"))?;
            if sf & CHECKIT != 0 && sf & CHECKED != 0 { mem.write_u16(sib + MI_FLAGS, sf & !CHECKED).map_err(Refusal::mem("MenuItem.Flags"))?; changed = true; }
        }
    }
    Ok(changed)
}

/// A native menu command on guest window `win`: the token is the packed menu number the strip was decoded with. It
/// must name an ENABLED item of the window's current strip (anything else is a named refusal, nothing changes). The
/// item's check state moves as Intuition moves it, its NextSelect becomes MENUNULL (the end of the program's
/// NextSelect loop), and an IDCMP_MENUPICK with Code = the number is posted if the window subscribes.
pub fn menu_pick<U: GuestUi>(a: &mut Amiga<U>, win: u32, token: CallbackToken) -> Result<Delivered, Refusal> {
    let code = u16::try_from(token.0).ok().filter(|&c| c != MENUNULL)
        .ok_or_else(|| Refusal(format!("deliver: MenuCommand token {:#x} is not a menu number", token.0)))?;
    let strip = *a.menu_strips.get(&win).ok_or_else(|| Refusal(format!("deliver: MenuCommand for window {win:#x}, which has no menu strip")))?;
    let r = resolve(&a.mem, strip, code)?.ok_or_else(|| Refusal(format!("deliver: menu number {code:#06x} names nothing in window {win:#x}'s strip")))?;
    if matches!(r.target, Target::Menu(_)) { return Err(Refusal(format!("deliver: menu number {code:#06x} names a menu title, not an item"))); }
    if !enabled(&a.mem, &r)? { return Err(Refusal(format!("deliver: menu number {code:#06x} is disabled"))); }
    let msg = if idcmp::subscribed(a, win, IDCMP_MENUPICK)? { Some(idcmp::reserve(a)?) } else { None };
    let applied = apply_check(&mut a.mem, &r).and_then(|changed| {
        a.mem.write_u16(r.address() + MI_NEXT_SELECT, MENUNULL).map_err(Refusal::mem("MenuItem.NextSelect"))?;
        if changed { resync(a, win)?; }
        Ok(())
    });
    match (applied, msg) {
        (Err(e), Some(m)) => { let _ = a.heap.free(m, idcmp::INTUIMESSAGE_SIZE); Err(e) }
        (Err(e), None) => Err(e),
        (Ok(()), Some(m)) => idcmp::queue(a, win, m, IDCMP_MENUPICK, code, 0),
        (Ok(()), None) => Ok(Delivered::IgnoredUnsubscribed),
    }
}
