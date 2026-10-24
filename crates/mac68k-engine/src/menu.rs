//! The Menu Manager: `MenuInfo` blocks in guest memory, the menu bar, and the bridge to `GuestUi::set_menu_strip`.
//!
//! A MenuHandle is a real Handle (see [`crate::memmgr`]) to this layout, which is also the on-disk `MENU` resource
//! layout, so `GetMenu` simply returns the resource's own handle (as the real one does):
//!
//! ```text
//! 0 menuID i16 | 2 menuWidth i16 | 4 menuHeight i16 | 6 menuProc (MDEF id i16 + 2 filler) | 10 enableFlags u32
//! 14 title: Pascal string | items: { text: Pascal string, icon u8, key u8, mark u8, style u8 }* | 0 terminator
//! ```
//! (Offsets and the AppendMenu metacharacters `^ ! < / (` `;` are interface facts read from executor's
//! `docs/subsystems`-adjacent `src/menu/menu.cpp` structure, SIZEOFMINFO = 14 + title, SIZEOFMEXT = 4.) `enableFlags`
//! bit 0 is the whole menu, bit n item n.
//!
//! The menu bar is global on a Mac; `GuestUi` menu strips are per window. `DrawMenuBar` therefore sets the strip on
//! EVERY open guest window, and a window opened later gets the current strip at once. A hierarchical item (key byte
//! `$1B`, mark byte = the submenu's id) becomes `MenuItem::sub_items`. Item tokens are `menuID << 16 | item`.
use crate::machine::{Call, Done, Mac};
use crate::memmgr::MemMgr;
use crate::pascal::{read_pstring, read_pstring_bytes, Arg};
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use guestui_abi::{CallbackToken, GuestUi, Menu, MenuItem, MenuModel};
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

const HEADER: usize = 14;
/// The key byte that marks a hierarchical item.
const KEY_HIERARCHICAL: u8 = 0x1B;
/// A check mark in the system font; any non-zero mark is "checked" for GuestUi.
pub const CHECK_MARK: u8 = 0x12;

/// Menu Manager state: id -> handle for every known menu, and the ids in the menu bar in order.
#[derive(Default)]
pub struct MenuMgr {
    pub by_id: BTreeMap<i16, u32>,
    pub bar: Vec<i16>,
    /// DrawMenuBar has run: new windows get the strip.
    pub drawn: bool,
    /// `HideMenuBar` is in effect: the host's strip is empty and `MBarHeight` is 0 until `ShowMenuBar` (spec: macos9-specs/MenusLib/HideMenuBar).
    pub hidden: bool,
    /// `MBarHeight` as it was when the bar was hidden.
    pub saved_bar_height: u16,
}

impl MenuMgr {
    pub fn forget(&mut self, h: u32) {
        self.by_id.retain(|_, v| *v != h);
        let ids: Vec<i16> = self.by_id.keys().copied().collect();
        self.bar.retain(|i| ids.contains(i));
    }

    /// Decode a `MenuItem` token to the choice `MenuSelect` returns, checking the menu and item exist.
    pub fn choice_for_token(&self, mem: &GuestMem, mm: &MemMgr, token: u64) -> Result<u32, Refusal> {
        let (id, item) = ((token >> 16) as u16 as i16, (token & 0xFFFF) as usize);
        let h = *self.by_id.get(&id).ok_or_else(|| Refusal(format!("menu command: no menu {id}")))?;
        let n = MenuView { mem, mm, h }.parse().map(|p| p.items.len()).unwrap_or(0);
        if item == 0 || item > n { return Err(Refusal(format!("menu command: menu {id} has no item {item}"))); }
        Ok(((id as u16 as u32) << 16) | item as u32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item { pub text: Vec<u8>, pub icon: u8, pub key: u8, pub mark: u8, pub style: u8 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed { pub id: i16, pub flags: u32, pub title: Vec<u8>, pub items: Vec<Item> }

/// A read-only view of a menu's block.
struct MenuView<'a> { mem: &'a GuestMem, mm: &'a MemMgr, h: u32 }

impl MenuView<'_> {
    fn parse(&self) -> Result<Parsed, Refusal> {
        let b = self.mm.block(self.h).ok_or_else(|| Refusal(format!("menu {:#x} is not a handle", self.h)))?;
        let bytes = self.mem.bytes(self.mem.span(b.ptr, b.size).map_err(Refusal::mem("menu"))?).map_err(Refusal::mem("menu"))?;
        parse_block(bytes)
    }
}

pub fn parse_block(b: &[u8]) -> Result<Parsed, Refusal> {
    let bad = |w: &str| Refusal(format!("malformed menu block: {w}"));
    if b.len() < HEADER + 1 { return Err(bad("shorter than the 14-byte header and a title")); }
    let id = i16::from_be_bytes([b[0], b[1]]);
    let flags = u32::from_be_bytes([b[10], b[11], b[12], b[13]]);
    let tl = b[HEADER] as usize;
    let title = b.get(HEADER + 1..HEADER + 1 + tl).ok_or_else(|| bad("title past the end"))?.to_vec();
    let mut at = HEADER + 1 + tl;
    let mut items = Vec::new();
    loop {
        let l = *b.get(at).ok_or_else(|| bad("no terminating zero after the items"))? as usize;
        if l == 0 { break; }
        let text = b.get(at + 1..at + 1 + l).ok_or_else(|| bad("item text past the end"))?.to_vec();
        let m = b.get(at + 1 + l..at + 5 + l).ok_or_else(|| bad("item attributes past the end"))?;
        items.push(Item { text, icon: m[0], key: m[1], mark: m[2], style: m[3] });
        at += 5 + l;
    }
    Ok(Parsed { id, flags, title, items })
}

pub fn encode_block(p: &Parsed, width: i16, height: i16, proc_id: i16) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&p.id.to_be_bytes());
    b.extend_from_slice(&width.to_be_bytes());
    b.extend_from_slice(&height.to_be_bytes());
    b.extend_from_slice(&proc_id.to_be_bytes());
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&p.flags.to_be_bytes());
    b.push(p.title.len() as u8);
    b.extend_from_slice(&p.title);
    for it in &p.items {
        b.push(it.text.len() as u8);
        b.extend_from_slice(&it.text);
        b.extend_from_slice(&[it.icon, it.key, it.mark, it.style]);
    }
    b.push(0);
    b
}

/// Parse an AppendMenu string into items. `^n` icon, `!c` mark, `<B/I/U/O/S` style, `/c` key equivalent, `(` disabled,
/// `;` or newline separates items. Returns each item with whether it starts disabled.
pub fn parse_append(s: &[u8]) -> Vec<(Item, bool)> {
    #[derive(Default)]
    struct Pending { text: Vec<u8>, icon: u8, mark: u8, key: u8, style: u8, dis: bool }
    impl Pending {
        /// Finish the item being built (an empty text is not an item) and start the next one.
        fn finish(&mut self, out: &mut Vec<(Item, bool)>) {
            let p = core::mem::take(self);
            if !p.text.is_empty() { out.push((Item { text: p.text, icon: p.icon, key: p.key, mark: p.mark, style: p.style }, p.dis)); }
        }
    }
    let mut out = Vec::new();
    let mut cur = Pending::default();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        match c {
            b'^' if i < s.len() => { cur.icon = s[i].wrapping_sub(b'0'); i += 1; }
            b'!' if i < s.len() => { cur.mark = s[i]; i += 1; }
            b'/' if i < s.len() => { cur.key = s[i]; i += 1; }
            b'<' if i < s.len() => {
                let bit = match s[i] { b'B' => 1, b'I' => 2, b'U' => 4, b'O' => 8, b'S' => 16, _ => 0 };
                if bit == 0 { cur.text.push(b'<'); cur.text.push(s[i]); } else { cur.style |= bit; }
                i += 1;
            }
            b'(' => cur.dis = true,
            b';' | b'\n' | b'\r' => cur.finish(&mut out),
            _ => cur.text.push(c),
        }
    }
    cur.finish(&mut out);
    out
}

/// Mac text for GuestUi: Latin-1 for the printable range, the system font's Apple glyph ($14) as "Apple".
pub fn mac_text(b: &[u8]) -> String {
    if b == [0x14] { return "Apple".to_string(); }
    b.iter().map(|&c| char::from(c)).collect()
}

impl<U: GuestUi> Mac<U> {
    pub fn menu_view(&self, h: u32) -> Result<Parsed, Refusal> { MenuView { mem: &self.mem, mm: &self.mm, h }.parse() }

    /// Replace a menu's block with `p` (the block moves; the handle stays).
    pub fn menu_store(&mut self, h: u32, p: &Parsed) -> Result<(), Refusal> {
        let old = self.mm.block(h).ok_or_else(|| Refusal(format!("menu {h:#x} is not a handle")))?;
        let keep = self.mem.bytes(self.mem.span(old.ptr, 10.min(old.size)).map_err(Refusal::mem("menu"))?).map_err(Refusal::mem("menu"))?.to_vec();
        let mut bytes = encode_block(p, 0, 0, 0);
        if keep.len() == 10 { bytes[2..10].copy_from_slice(&keep[2..10]); } // width, height, proc survive an edit
        let was_locked = old.locked;
        if was_locked { self.mm.set_lock(&mut self.mem, h, false).map_err(|e| Refusal(format!("menu unlock {e}")))?; }
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, h, bytes.len() as u32).map_err(|e| Refusal(format!("menu SetHandleSize {e}")))?;
        let ptr = self.mm.block(h).map(|b| b.ptr).unwrap_or(0);
        self.mem.write_bytes(ptr, &bytes).map_err(Refusal::mem("menu"))?;
        if was_locked { let _ = self.mm.set_lock(&mut self.mem, h, true); }
        Ok(())
    }

    fn menu_model(&self, ids: &[i16], depth: u8) -> Result<Vec<Menu>, Refusal> {
        let mut menus = Vec::new();
        for id in ids {
            let Some(&h) = self.menus.by_id.get(id) else { continue };
            let p = self.menu_view(h)?;
            let items = self.item_models(&p, depth)?;
            menus.push(Menu { title: mac_text(&p.title), enabled: p.flags & 1 != 0, items });
        }
        Ok(menus)
    }

    fn item_models(&self, p: &Parsed, depth: u8) -> Result<Vec<MenuItem>, Refusal> {
        let mut out = Vec::new();
        for (i, it) in p.items.iter().enumerate() {
            let n = i as u32 + 1;
            let sub = if it.key == KEY_HIERARCHICAL && depth < 4 { self.menu_model(&[i16::from(it.mark)], depth + 1)?.into_iter().next().map(|m| m.items).unwrap_or_default() } else { Vec::new() };
            let plain = it.key != KEY_HIERARCHICAL;
            out.push(MenuItem {
                label: mac_text(&it.text),
                enabled: n > 31 || p.flags & (1 << n) != 0,
                shortcut: (plain && it.key >= 0x20).then(|| char::from(it.key).to_ascii_uppercase()),
                check: (plain && it.mark != 0).then_some(true),
                sub_items: sub,
                token: CallbackToken((u64::from(p.id as u16) << 16) | u64::from(n)),
            });
        }
        Ok(out)
    }

    /// The bar as a GuestUi model.
    pub fn menu_strip(&self) -> Result<MenuModel, Refusal> {
        if self.menus.hidden { return Ok(MenuModel { menus: Vec::new() }); }
        Ok(MenuModel { menus: self.menu_model(&self.menus.bar.clone(), 0)? })
    }

    /// Push the current bar to every open guest window.
    pub fn sync_menus(&mut self) -> Result<(), Refusal> {
        let model = self.menu_strip()?;
        let natives: Vec<_> = self.windows.values().filter_map(|w| w.native).collect();
        for n in natives { self.ui.set_menu_strip(n, model.clone()).map_err(Refusal::ui)?; }
        Ok(())
    }
}

fn done(log: String) -> Result<Done, Refusal> { Ok(Done { result: 0, log }) }
pub(crate) fn h_of<U: GuestUi>(m: &Mac<U>, what: &str, h: u32) -> Result<(), Refusal> {
    if m.menu_view(h).is_ok() { Ok(()) } else { Err(Refusal(format!("{what}: {h:#x} is not a MenuHandle"))) }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA930, &[], None, init_menus::<U>)?; // InitMenus
    m.register(0xA931, &[Arg::W, Arg::L], Some(Arg::L), new_menu::<U>)?; // NewMenu(id, title)
    m.register(0xA9BF, &[Arg::W], Some(Arg::L), get_menu::<U>)?; // GetMenu(resourceID) = _GetRMenu
    m.register(0xA932, &[Arg::L], None, dispose_menu::<U>)?;
    m.register(0xA933, &[Arg::L, Arg::L], None, append_menu::<U>)?; // AppendMenu(menu, data)
    m.register(0xA94D, &[Arg::L, Arg::L], None, add_res_menu::<U>)?; // AddResMenu(menu, theType)
    m.register(0xA935, &[Arg::L, Arg::W], None, insert_menu::<U>)?; // InsertMenu(menu, beforeID)
    m.register(0xA936, &[Arg::W], None, delete_menu::<U>)?;
    m.register(0xA934, &[], None, clear_menu_bar::<U>)?;
    m.register(0xA937, &[], None, draw_menu_bar::<U>)?;
    m.register(0xA9C0, &[Arg::W], Some(Arg::L), get_new_mbar::<U>)?;
    m.register(0xA93C, &[Arg::L], None, set_menu_bar::<U>)?;
    m.register(0xA949, &[Arg::W], Some(Arg::L), get_mhandle::<U>)?;
    m.register(0xA950, &[Arg::L], Some(Arg::W), count_mitems::<U>)?;
    m.register(0xA945, &[Arg::L, Arg::W, Arg::B], None, check_item::<U>)?;
    m.register(0xA939, &[Arg::L, Arg::W], None, enable_item::<U>)?;
    m.register(0xA93A, &[Arg::L, Arg::W], None, disable_item::<U>)?;
    m.register(0xA946, &[Arg::L, Arg::W, Arg::L], None, get_item::<U>)?; // GetItem(menu, item, VAR itemString)
    m.register(0xA947, &[Arg::L, Arg::W, Arg::L], None, set_item::<U>)?;
    m.register(0xA826, &[Arg::L, Arg::L, Arg::W], None, insert_menu_item::<U>)?; // InsertMenuItem(menu, itemString, afterItem)
    m.register(0xA952, &[Arg::L, Arg::W], None, delete_menu_item::<U>)?; // DeleteMenuItem(menu, item)
    m.register(0xA93D, &[Arg::L], Some(Arg::L), menu_select::<U>)?;
    m.register(0xA938, &[Arg::W], None, crate::quickdraw::accept::<U>) // HiliteMenu(menuID)
}

fn init_menus<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.menus.by_id.clear();
    m.menus.bar.clear();
    m.menus.drawn = false;
    let list = m.mm.new_handle(&mut m.mem, &mut m.heap, 8, true).map_err(|e| Refusal(format!("InitMenus: no memory ({e})")))?;
    m.mem.write_u32(crate::lowmem::MENU_LIST, list).map_err(Refusal::mem("MenuList"))?;
    done("[menu] InitMenus".into())
}

fn make_menu<U: GuestUi>(m: &mut Mac<U>, id: i16, title: Vec<u8>) -> Result<u32, Refusal> {
    let p = Parsed { id, flags: 0xFFFF_FFFF, title, items: Vec::new() };
    let bytes = encode_block(&p, 0, 0, 0);
    let h = m.mm.new_handle(&mut m.mem, &mut m.heap, bytes.len() as u32, false).map_err(|e| Refusal(format!("NewMenu: no memory ({e})")))?;
    let ptr = m.mm.block(h).map(|b| b.ptr).unwrap_or(0);
    m.mem.write_bytes(ptr, &bytes).map_err(Refusal::mem("menu"))?;
    m.menus.by_id.insert(id, h);
    Ok(h)
}

fn new_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let title = read_pstring_bytes(&m.mem, c.args.get(1))?;
    let h = make_menu(m, id, title.clone())?;
    Ok(Done { result: h, log: format!("[menu] NewMenu {id} \"{}\" -> {h:#x}", mac_text(&title)) })
}

fn get_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rid = c.args.get(0) as u16 as i16;
    let r = m.get_resource(*b"MENU", rid, false);
    let Ok(h) = r else { return Ok(Done { result: 0, log: format!("[menu] GetMenu {rid}: no MENU resource (ResErr {})", r.unwrap_err()) }) };
    let p = m.menu_view(h)?;
    m.menus.by_id.insert(p.id, h);
    Ok(Done { result: h, log: format!("[menu] GetMenu {rid} -> menu {} \"{}\" {} items", p.id, mac_text(&p.title), p.items.len()) })
}

fn dispose_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    h_of(m, "DisposeMenu", h)?;
    m.dispose_handle(h).map_err(|e| Refusal(format!("DisposeMenu: {e}")))?;
    done(format!("[menu] DisposeMenu {h:#x}"))
}

fn append_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    h_of(m, "AppendMenu", h)?;
    let data = read_pstring_bytes(&m.mem, c.args.get(1))?;
    let mut p = m.menu_view(h)?;
    let added = parse_append(&data);
    for (it, dis) in &added {
        p.items.push(it.clone());
        let n = p.items.len() as u32;
        if n <= 31 { if *dis { p.flags &= !(1 << n); } else { p.flags |= 1 << n; } }
    }
    m.menu_store(h, &p)?;
    done(format!("[menu] AppendMenu menu {} {:?} +{} items (now {})", p.id, mac_text(&data), added.len(), p.items.len()))
}

/// `InsertMenuItem(theMenu, itemString, afterItem)`: like `AppendMenu` (the same metacharacters) but after item `afterItem`
/// (0 = first; at or past the last item = appended). The enable bits of the items after the insertion shift up with them.
fn insert_menu_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, after) = (c.args.get(0), c.args.get(2) as u16 as i16);
    h_of(m, "InsertMenuItem", h)?;
    let data = read_pstring_bytes(&m.mem, c.args.get(1))?;
    let mut p = m.menu_view(h)?;
    let added = parse_append(&data);
    let at = (after.max(0) as usize).min(p.items.len());
    let old = p.flags;
    // bit n (1..=31) is item n: the bits above the insertion move up by the number of items added
    let k = added.len() as u32;
    let low_mask = if at >= 31 { 0xFFFF_FFFF } else { (1u32 << (at + 1)) - 1 };
    let mut flags = (old & low_mask) | if k < 32 { (old & !low_mask) << k } else { 0 };
    for (j, (it, dis)) in added.iter().enumerate() {
        p.items.insert(at + j, it.clone());
        let n = (at + j + 1) as u32;
        if n <= 31 { if *dis { flags &= !(1 << n); } else { flags |= 1 << n; } }
    }
    p.flags = flags;
    m.menu_store(h, &p)?;
    done(format!("[menu] InsertMenuItem menu {} +{} after {after} (now {})", p.id, added.len(), p.items.len()))
}

/// `DeleteMenuItem(theMenu, item)`: item `item` (1-based) leaves the menu and the items after it move up; so do their enable bits
/// (bit n of the flags is item n). An item that is not in the menu is left alone, as the book says ("if item is out of range, nothing happens").
fn delete_menu_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, item) = (c.args.get(0), c.args.get(1) as u16 as i16);
    h_of(m, "DeleteMenuItem", h)?;
    let mut p = m.menu_view(h)?;
    if item < 1 || item as usize > p.items.len() { return done(format!("[menu] DeleteMenuItem menu {} #{item}: not an item, nothing deleted (menu has {})", p.id, p.items.len())); }
    let at = item as u32;
    p.items.remove(at as usize - 1);
    // bits 0..at-1 stay (the menu's own bit and the items before), the bits above `at` move down one
    let low = if at >= 32 { 0xFFFF_FFFF } else { (1u32 << at) - 1 };
    let high = if at >= 31 { 0 } else { (p.flags >> (at + 1)) << at };
    p.flags = (p.flags & low) | high;
    m.menu_store(h, &p)?;
    done(format!("[menu] DeleteMenuItem menu {} #{item} (now {})", p.id, p.items.len()))
}

fn add_res_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, ty) = (c.args.get(0), c.args.get(1).to_be_bytes());
    h_of(m, "AddResMenu", h)?;
    let mut p = m.menu_view(h)?;
    let mut added = 0;
    let mut names: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    for n in 1..=m.count_resources(ty, false) {
        let rh = m.get_ind_resource(ty, n, false).map_err(|e| Refusal(format!("AddResMenu: ResErr {e}")))?;
        let Some((_, _, Some(name))) = m.res_info(rh) else { continue };
        if !name.is_empty() { names.push(name); }
    }
    // The System file's own fonts: a `FONT`/`FOND` menu lists the standard families too (no System file exists to enumerate).
    if &ty == b"FONT" || &ty == b"FOND" { names.extend(crate::fontnames::MENU_FAMILIES.iter().map(|n| n.as_bytes().to_vec())); }
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    for name in names {
        p.items.push(Item { text: name, icon: 0, key: 0, mark: 0, style: 0 });
        let k = p.items.len() as u32;
        if k <= 31 { p.flags |= 1 << k; }
        added += 1;
    }
    m.menu_store(h, &p)?;
    done(format!("[menu] AddResMenu menu {} '{}' +{added} (names of the resources of that type)", p.id, mac_text(&ty)))
}

fn insert_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, before) = (c.args.get(0), c.args.get(1) as u16 as i16);
    h_of(m, "InsertMenu", h)?;
    let id = m.menu_view(h)?.id;
    m.menus.by_id.insert(id, h);
    if before != -1 && !m.menus.bar.contains(&id) {
        let at = m.menus.bar.iter().position(|&b| b == before).unwrap_or(m.menus.bar.len());
        m.menus.bar.insert(at, id);
    }
    done(format!("[menu] InsertMenu {id} before {before}: bar {:?}", m.menus.bar))
}

fn delete_menu<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    m.menus.bar.retain(|&b| b != id);
    done(format!("[menu] DeleteMenu {id}"))
}

fn clear_menu_bar<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.menus.bar.clear();
    done("[menu] ClearMenuBar".into())
}

fn draw_menu_bar<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.menus.drawn = true;
    m.sync_menus()?;
    done(format!("[menu] DrawMenuBar: {} menus -> {} windows", m.menus.bar.len(), m.windows.len()))
}

/// Our MenuBar handle (what `GetNewMBar`/`GetMenuBar` return and `SetMenuBar` takes) is the `MBAR` resource layout:
/// `i16 count` then that many `i16` menu ids.
fn get_new_mbar<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rid = c.args.get(0) as u16 as i16;
    let Ok(rh) = m.get_resource(*b"MBAR", rid, false) else { return done(format!("[menu] GetNewMBar {rid}: no MBAR resource")) };
    let raw = m.handle_bytes(rh).unwrap_or_default();
    let n = raw.get(0..2).map(|b| i16::from_be_bytes([b[0], b[1]])).unwrap_or(0).max(0) as usize;
    let ids: Vec<i16> = (0..n).filter_map(|i| raw.get(2 + 2 * i..4 + 2 * i)).map(|b| i16::from_be_bytes([b[0], b[1]])).collect();
    if ids.len() != n { return Err(Refusal(format!("GetNewMBar {rid}: MBAR lists {n} menus but holds {}", ids.len()))); }
    for id in &ids {
        let mh = m.get_resource(*b"MENU", *id, false).map_err(|e| Refusal(format!("GetNewMBar {rid}: no MENU {id} (ResErr {e})")))?;
        m.menus.by_id.insert(*id, mh);
    }
    let out = m.mm.new_handle(&mut m.mem, &mut m.heap, raw.len() as u32, false).map_err(|e| Refusal(format!("GetNewMBar: no memory ({e})")))?;
    let p = m.mm.block(out).map(|b| b.ptr).unwrap_or(0);
    m.mem.write_bytes(p, &raw).map_err(Refusal::mem("MBAR copy"))?;
    Ok(Done { result: out, log: format!("[menu] GetNewMBar {rid}: menus {ids:?}") })
}

fn set_menu_bar<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let raw = m.handle_bytes(h).ok_or_else(|| Refusal(format!("SetMenuBar: {h:#x} is not a handle")))?;
    let n = raw.get(0..2).map(|b| i16::from_be_bytes([b[0], b[1]])).unwrap_or(-1);
    if n < 0 || raw.len() < 2 + 2 * n as usize { return Err(Refusal(format!("SetMenuBar: {h:#x} is not a menu list"))); }
    let ids: Vec<i16> = (0..n as usize).map(|i| i16::from_be_bytes([raw[2 + 2 * i], raw[3 + 2 * i]])).collect();
    if let Some(id) = ids.iter().find(|i| !m.menus.by_id.contains_key(i)) { return Err(Refusal(format!("SetMenuBar: menu {id} is not loaded"))); }
    m.menus.bar = ids.clone();
    done(format!("[menu] SetMenuBar {ids:?}"))
}

fn get_mhandle<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let h = m.menus.by_id.get(&id).copied().unwrap_or(0);
    Ok(Done { result: h, log: format!("[menu] GetMHandle {id} -> {h:#x}") })
}

fn count_mitems<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    h_of(m, "CountMItems", h)?;
    let n = m.menu_view(h)?.items.len() as u32;
    Ok(Done { result: n, log: format!("[menu] CountMItems {h:#x} -> {n}") })
}

pub(crate) fn edit_item<U: GuestUi>(m: &mut Mac<U>, what: &str, h: u32, item: u16, f: impl FnOnce(&mut Parsed, usize)) -> Result<(), Refusal> {
    h_of(m, what, h)?;
    let mut p = m.menu_view(h)?;
    if item as usize > p.items.len() { return Err(Refusal(format!("{what}: menu {} has {} items, not {item}", p.id, p.items.len()))); }
    f(&mut p, item as usize);
    m.menu_store(h, &p)
}

fn check_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, item, on) = (c.args.get(0), c.args.get(1) as u16, c.args.get(2) as u8 != 0);
    if item == 0 { return Err(Refusal("CheckItem: item 0 is not an item".into())); }
    edit_item(m, "CheckItem", h, item, |p, i| p.items[i - 1].mark = if on { CHECK_MARK } else { 0 })?;
    done(format!("[menu] CheckItem {h:#x} #{item} {on}"))
}
fn set_enabled<U: GuestUi>(m: &mut Mac<U>, c: &Call, on: bool) -> Result<Done, Refusal> {
    let (h, item) = (c.args.get(0), c.args.get(1) as u16);
    edit_item(m, c.name, h, item, |p, i| { if i <= 31 { if on { p.flags |= 1 << i } else { p.flags &= !(1 << i) } } })?;
    done(format!("[menu] {} {h:#x} #{item}", c.name))
}
fn enable_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_enabled(m, c, true) }
fn disable_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_enabled(m, c, false) }

fn get_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, item, out) = (c.args.get(0), c.args.get(1) as u16, c.args.get(2));
    h_of(m, "GetItem", h)?;
    let p = m.menu_view(h)?;
    // Past the last item the Menu Manager reads the menu data's zero terminator: an EMPTY string, not a fault (a program
    // counts a menu's items by calling GetItem until the string is empty).
    let text = p.items.get((item as usize).wrapping_sub(1)).map(|i| i.text.clone()).unwrap_or_default();
    m.mem.write_u8(out, text.len() as u8).map_err(Refusal::mem("GetItem"))?;
    m.mem.write_bytes(out + 1, &text).map_err(Refusal::mem("GetItem"))?;
    done(format!("[menu] GetItem {h:#x} #{item} -> \"{}\"", mac_text(&text)))
}

fn set_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, item, src) = (c.args.get(0), c.args.get(1) as u16, c.args.get(2));
    let text = read_pstring_bytes(&m.mem, src)?;
    if item == 0 { return Err(Refusal("SetItem: item 0 is not an item".into())); }
    edit_item(m, "SetItem", h, item, |p, i| p.items[i - 1].text = text.clone())?;
    done(format!("[menu] SetItem {h:#x} #{item} \"{}\"", read_pstring(&m.mem, src).unwrap_or_default()))
}

/// `MenuSelect(startPt): LONGINT`: instead of tracking the mouse, return the oldest native menu command as
/// `menuID << 16 | item`, or 0 (no choice) when none is queued.
fn menu_select<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let r = m.ev.choices.pop_front().unwrap_or(0);
    Ok(Done { result: r, log: format!("[menu] MenuSelect -> menu {} item {}", (r >> 16) as i16, r & 0xFFFF) })
}


#[cfg(test)]
mod tests {
    use super::*;
    use guestui_abi::RecordingGuestUi;

    fn items(s: &str) -> Vec<(Item, bool)> { parse_append(s.as_bytes()) }

    #[test]
    fn append_metacharacters() {
        let v = items("New/N;Open.../O;(-;Bold<B<I!\u{12};Hello^2");
        let t: Vec<(String, u8, u8, u8, u8, bool)> = v.iter().map(|(i, d)| (mac_text(&i.text), i.icon, i.key, i.mark, i.style, *d)).collect();
        assert_eq!(t[0], ("New".into(), 0, b'N', 0, 0, false));
        assert_eq!(t[1], ("Open...".into(), 0, b'O', 0, 0, false));
        assert_eq!(t[2], ("-".into(), 0, 0, 0, 0, true), "( disables");
        assert_eq!(t[3].0, "Bold");
        assert_eq!((t[3].3, t[3].4), (0x12, 1 | 2), "! sets the mark char, <B and <I the style bits");
        assert_eq!((t[4].0.as_str(), t[4].1), ("Hello", 2), "^n is the icon number");
        assert_eq!(v.len(), 5);
        // NEGATIVE: a plain string has no metacharacter effect, and an empty item is dropped
        let plain = items("Plain text;;");
        assert_eq!(plain.len(), 1);
        assert_eq!((plain[0].0.key, plain[0].0.mark, plain[0].1), (0, 0, false));
    }

    #[test]
    fn block_round_trip_and_malformed_blocks() {
        let p = Parsed { id: 7, flags: 0b110, title: b"T".to_vec(), items: items("A;B").into_iter().map(|(i, _)| i).collect() };
        let bytes = encode_block(&p, 10, 20, 0);
        assert_eq!(parse_block(&bytes).unwrap(), p);
        assert_eq!(&bytes[..6], &[0, 7, 0, 10, 0, 20], "id, width, height");
        // NEGATIVE: every truncation is a named refusal, never a panic
        for cut in 0..bytes.len() {
            let r = parse_block(&bytes[..cut]);
            assert!(r.is_err() || cut == bytes.len(), "cut {cut}");
        }
        assert!(parse_block(&bytes[..bytes.len() - 1]).unwrap_err().0.contains("no terminating zero"));
    }

    #[test]
    fn model_has_checks_disabled_items_and_hierarchical_submenus() {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        let sub = make_menu(&mut m, 200, b"Sub".to_vec()).unwrap();
        let mut p = m.menu_view(sub).unwrap();
        p.items = items("One;Two").into_iter().map(|(i, _)| i).collect();
        m.menu_store(sub, &p).unwrap();
        let top = make_menu(&mut m, 201, b"Top".to_vec()).unwrap();
        let mut p = m.menu_view(top).unwrap();
        p.items = items("Plain/P;Checked!\u{12};Nested").into_iter().map(|(i, _)| i).collect();
        p.items[2].key = KEY_HIERARCHICAL;
        p.items[2].mark = 200;
        p.flags &= !(1 << 1); // disable item 1
        m.menu_store(top, &p).unwrap();
        m.menus.bar = vec![201];
        let strip = m.menu_strip().unwrap();
        let it = &strip.menus[0].items;
        assert_eq!((it[0].enabled, it[0].shortcut, it[0].check), (false, Some('P'), None));
        assert_eq!((it[1].enabled, it[1].check), (true, Some(true)));
        assert_eq!(it[2].sub_items.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), vec!["One", "Two"]);
        assert_eq!(it[2].shortcut, None, "the $1B key byte is not a shortcut");
        assert_eq!(it[2].sub_items[1].token, CallbackToken(200 << 16 | 2), "submenu items carry their own menu id");
    }

    #[test]
    fn editing_a_menu_keeps_its_handle_and_dispose_forgets_it() {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        let h = make_menu(&mut m, 5, b"M".to_vec()).unwrap();
        let mut p = m.menu_view(h).unwrap();
        p.items = items("x").into_iter().map(|(i, _)| i).collect();
        m.menu_store(h, &p).unwrap();
        assert_eq!(m.menu_view(h).unwrap().items.len(), 1);
        m.menus.bar = vec![5];
        m.dispose_handle(h).unwrap();
        assert!(m.menus.by_id.is_empty() && m.menus.bar.is_empty(), "the menu is gone from the id table and the bar");
        assert!(m.menu_view(h).is_err(), "NEGATIVE: a disposed MenuHandle is not a menu");
        assert!(m.menus.choice_for_token(&m.mem, &m.mm, 5 << 16 | 1).is_err());
    }
}
