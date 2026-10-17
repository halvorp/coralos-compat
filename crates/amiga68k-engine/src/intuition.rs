//! intuition.library handlers that translate onto [`GuestUi`].
//! Menus (windows are window.rs).
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, menu, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_SET_MENU_STRIP: i32 = -264;
pub const LVO_CLEAR_MENU_STRIP: i32 = -54;


/// a0 = window, a1 = first Menu. d0 = TRUE on success.
pub fn set_menu_strip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (h, first) = (cpu.areg(0), cpu.areg(1));
    let win = *a.windows.get(&h).ok_or_else(|| Refusal(format!("SetMenuStrip: {h:#x} is not an open window")))?;
    let model = menu::decode_menu_strip(&a.mem, first)?;
    let n = model.menus.len();
    a.ui.set_menu_strip(win, model).map_err(Refusal::ui)?;
    cpu.set_d0(1);
    Ok(format!("[intuition] SetMenuStrip handle={h:#x} menus={n}"))
}

/// a0 = window.
pub fn clear_menu_strip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    let win = *a.windows.get(&h).ok_or_else(|| Refusal(format!("ClearMenuStrip: {h:#x} is not an open window")))?;
    a.ui.clear_menu_strip(win).map_err(Refusal::ui)?;
    Ok(format!("[intuition] ClearMenuStrip handle={h:#x}"))
}
