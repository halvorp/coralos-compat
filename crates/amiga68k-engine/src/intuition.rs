//! intuition.library handlers that translate onto [`GuestUi`].
//! NewWindow = Left 0, Top 2, Width 4, Height 6 (words), Flags 14 (long), Title 26 (ptr).
use crate::{machine::Amiga, menu, Refusal};
use guestui_abi::{CallbackToken, GuestRect, GuestUi, WindowModel, WindowRole};
use retro_m68k::Cpu;

pub const LVO_OPEN_WINDOW: i32 = -204;
pub const LVO_CLOSE_WINDOW: i32 = -72;
pub const LVO_SET_MENU_STRIP: i32 = -264;
pub const LVO_CLEAR_MENU_STRIP: i32 = -54;

const WFLG_SIZEGADGET: u32 = 0x1;
const WFLG_CLOSEGADGET: u32 = 0x8;

/// a0 = NewWindow. Returns an opaque window handle in d0 (NULL on refusal). The handle is a
/// token, not a guest `struct Window` (programs reading Window fields are not supported yet).
pub fn open_window<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let nw = cpu.areg(0);
    let e = Refusal::mem("NewWindow");
    let at = |off: u32| nw.wrapping_add(off);
    let (l, t, w, h) = (a.mem.read_i16(at(0)).map_err(&e)?, a.mem.read_i16(at(2)).map_err(&e)?,
        a.mem.read_i16(at(4)).map_err(&e)?, a.mem.read_i16(at(6)).map_err(&e)?);
    let flags = a.mem.read_u32(at(14)).map_err(&e)?;
    let title_ptr = a.mem.read_u32(at(26)).map_err(&e)?;
    let title = if title_ptr == 0 { String::new() } else { a.mem.read_cstr(title_ptr).map_err(Refusal::mem("NewWindow.Title"))? };
    let handle = a.alloc_handle();
    let model = WindowModel {
        title: title.clone(),
        rect: GuestRect { left: l.into(), top: t.into(), width: w.into(), height: h.into() },
        role: WindowRole::Document,
        closable: flags & WFLG_CLOSEGADGET != 0,
        resizable: flags & WFLG_SIZEGADGET != 0,
        token: CallbackToken(handle as u64),
    };
    let win = a.ui.open_window(model).map_err(Refusal::ui)?;
    a.windows.insert(handle, win);
    cpu.set_d0(handle);
    Ok(format!("[intuition] OpenWindow left={l} top={t} w={w} h={h} title=\"{title}\" -> handle {handle:#x}"))
}

/// a0 = window handle.
pub fn close_window<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    let win = a.windows.remove(&h).ok_or_else(|| Refusal(format!("CloseWindow: {h:#x} is not an open window")))?;
    a.ui.close_window(win).map_err(Refusal::ui)?;
    Ok(format!("[intuition] CloseWindow handle={h:#x}"))
}

/// a0 = window handle, a1 = first Menu. d0 = TRUE on success.
pub fn set_menu_strip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (h, first) = (cpu.areg(0), cpu.areg(1));
    let win = *a.windows.get(&h).ok_or_else(|| Refusal(format!("SetMenuStrip: {h:#x} is not an open window")))?;
    let model = menu::decode_menu_strip(&a.mem, first)?;
    let n = model.menus.len();
    a.ui.set_menu_strip(win, model).map_err(Refusal::ui)?;
    cpu.set_d0(1);
    Ok(format!("[intuition] SetMenuStrip handle={h:#x} menus={n}"))
}

/// a0 = window handle.
pub fn clear_menu_strip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    let win = *a.windows.get(&h).ok_or_else(|| Refusal(format!("ClearMenuStrip: {h:#x} is not an open window")))?;
    a.ui.clear_menu_strip(win).map_err(Refusal::ui)?;
    Ok(format!("[intuition] ClearMenuStrip handle={h:#x}"))
}
