//! Menustrip / Menu / Menuitem objects -> a GuestUi MenuModel; a menu command back to its item's `MUIA_UserData`.
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use crate::boopsi::Objects;
use crate::machine::Amiga;
use guestui_abi::{CallbackToken, GuestUi, Menu, MenuItem, MenuModel};

fn item(o: &Objects, i: u32) -> MenuItem {
    let checkit = o.flag(i, MUIA_Menuitem_Checkit);
    MenuItem { label: o.text(i, MUIA_Menuitem_Title).into(), enabled: o.num(i, MUIA_Menuitem_Enabled) != Some(0),
        shortcut: o.text(i, MUIA_Menuitem_Shortcut).chars().next(), check: checkit.then(|| o.flag(i, MUIA_Menuitem_Checked)),
        sub_items: o.children(i).iter().map(|&c| item(o, c)).collect(), token: CallbackToken(i as u64) }
}

pub fn model(o: &Objects, strip: u32) -> MenuModel {
    MenuModel { menus: o.children(strip).iter().map(|&m| {
        let items: Vec<MenuItem> = o.children(m).iter().map(|&i| item(o, i)).collect();
        Menu { title: o.text(m, MUIA_Menu_Title).into(), enabled: o.num(m, MUIA_Menu_Enabled) != Some(0), items }
    }).collect() }
}

/// The menu item `token` names -> its `MUIA_UserData` (what `MUIA_Application_MenuAction` is set to), if it is one.
pub fn item_action<U: GuestUi>(a: &Amiga<U>, token: u32) -> Option<u32> {
    let it = a.objects.map.get(&token).filter(|o| o.kind == Kind::Menuitem)?;
    Some(it.attrs.get(&MUIA_UserData).copied().unwrap_or(0))
}
