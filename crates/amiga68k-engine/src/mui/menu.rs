//! Menustrip / Menu / Menuitem objects -> a GuestUi MenuModel; a menu command back to its application.
use alloc::string::String;
use alloc::vec::Vec;
use super::{ids::*, notify::root};
use crate::boopsi::{Kind, Objects};
use crate::machine::Amiga;
use guestui_abi::{CallbackToken, GuestUi, Menu, MenuItem, MenuModel};

fn text(objs: &Objects, o: u32, attr: u32) -> String { objs.map.get(&o).and_then(|x| x.strs.get(&attr).cloned()).unwrap_or_default() }

fn item(objs: &Objects, o: u32) -> MenuItem {
    let sub = objs.map.get(&o).map(|x| x.children.clone()).unwrap_or_default();
    MenuItem { label: text(objs, o, MUIA_MENUITEM_TITLE), enabled: true, shortcut: text(objs, o, MUIA_MENUITEM_SHORTCUT).chars().next(), check: None,
        sub_items: sub.into_iter().map(|c| item(objs, c)).collect(), token: CallbackToken(o as u64) }
}

pub fn model(objs: &Objects, strip: u32) -> MenuModel {
    let menus = objs.map.get(&strip).map(|s| s.children.clone()).unwrap_or_default();
    MenuModel { menus: menus.into_iter().map(|m| {
        let items: Vec<MenuItem> = objs.map.get(&m).map(|x| x.children.clone()).unwrap_or_default().into_iter().map(|i| item(objs, i)).collect();
        Menu { title: text(objs, m, MUIA_MENU_TITLE), enabled: true, items }
    }).collect() }
}

/// The menu item `token` names -> (its application, its MUIA_UserData).
pub fn item_action<U: GuestUi>(a: &Amiga<U>, token: u32) -> Option<(u32, u32)> {
    let it = a.objects.map.get(&token).filter(|o| o.kind == Kind::Menuitem)?;
    let app = root(a, token);
    (a.objects.map.get(&app)?.kind == Kind::Application).then(|| (app, it.attrs.get(&MUIA_USER_DATA).copied().unwrap_or(0)))
}
