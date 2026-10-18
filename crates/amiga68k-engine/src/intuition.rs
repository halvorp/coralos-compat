//! intuition.library handlers that translate onto [`GuestUi`].
//! Menus (windows are window.rs), and the window calls a program makes that the native window manager owns: they are
//! accepted and logged (ActivateWindow, WindowToFront / Back, RefreshGadgets / RefreshGList: the native side stacks,
//! focuses and draws), or forwarded (SetWindowTitles). EasyRequestArgs is deliberately NOT bridged: GuestUi has no
//! requester call, so it stays the named `intuition.library/EasyRequestArgs: unimplemented` refusal.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, menu, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_SET_MENU_STRIP: i32 = -264;
pub const LVO_CLEAR_MENU_STRIP: i32 = -54;
pub const LVO_REFRESH_GADGETS: i32 = -222;
pub const LVO_SET_WINDOW_TITLES: i32 = -276;
pub const LVO_WINDOW_TO_BACK: i32 = -306;
pub const LVO_WINDOW_TO_FRONT: i32 = -312;
pub const LVO_REFRESH_GLIST: i32 = -432;
pub const LVO_ACTIVATE_WINDOW: i32 = -450;
const WD_TITLE: u32 = 32;


/// a0 = window, a1 = first Menu. d0 = TRUE on success.
pub fn set_menu_strip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (h, first) = (cpu.areg(0), cpu.areg(1));
    let win = *a.windows.get(&h).ok_or_else(|| Refusal(format!("SetMenuStrip: {h:#x} is not an open window")))?;
    let model = menu::decode_menu_strip(&a.mem, first)?;
    let n = model.menus.len();
    a.ui.set_menu_strip(win, model).map_err(Refusal::ui)?;
    a.menu_strips.insert(h, first);
    cpu.set_d0(1);
    Ok(format!("[intuition] SetMenuStrip handle={h:#x} menus={n}"))
}

/// a0 = window.
pub fn clear_menu_strip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    let win = *a.windows.get(&h).ok_or_else(|| Refusal(format!("ClearMenuStrip: {h:#x} is not an open window")))?;
    a.ui.clear_menu_strip(win).map_err(Refusal::ui)?;
    a.menu_strips.remove(&h);
    Ok(format!("[intuition] ClearMenuStrip handle={h:#x}"))
}

fn open_window<U: GuestUi>(a: &Amiga<U>, what: &str, h: u32) -> Result<guestui_abi::UiObjectRef, Refusal> {
    a.windows.get(&h).copied().ok_or_else(|| Refusal(format!("{what}: {h:#x} is not an open window")))
}

macro_rules! accepted {
    ($name:ident, $what:literal) => {
        /// a0 = window. The native window manager owns stacking and focus: accepted and logged.
        pub fn $name<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
            let h = cpu.areg(0);
            open_window(a, $what, h)?;
            Ok(format!("[intuition] {} {h:#x} (native window manager's)", $what))
        }
    };
}
accepted!(activate_window, "ActivateWindow");
accepted!(window_to_front, "WindowToFront");
accepted!(window_to_back, "WindowToBack");

/// SetWindowTitles(a0 = window, a1 = window title, a2 = screen title). (UBYTE *)-1 leaves a title as it is; NULL clears
/// the window title. The screen title belongs to the one native screen and is not carried.
pub fn set_window_titles<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (h, title, screen_title) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let native = open_window(a, "SetWindowTitles", h)?;
    let mut shown = String::from("unchanged");
    if title != u32::MAX {
        let text = if title == 0 { String::new() } else { a.mem.read_cstr(title).map_err(Refusal::mem("SetWindowTitles window title"))? };
        a.ui.set_window_title(native, text.clone()).map_err(Refusal::ui)?;
        a.mem.write_u32(h + WD_TITLE, title).map_err(Refusal::mem("Window.Title"))?;
        shown = format!("{text:?}");
    }
    let screen_note = if screen_title == u32::MAX { "" } else { " (screen title not carried)" };
    Ok(format!("[intuition] SetWindowTitles {h:#x} title={shown}{screen_note}"))
}
