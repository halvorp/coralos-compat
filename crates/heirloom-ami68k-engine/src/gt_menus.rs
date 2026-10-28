//! gadtools.library menus: CreateMenusA builds REAL guest `struct Menu` / `MenuItem` / `IntuiText` chains from a
//! `struct NewMenu` array, LayoutMenusA / LayoutMenuItemsA fill in their geometry, FreeMenus gives them back.
//!
//! The chains are exactly what SetMenuStrip decodes (menu.rs), so a GadTools menu reaches the native side through the
//! same path as a hand-built one; the native menu is guestui's, the geometry written here is guest-visible only.
//! Layout follows the AROS gadtools constants (facts only: item text padding 2/2/1/1, bar label 10x7, 8 px between
//! titles' borders) over the screen's topaz-8 metrics (graphics.rs); the checkmark and Amiga-key image widths are
//! approximations (the engine has no DrawInfo images), so item widths can differ from a real Amiga by a few pixels.
//!
//! Offsets: m68k gcc -mno-align-int over the AROS field lists. NewMenu (20 bytes): nm_Type 0, nm_Label 2, nm_CommKey 6,
//! nm_Flags 10, nm_MutualExclude 12, nm_UserData 16. Menu 30 (+4 userdata = GTMENU_USERDATA): NextMenu 0, LeftEdge 4,
//! TopEdge 6, Width 8, Height 10, Flags 12, MenuName 14, FirstItem 18. MenuItem 34 (+4 userdata = GTMENUITEM_USERDATA):
//! NextItem 0, LeftEdge 4, TopEdge 6, Width 8, Height 10, Flags 12, MutualExclude 14, ItemFill 18, Command 26, SubItem
//! 28, NextSelect 32. IntuiText 20: FrontPen 0, DrawMode 2, IText 12, NextText 16. Image 20: Width 4, Height 6.
//!
//! One block per Menu / MenuItem (text items carry two IntuiTexts and the "»" string: the second IntuiText holds a
//! command STRING or the sub-menu mark). Every block is tracked host-side under the first menu: FreeMenus of anything
//! CreateMenusA did not return, or of a strip an open window still shows, is a named refusal.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::{graphics::TOPAZ8_WIDTH, machine::Amiga, memory, tags, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use crate::cpu::Cpu;

pub const LVO_CREATE_MENUS_A: i32 = -48;
pub const LVO_FREE_MENUS: i32 = -54;
pub const LVO_LAYOUT_MENU_ITEMS_A: i32 = -60;
pub const LVO_LAYOUT_MENUS_A: i32 = -66;

const NM_END: u8 = 0;
const NM_TITLE: u8 = 1;
const NM_ITEM: u8 = 2;
const NM_SUB: u8 = 3;
const IM_ITEM: u8 = 4;
const IM_SUB: u8 = 5;
const NM_IGNORE: u8 = 0x40;
const NM_BARLABEL: u32 = 0xFFFF_FFFF;
const NM_SIZE: u32 = 20;
const MAX_ENTRIES: u32 = 4096;
/// Intuition's per-menu limits: 64 items (NOITEM = 63 is "none"), 32 sub items.
const MAX_ITEMS: u32 = 64;
const MAX_SUBS: u32 = 32;
const MAX_CHAIN: usize = 64;

const GT_TAGBASE: u32 = tags::TAG_USER + 0x0008_0000;
const GTMN_FRONTPEN: u32 = GT_TAGBASE + 50;
const GTMN_MENU: u32 = GT_TAGBASE + 60;
const GTMN_FULLMENU: u32 = GT_TAGBASE + 62;
const GTMN_SECONDARY_ERROR: u32 = GT_TAGBASE + 63;
const GTMENU_TRIMMED: u32 = 1;
const GTMENU_INVALID: u32 = 2;
const GTMENU_NOMEM: u32 = 3;

// Block sizes: Menu + userdata; MenuItem + userdata, then two IntuiTexts + "»\0" (text), an Image (bar), or nothing.
const MENU_SIZE: u32 = 34;
const ITEM_SIZE: u32 = 38;
const TEXT_ITEM_SIZE: u32 = ITEM_SIZE + 40 + 2;
const BAR_ITEM_SIZE: u32 = ITEM_SIZE + 20;
const MENU_USERDATA: u32 = 30;
const ITEM_USERDATA: u32 = 34;
const IT0: u32 = ITEM_SIZE;
const IT1: u32 = ITEM_SIZE + 20;
const SUB_MARK: u32 = ITEM_SIZE + 40;
/// ISO-8859-1 "»" (AROS' ISOASCII_RGUILLEMET), the mark on an item that opens a sub menu.
const RGUILLEMET: u8 = 0xBB;

const MENUENABLED: u16 = 0x0001;
const CHECKIT: u16 = 0x0001;
const ITEMTEXT: u16 = 0x0002;
const COMMSEQ: u16 = 0x0004;
const MENUTOGGLE: u16 = 0x0008;
const ITEMENABLED: u16 = 0x0010;
const HIGHCOMP: u16 = 0x0040;
const CHECKED: u16 = 0x0100;

// Layout metrics (see the module doc).
const ITEXT_EXTRA_LEFT: i32 = 2;
const ITEXT_EXTRA_RIGHT: i32 = 2;
const ITEXT_EXTRA_V: i32 = 1;
const FONT_HEIGHT: i32 = 8;
const MENU_H_BORDER: i32 = 4; // screen MenuHBorder (tests/oracle/screen.bin)
const BAR_H_BORDER: i32 = 5; // screen BarHBorder
const CHECKMARK_WIDTH: i32 = 16 + 2; // approximate dri_CheckMark width + CHECKMARK_TEXT_SPACING
const AMIGAKEY_WIDTH: i32 = 24; // approximate dri_AmigaKey width
const BAR_SIZE: (i32, i32) = (10, 7);

/// Why building stopped: a GadTools secondary error, or a guest memory refusal.
enum Fail { Gt(u32), Mem(Refusal) }
impl From<Refusal> for Fail { fn from(r: Refusal) -> Self { Fail::Mem(r) } }

fn put16<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u16) -> Result<(), Fail> { Ok(a.mem.write_u16(at, v).map_err(Refusal::mem("menu"))?) }
fn put32<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32) -> Result<(), Fail> { Ok(a.mem.write_u32(at, v).map_err(Refusal::mem("menu"))?) }
fn get16<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u16, Fail> { Ok(a.mem.read_u16(at).map_err(Refusal::mem("menu"))?) }
fn get32<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u32, Fail> { Ok(a.mem.read_u32(at).map_err(Refusal::mem("menu"))?) }

fn block<U: GuestUi>(a: &mut Amiga<U>, blocks: &mut Vec<(u32, u32)>, size: u32) -> Result<u32, Fail> {
    let at = memory::alloc_zeroed(a, size, "CreateMenusA").map_err(|_| Fail::Gt(GTMENU_NOMEM))?;
    blocks.push((at, size));
    Ok(at)
}

/// One MenuItem (or sub item) from the NewMenu entry at `e`.
fn make_item<U: GuestUi>(a: &mut Amiga<U>, blocks: &mut Vec<(u32, u32)>, e: u32, mut image: bool, pen: u8) -> Result<u32, Fail> {
    let (label, key, nflags, exclude, user) = (get32(a, e + 2)?, get32(a, e + 6)?, get16(a, e + 10)?, get32(a, e + 12)?, get32(a, e + 16)?);
    let bar = label == NM_BARLABEL;
    image |= bar;
    let item = block(a, blocks, if bar { BAR_ITEM_SIZE } else if image { ITEM_SIZE } else { TEXT_ITEM_SIZE })?;
    // nm_Flags carries the DISABLED sense of ITEMENABLED (NM_ITEMDISABLED); CHECKIT / MENUTOGGLE / CHECKED pass through.
    let mut flags = ((nflags & (ITEMENABLED | CHECKIT | MENUTOGGLE | CHECKED)) ^ ITEMENABLED) | HIGHCOMP;
    if key != 0 && nflags & COMMSEQ == 0 {
        flags |= COMMSEQ;
        let c = a.mem.read_u8(key).map_err(Refusal::mem("nm_CommKey"))?;
        a.mem.write_u8(item + 26, c).map_err(Refusal::mem("MenuItem.Command"))?;
    }
    put32(a, item + 14, exclude)?;
    put32(a, item + ITEM_USERDATA, user)?;
    if !image {
        if label == 0 { return Err(Fail::Gt(GTMENU_INVALID)); }
        flags |= ITEMTEXT;
        put32(a, item + 18, item + IT0)?;
        a.mem.write_u8(item + IT0, pen).map_err(Refusal::mem("IntuiText.FrontPen"))?;
        put32(a, item + IT0 + 12, label)?;
        if key != 0 && nflags & COMMSEQ != 0 {
            a.mem.write_u8(item + IT1, pen).map_err(Refusal::mem("IntuiText.FrontPen"))?;
            put32(a, item + IT1 + 12, key)?;
            put32(a, item + IT0 + 16, item + IT1)?;
        }
    } else if !bar {
        put32(a, item + 18, label)?; // an Image the program made
    } else {
        flags &= !ITEMENABLED; // a separator is not selectable
        put32(a, item + 18, item + ITEM_SIZE)?;
        put16(a, item + ITEM_SIZE + 4, 20)?;
        put16(a, item + ITEM_SIZE + 6, 2)?;
    }
    put16(a, item + 12, flags)?;
    Ok(item)
}

/// `item` just got its first sub item: it can no longer carry a command key, and shows the sub-menu mark.
fn mark_parent<U: GuestUi>(a: &mut Amiga<U>, item: u32) -> Result<(), Fail> {
    let f = get16(a, item + 12)?;
    put16(a, item + 12, f & !COMMSEQ)?;
    if f & ITEMTEXT != 0 {
        let it = get32(a, item + 18)?;
        if get32(a, it + 16)? == 0 {
            a.mem.write_u8(item + SUB_MARK, RGUILLEMET).map_err(Refusal::mem("sub mark"))?;
            put32(a, it + 16, item + IT1)?;
            put32(a, item + IT1 + 12, item + SUB_MARK)?;
        }
    }
    Ok(())
}

/// Walk the NewMenu array at `nm`, building into `blocks`. Returns (first menu or first loose item, GTMENU_ code).
fn build<U: GuestUi>(a: &mut Amiga<U>, nm: u32, full: bool, pen: u8, blocks: &mut Vec<(u32, u32)>) -> Result<(u32, u32), Fail> {
    let (mut first_menu, mut menu, mut first_item, mut item, mut sub) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let (mut items, mut subs, mut err, mut prev) = (0u32, 0u32, 0u32, 0xFFu8);
    for n in 0..=MAX_ENTRIES {
        if n == MAX_ENTRIES { return Err(Fail::Mem(Refusal(format!("CreateMenusA: NewMenu table longer than {MAX_ENTRIES} entries (no NM_END?)")))); }
        let e = nm + n * NM_SIZE;
        let ty = a.mem.read_u8(e).map_err(Refusal::mem("nm_Type"))?;
        match ty {
            NM_END => break,
            t if t & NM_IGNORE != 0 => {}
            NM_TITLE => {
                let label = get32(a, e + 2)?;
                if label == 0 || label == NM_BARLABEL { return Err(Fail::Gt(GTMENU_INVALID)); }
                let m = block(a, blocks, MENU_SIZE)?;
                let nflags = get16(a, e + 10)?;
                put16(a, m + 12, (nflags & MENUENABLED) ^ MENUENABLED)?; // NM_MENUDISABLED is the inverse of MENUENABLED
                put32(a, m + 14, label)?;
                let user = get32(a, e + 16)?;
                put32(a, m + MENU_USERDATA, user)?;
                if first_menu == 0 { first_menu = m } else { put32(a, menu, m)? }
                (menu, items, item, first_item, sub) = (m, 0, 0, 0, 0);
            }
            NM_ITEM | IM_ITEM => {
                if full && menu == 0 { return Err(Fail::Gt(GTMENU_INVALID)); }
                if items >= MAX_ITEMS { err = GTMENU_TRIMMED; } else {
                    let it = make_item(a, blocks, e, ty == IM_ITEM, pen)?;
                    if item != 0 { put32(a, item, it)? } else { first_item = it; if menu != 0 { put32(a, menu + 18, it)? } }
                    (item, sub, subs) = (it, 0, 0);
                    items += 1;
                }
            }
            NM_SUB | IM_SUB => {
                if (full && item == 0) || prev == NM_TITLE { return Err(Fail::Gt(GTMENU_INVALID)); }
                if subs >= MAX_SUBS { err = GTMENU_TRIMMED; } else {
                    let it = make_item(a, blocks, e, ty == IM_SUB, pen)?;
                    if sub != 0 { put32(a, sub, it)? } else if item != 0 { put32(a, item + 28, it)?; mark_parent(a, item)?; }
                    sub = it;
                    subs += 1;
                }
            }
            _ => return Err(Fail::Gt(GTMENU_INVALID)),
        }
        prev = ty;
    }
    Ok((if first_menu != 0 { first_menu } else { first_item }, err))
}

/// CreateMenusA(a0 = NewMenu array, a1 = tags: GTMN_FullMenu, GTMN_SecondaryError, GTMN_FrontPen) -> the first Menu
/// (the first MenuItem when the table has no titles and GTMN_FullMenu is not set), or NULL.
pub fn create_menus<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (nm, t) = (cpu.areg(0), tags::walk(&a.mem, cpu.areg(1))?);
    let full = tags::get(&t, GTMN_FULLMENU).unwrap_or(0) != 0;
    let pen = tags::get(&t, GTMN_FRONTPEN).unwrap_or(0) as u8;
    let mut blocks = Vec::new();
    let result = build(a, nm, full, pen, &mut blocks);
    let (first, code) = match result {
        Ok(ok) => ok,
        Err(f) => {
            for (at, size) in blocks.drain(..) { a.heap.free(at, size).map_err(|e| Refusal(format!("CreateMenusA cleanup: {e}")))?; }
            match f {
                Fail::Mem(r) => return Err(r),
                Fail::Gt(code) => (0, code),
            }
        }
    };
    if let Some(p) = tags::get(&t, GTMN_SECONDARY_ERROR).filter(|&p| p != 0) { a.mem.write_u32(p, code).map_err(Refusal::mem("GTMN_SecondaryError"))?; }
    if first != 0 { a.menu_allocs.insert(first, blocks); }
    cpu.set_d0(first);
    Ok(format!("[gadtools] CreateMenusA newmenu={nm:#x} full={full} -> {first:#x} (secondary error {code})"))
}

/// FreeMenus(a0 = menu from CreateMenusA, or NULL).
pub fn free_menus<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let first = cpu.areg(0);
    if first == 0 { return Ok("[gadtools] FreeMenus NULL".into()); }
    if a.menu_strips.values().any(|&s| s == first) { return Err(Refusal(format!("FreeMenus: {first:#x} is still the menu strip of an open window"))); }
    let blocks = a.menu_allocs.remove(&first).ok_or_else(|| Refusal(format!("FreeMenus: {first:#x} was not made by CreateMenusA")))?;
    let n = blocks.len();
    for (at, size) in blocks { a.heap.free(at, size).map_err(|e| Refusal(format!("FreeMenus: {e}")))?; }
    Ok(format!("[gadtools] FreeMenus {first:#x} ({n} blocks)"))
}

/// The nodes of a NextXxx-at-offset-0 chain, bounded.
fn chain(mem: &GuestMem, head: u32) -> Result<Vec<u32>, Refusal> {
    let (mut out, mut at) = (Vec::new(), head);
    while at != 0 {
        if out.len() >= MAX_CHAIN { return Err(Refusal(format!("menu chain longer than {MAX_CHAIN} (cycle?)"))); }
        out.push(at);
        at = mem.read_u32(at).map_err(Refusal::mem("menu chain"))?;
    }
    Ok(out)
}

fn text_px(mem: &GuestMem, ptr: u32) -> Result<i32, Refusal> {
    Ok(mem.read_cstr(ptr).map_err(Refusal::mem("menu text"))?.chars().count() as i32 * TOPAZ8_WIDTH as i32)
}

/// (width, height) one item wants.
fn item_size(mem: &GuestMem, item: u32) -> Result<(i32, i32), Refusal> {
    let m = Refusal::mem("MenuItem");
    let flags = mem.read_u16(item + 12).map_err(&m)?;
    let fill = mem.read_u32(item + 18).map_err(&m)?;
    if fill == 0 { return Err(Refusal(format!("layout: MenuItem {item:#x} has no ItemFill"))); }
    let check = if flags & CHECKIT != 0 { CHECKMARK_WIDTH } else { 0 };
    if flags & ITEMTEXT != 0 {
        let mut w = text_px(mem, mem.read_u32(fill + 12).map_err(&m)?)? + check + ITEXT_EXTRA_LEFT + ITEXT_EXTRA_RIGHT;
        let next = mem.read_u32(fill + 16).map_err(&m)?;
        if next != 0 { w += text_px(mem, mem.read_u32(next + 12).map_err(&m)?)?; }
        if flags & COMMSEQ != 0 { w += AMIGAKEY_WIDTH + TOPAZ8_WIDTH as i32; }
        return Ok((w, FONT_HEIGHT + 2 * ITEXT_EXTRA_V));
    }
    let (iw, ih) = (mem.read_u16(fill + 4).map_err(&m)? as i32, mem.read_u16(fill + 6).map_err(&m)? as i32);
    Ok(if flags & ITEMENABLED == 0 && iw == 20 && ih == 2 { BAR_SIZE } else { (iw + check + 2, ih + 2) })
}

/// Lay one item list (and its sub lists) out as a single column, all items as wide as the widest (and `min_width`).
fn layout_items<U: GuestUi>(a: &mut Amiga<U>, first: u32, min_width: i32) -> Result<(), Refusal> {
    let m = Refusal::mem("layout");
    let items = chain(&a.mem, first)?;
    let sizes = items.iter().map(|&i| item_size(&a.mem, i)).collect::<Result<Vec<_>, _>>()?;
    let width = sizes.iter().map(|s| s.0).max().unwrap_or(0).max(min_width);
    let mut y = 0;
    for (&it, &(_, h)) in items.iter().zip(&sizes) {
        for (off, v) in [(4, 0), (6, y), (8, width), (10, h)] { a.mem.write_u16(it + off, v as u16).map_err(&m)?; }
        y += h;
        let sub = a.mem.read_u32(it + 28).map_err(&m)?;
        if sub != 0 {
            layout_items(a, sub, 0)?;
            for s in chain(&a.mem, sub)? { a.mem.write_u16(s + 4, (width - TOPAZ8_WIDTH as i32) as u16).map_err(&m)?; } // overlaps the parent's right edge
        }
    }
    Ok(())
}

/// LayoutMenuItemsA(a0 = first item, a1 = VisualInfo, a2 = tags: GTMN_Menu = the Menu these items belong to, whose
/// title the box is made at least as wide as) -> TRUE.
pub fn layout_menu_items<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let first = cpu.areg(0);
    let t = tags::walk(&a.mem, cpu.areg(2))?;
    let title = match tags::get(&t, GTMN_MENU).filter(|&m| m != 0) {
        Some(m) if a.mem.read_u32(m + 18).map_err(Refusal::mem("Menu.FirstItem"))? == first => text_px(&a.mem, a.mem.read_u32(m + 14).map_err(Refusal::mem("Menu.MenuName"))?)?,
        _ => 0,
    };
    layout_items(a, first, title)?;
    cpu.set_d0(1);
    Ok(format!("[gadtools] LayoutMenuItemsA {first:#x}"))
}

/// LayoutMenusA(a0 = first Menu, a1 = VisualInfo, a2 = tags) -> TRUE: titles side by side along the bar, every menu's
/// items laid out under them.
pub fn layout_menus<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let first = cpu.areg(0);
    let m = Refusal::mem("layout");
    let (mut x, menus) = (0, chain(&a.mem, first)?);
    for &menu in &menus {
        let name = text_px(&a.mem, a.mem.read_u32(menu + 14).map_err(&m)?)?;
        let items = a.mem.read_u32(menu + 18).map_err(&m)?;
        if items != 0 { layout_items(a, items, name)?; }
        let width = name + MENU_H_BORDER * 2;
        for (off, v) in [(4, x), (6, 0), (8, width), (10, FONT_HEIGHT)] { a.mem.write_u16(menu + off, v as u16).map_err(&m)?; }
        x += width + BAR_H_BORDER * 2;
    }
    cpu.set_d0(1);
    Ok(format!("[gadtools] LayoutMenusA {first:#x} ({} menus)", menus.len()))
}
