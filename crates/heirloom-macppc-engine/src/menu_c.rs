//! Menu Manager Carbon accessors over the engine's native menu path (the `MENU` blocks and the bar in `heirloom-mac68k-engine/src/menu.rs`, which becomes the host's menu strip):
//! `GetMenuTitle`, `SetMenuTitle`, `GetMenuID`, `IsMenuItemEnabled`, `Get/SetMenuItemProperty`, `MenuKey`, `IsMenuBarVisible`, `ShowMenuBar`. `EnableMenuItem` / `DisableMenuItem` are
//! the classic `EnableItem` / `DisableItem` handlers under their Carbon names (symbols.rs). Item 0 means the whole menu (enable bit 0 of the flags word).
//! Specs (clean room, /data/build/macos9-specs): CarbonLib/GetMenuTitle, CarbonLib/SetMenuTitle, MenusLib/HideMenuBar + ShowMenuBar.
//! Named limits: item properties are keyed by item NUMBER, so inserting or deleting items above one does not move its properties; `MenuKey` does not hilite the title (the host owns the
//! strip). `HideMenuBar` / `ShowMenuBar` follow the spec: `MBarHeight` (0x0BAA) goes to 0 and back, the host's strip is emptied/refilled (an empty `set_menu_strip`), and a repeated call
//! changes nothing. Left unbound, stopping BY NAME: `PopUpMenuSelect` (there is no native pop-up request on the seam).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_ppc::Cpu;

#[derive(Default)]
pub struct MenuState { props: BTreeMap<(u32, u16, u32, u32), Vec<u8>> }

const PROPERTY_NOT_FOUND: i32 = -5604;
const PROPERTY_INVALID: i32 = -5603;
const MENU_ITEM_NOT_FOUND: i32 = -50; // paramErr: the Menu Manager reports a bad item index as paramErr here
const KEY_HIERARCHICAL: u8 = 0x1B;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MnFn { GetMenuTitle, SetMenuTitle, GetMenuID, IsMenuItemEnabled, GetMenuItemProperty, SetMenuItemProperty, MenuKey, IsMenuBarVisible, HideMenuBar, ShowMenuBar }

impl MnFn {
    pub fn by_name(name: &str) -> Option<MnFn> {
        use MnFn::*;
        Some(match name {
            "GetMenuTitle" => GetMenuTitle, "SetMenuTitle" => SetMenuTitle, "GetMenuID" => GetMenuID, "IsMenuItemEnabled" => IsMenuItemEnabled,
            "GetMenuItemProperty" => GetMenuItemProperty, "SetMenuItemProperty" => SetMenuItemProperty, "MenuKey" => MenuKey,
            "IsMenuBarVisible" => IsMenuBarVisible, "HideMenuBar" => HideMenuBar, "ShowMenuBar" => ShowMenuBar,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn mn_call(&mut self, f: MnFn, cpu: &mut Cpu) -> Result<(), String> {
        use MnFn::*;
        let name = format!("{f:?}");
        let n = name.as_str();
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let r: i32 = match f {
            // StringPtr GetMenuTitle(MenuRef, Str255 title) returns `title`; NULL menu: nothing written; NULL title: nothing written (spec). A title that is the icon-suite encoding
            // (05 01 + handle) is turned into the EMPTY string after the copy.
            GetMenuTitle => {
                if a0 == 0 || a1 == 0 { a1 as i32 } else {
                    let p = self.mac.menu_view(a0).map_err(|e| format!("{n}: {}", e.0))?;
                    let t = &p.title[..p.title.len().min(255)];
                    let mut b = alloc::vec![t.len() as u8]; b.extend_from_slice(t);
                    self.put_bytes(a1, &b, n)?;
                    if b.len() >= 2 && b[0] == 5 && b[1] == 1 { self.put_bytes(a1, &[0], n)?; }
                    a1 as i32
                }
            }
            // OSStatus SetMenuTitle(MenuRef, ConstStr255Param): NULL menu -5623 (menuInvalidErr), NULL title -50, else noErr; the strip is invalidated only when the menu is installed (spec).
            // The icon-suite title (05 01 + handle, whose suite the menu may own) is not modelled: refused by name.
            SetMenuTitle => {
                if a0 == 0 { -5623 } else if a1 == 0 { -50 } else {
                    let mut p = self.mac.menu_view(a0).map_err(|e| format!("{n}: {}", e.0))?;
                    if p.title.len() == 5 && p.title[0] == 1 { return Err(format!("{n}: the menu's title is an icon suite (05 01 + handle); replacing it disposes the suite under the menu-attribute rules, which are not modelled")); }
                    let len = self.mac.mem.read_u8(a1).map_err(|_| format!("{n}: the title at {a1:#x} leaves guest RAM"))?;
                    p.title = self.get_bytes(a1 + 1, u32::from(len), n)?;
                    self.mac.menu_store(a0, &p).map_err(|e| format!("{n}: {}", e.0))?;
                    if self.mac.menus.drawn && self.mac.menus.by_id.get(&p.id) == Some(&a0) { self.mac.sync_menus().map_err(|e| format!("{n}: {}", e.0))?; }
                    0
                }
            }
            GetMenuID => i32::from(self.mac.menu_view(a0).map_err(|e| format!("{n}: {}", e.0))?.id),
            IsMenuItemEnabled => {
                let p = self.mac.menu_view(a0).map_err(|e| format!("{n}: {}", e.0))?;
                let item = a1 as u16;
                i32::from(item as usize <= p.items.len() && (item > 31 || p.flags & (1 << item) != 0))
            }
            // GetMenuItemProperty(menu, item, creator, tag, bufferSize, UInt32 *actualSize, void *buffer)
            GetMenuItemProperty => {
                let (size, actual, buf) = (arg(self, cpu, 4)?, arg(self, cpu, 5)?, arg(self, cpu, 6)?);
                match self.rt_mn.props.get(&(a0, a1 as u16, a2, a3)).cloned() {
                    _ if self.mn_check(a0, a1).is_err() => MENU_ITEM_NOT_FOUND,
                    None => PROPERTY_NOT_FOUND,
                    Some(v) => {
                        if actual != 0 { self.mac.mem.write_u32(actual, v.len() as u32).map_err(|_| format!("{n}: actualSize leaves guest RAM"))?; }
                        if v.len() as u32 > size { PROPERTY_INVALID } else { self.put_bytes(buf, &v, n)?; 0 }
                    }
                }
            }
            SetMenuItemProperty => {
                let (size, buf) = (arg(self, cpu, 4)?, arg(self, cpu, 5)?);
                if let Err(c) = self.mn_check(a0, a1) { c }
                else { let v = self.get_bytes(buf, size, n)?; self.rt_mn.props.insert((a0, a1 as u16, a2, a3), v); 0 }
            }
            // MenuKey(charCode): the item of an ENABLED menu in the bar whose command-key equivalent is that character (case-insensitive); (menuID << 16) | item, or 0
            MenuKey => {
                let ch = (a0 as u8).to_ascii_uppercase();
                let mut hit = 0u32;
                'bar: for id in self.mac.menus.bar.clone() {
                    let Some(&h) = self.mac.menus.by_id.get(&id) else { continue };
                    let p = self.mac.menu_view(h).map_err(|e| format!("{n}: {}", e.0))?;
                    if p.flags & 1 == 0 { continue; }
                    for (i, it) in p.items.iter().enumerate() {
                        let num = i as u32 + 1;
                        if it.key != KEY_HIERARCHICAL && it.key >= 0x20 && it.key.to_ascii_uppercase() == ch && (num > 31 || p.flags & (1 << num) != 0) {
                            hit = u32::from(id as u16) << 16 | num;
                            break 'bar;
                        }
                    }
                }
                hit as i32
            }
            IsMenuBarVisible => i32::from(!self.mac.menus.hidden),
            // spec: Hide with the bar visible saves MBarHeight, sets it 0, clears the visible flag and redraws; Hide again does nothing. Show with it hidden restores the saved height
            // unless the program left a non-zero MBarHeight, sets the flag and redraws; Show again does nothing.
            HideMenuBar => {
                if !self.mac.menus.hidden {
                    self.mac.menus.saved_bar_height = self.mac.mem.read_u16(heirloom_mac68k_engine::lowmem::MBAR_HEIGHT).unwrap_or(0);
                    self.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::MBAR_HEIGHT, 0).map_err(|_| format!("{n}: MBarHeight leaves guest RAM"))?;
                    self.mac.menus.hidden = true;
                    self.mac.sync_menus().map_err(|e| format!("{n}: {}", e.0))?;
                }
                0
            }
            ShowMenuBar => {
                if self.mac.menus.hidden {
                    if self.mac.mem.read_u16(heirloom_mac68k_engine::lowmem::MBAR_HEIGHT).unwrap_or(0) == 0 {
                        let h = self.mac.menus.saved_bar_height;
                        self.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::MBAR_HEIGHT, h).map_err(|_| format!("{n}: MBarHeight leaves guest RAM"))?;
                    }
                    self.mac.menus.hidden = false;
                    self.mac.sync_menus().map_err(|e| format!("{n}: {}", e.0))?;
                }
                0
            }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    /// The menu must exist and the item be 0..=count; the Menu Manager's property calls report a bad item as paramErr.
    fn mn_check(&self, menu: u32, item: u32) -> Result<(), i32> {
        match self.mac.menu_view(menu) {
            Ok(p) if (item as usize) <= p.items.len() => Ok(()),
            Ok(_) => Err(MENU_ITEM_NOT_FOUND),
            Err(_) => Err(MENU_ITEM_NOT_FOUND)
        }
    }
}
