//! Decode Intuition Menu / MenuItem / IntuiText chains into a [`MenuModel`].
//! Offsets (NDK-style): Menu = Next 0, Left 4, Top 6, W 8, H 10, Flags 12(w), Name 14, FirstItem 18.
//! MenuItem = Next 0, ..., Flags 12(w), MutualExclude 14, ItemFill 18, SelectFill 22,
//! Command 26(b), SubItem 28. IntuiText = ..., IText 12(ptr).
use crate::Refusal;
use guestui_abi::{CallbackToken, Menu, MenuItem, MenuModel};
use retro_guest_mem::GuestMem;

const MENUENABLED: u16 = 0x0001;
const CHECKIT: u16 = 0x0001;
const ITEMTEXT: u16 = 0x0002;
const COMMSEQ: u16 = 0x0004;
const ITEMENABLED: u16 = 0x0010;
const CHECKED: u16 = 0x0100;
const MAX_MENUS: usize = 31;
const MAX_ITEMS: usize = 63;
const MAX_SUBS: usize = 31;

/// Intuition's FULLMENUNUM packing: menu 5 bits, item 6 bits, sub 5 bits.
pub fn menu_token(menu: usize, item: Option<usize>, sub: Option<usize>) -> CallbackToken {
    let i = item.map_or(0x3F, |i| i as u64);
    let s = sub.map_or(0x1F, |s| s as u64);
    CallbackToken(menu as u64 | (i << 5) | (s << 11))
}

fn item(mem: &GuestMem, addr: u32, m: usize, i: usize, sub: Option<usize>) -> Result<MenuItem, Refusal> {
    let e = Refusal::mem("MenuItem");
    let flags = mem.read_u16(addr.wrapping_add(12)).map_err(&e)?;
    let fill = mem.read_u32(addr.wrapping_add(18)).map_err(&e)?;
    let label = if flags & ITEMTEXT != 0 {
        let itext = mem.read_u32(fill.wrapping_add(12)).map_err(Refusal::mem("MenuItem.ItemFill.IText"))?;
        mem.read_cstr(itext).map_err(Refusal::mem("MenuItem.ItemFill.IText"))?
    } else {
        "-".to_string() // an Image fill (separator bar); the model has no separator kind yet
    };
    let shortcut = if flags & COMMSEQ != 0 { Some(mem.read_u8(addr.wrapping_add(26)).map_err(&e)? as char) } else { None };
    let mut sub_items = vec![];
    if sub.is_none() {
        let mut p = mem.read_u32(addr.wrapping_add(28)).map_err(&e)?;
        while p != 0 {
            if sub_items.len() >= MAX_SUBS { return Err(Refusal("MenuItem.SubItem: chain longer than 31 (cycle?)".into())); }
            let n = sub_items.len();
            sub_items.push(item(mem, p, m, i, Some(n))?);
            p = mem.read_u32(p).map_err(&e)?;
        }
    }
    Ok(MenuItem {
        label,
        enabled: flags & ITEMENABLED != 0,
        shortcut,
        check: (flags & CHECKIT != 0).then_some(flags & CHECKED != 0),
        sub_items,
        token: menu_token(m, Some(i), sub),
    })
}

pub fn decode_menu_strip(mem: &GuestMem, first: u32) -> Result<MenuModel, Refusal> {
    let e = Refusal::mem("Menu");
    let (mut menus, mut p) = (vec![], first);
    while p != 0 {
        if menus.len() >= MAX_MENUS { return Err(Refusal("Menu: chain longer than 31 (cycle?)".into())); }
        let m = menus.len();
        let flags = mem.read_u16(p.wrapping_add(12)).map_err(&e)?;
        let name = mem.read_u32(p.wrapping_add(14)).map_err(&e)?;
        let title = mem.read_cstr(name).map_err(Refusal::mem("Menu.MenuName"))?;
        let (mut items, mut ip) = (vec![], mem.read_u32(p.wrapping_add(18)).map_err(&e)?);
        while ip != 0 {
            if items.len() >= MAX_ITEMS { return Err(Refusal("Menu.FirstItem: chain longer than 63 (cycle?)".into())); }
            let i = items.len();
            items.push(item(mem, ip, m, i, None)?);
            ip = mem.read_u32(ip).map_err(Refusal::mem("MenuItem.NextItem"))?;
        }
        menus.push(Menu { title, enabled: flags & MENUENABLED != 0, items });
        p = mem.read_u32(p).map_err(&e)?;
    }
    Ok(MenuModel { menus })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fixture::build_hunk, Amiga};
    use guestui_abi::RecordingGuestUi;

    fn loaded() -> (Amiga<RecordingGuestUi>, u32) {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        let r = a.run_hunk(&build_hunk(None), 100_000).unwrap();
        let data = r.loaded.bases[1];
        let menu = (data..data + r.loaded.sizes[1]).step_by(4)
            .find(|p| a.mem.read_u32(p + 14).ok().and_then(|n| a.mem.read_cstr(n).ok()).as_deref() == Some("Project"))
            .expect("Menu struct in fixture data");
        (a, menu)
    }

    #[test]
    fn decodes_fixture_menu() {
        let (a, menu) = loaded();
        let m = decode_menu_strip(&a.mem, menu).unwrap();
        assert_eq!(m.menus[0].items.len(), 2);
        assert_eq!(m.menus[0].items[1].shortcut, Some('Q'));
    }

    #[test]
    fn item_chain_cycle_is_refused_by_name() {
        let (mut a, menu) = loaded();
        let first = a.mem.read_u32(menu + 18).unwrap();
        a.mem.write_u32(first, first).unwrap(); // NextItem -> itself
        assert_eq!(decode_menu_strip(&a.mem, menu).unwrap_err().0, "Menu.FirstItem: chain longer than 63 (cycle?)");
    }

    #[test]
    fn wild_menu_pointer_is_refused() {
        let (a, _) = loaded();
        assert!(decode_menu_strip(&a.mem, 0xFFFF_FF00).unwrap_err().0.starts_with("Menu: pointer 0xffffff0c"));
    }
}
