//! intuition.library handlers that translate onto [`GuestUi`].
//! Menus (windows are window.rs), and the window calls a program makes that the native window manager owns: they are
//! accepted and logged (ActivateWindow, WindowToFront / Back, RefreshGadgets / RefreshGList: the native side stacks,
//! focuses and draws), or forwarded (SetWindowTitles). The requesters (EasyRequestArgs, AutoRequest, BuildSysRequest ...) are
//! requester.rs: one generic `GuestUi::request`, native alerts.
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
        // A native window always has a name: a title the guest cleared is the program's own name (see `Amiga::start_hunk_as`), as at open.
        let shown_title = if text.is_empty() { a.gfx.program_name.clone() } else { text.clone() };
        a.ui.set_window_title(native, shown_title).map_err(Refusal::ui)?;
        a.mem.write_u32(h + WD_TITLE, title).map_err(Refusal::mem("Window.Title"))?;
        shown = format!("{text:?}");
    }
    let screen_note = if screen_title == u32::MAX { "" } else { " (screen title not carried)" };
    Ok(format!("[intuition] SetWindowTitles {h:#x} title={shown}{screen_note}"))
}

/// WindowLimits(a0 = window, d0 = minWidth, d1 = minHeight, d2 = maxWidth, d3 = maxHeight) -> TRUE. A limit of 0 leaves that
/// one as it is; -1 means "no limit" for the maxima (stored 0xFFFF). The values land in Window.MinWidth 16, MinHeight 18,
/// MaxWidth 20, MaxHeight 22 (m68k gcc -mno-align-int over AROS intuition/intuition.h). The native window manager owns the
/// size, so a window already outside the new limits is not resized (logged).
pub fn window_limits<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    open_window(a, "WindowLimits", h)?;
    let m = Refusal::mem("Window limits");
    for (i, off) in [16u32, 18, 20, 22].into_iter().enumerate() {
        let v = cpu.dreg(i as u32) as i32;
        if v != 0 { a.mem.write_u16(h + off, if v < 0 { 0xFFFF } else { v.min(0xFFFF) as u16 }).map_err(&m)?; }
    }
    cpu.set_d0(1);
    Ok(format!("[intuition] WindowLimits {h:#x} min {}x{} max {}x{} (the native window manager owns the size)", cpu.dreg(0) as i32, cpu.dreg(1) as i32, cpu.dreg(2) as i32, cpu.dreg(3) as i32))
}

/// ChangeWindowBox(a0 = window, d0 = left, d1 = top, d2 = width, d3 = height): moves and resizes. A new size resizes the
/// window's surface (graphics::resize_window: the canvas keeps the pixels that fit, the whole of it is damaged) and updates the
/// Window struct; the position is written to LeftEdge/TopEdge. GuestUi has no move/resize call, so the NATIVE window keeps its
/// place and shows the new canvas size through the next present_content frame (logged). No NEWSIZE message is sent.
pub fn change_window_box<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    open_window(a, "ChangeWindowBox", h)?;
    let (l, t, w, hh) = (cpu.dreg(0) as i16, cpu.dreg(1) as i16, cpu.dreg(2) as i16, cpu.dreg(3) as i16);
    let m = Refusal::mem("Window box");
    let (cw, ch) = (a.mem.read_u16(h + 8).map_err(&m)? as i16, a.mem.read_u16(h + 10).map_err(&m)? as i16);
    if w <= 0 || hh <= 0 { return Err(Refusal(format!("ChangeWindowBox {h:#x}: size {w}x{hh} is not positive"))); }
    for (off, v) in [(4u32, l), (6, t), (8, w), (10, hh)] { a.mem.write_u16(h + off, v as u16).map_err(&m)?; }
    let note = if (w, hh) != (cw, ch) { let (iw, ih) = crate::graphics::resize_window(a, h, (w as i32, hh as i32))?; format!("; canvas now {iw}x{ih}") } else { String::new() };
    Ok(format!("[intuition] ChangeWindowBox {h:#x} -> {l},{t} {w}x{hh} (was {cw}x{ch}{note}; the native window is not moved)"))
}

/// WAC_SENDIDCMPCLOSE (compiler/include/intuition/extensions.h:65).
const WAC_SENDIDCMPCLOSE: u32 = 3;

/// WindowAction(a0 = window, d0 = action, a1 = tags). rom/intuition/windowaction.c:51-62 implements ONE action, WAC_SENDIDCMPCLOSE:
/// `ih_fire_intuimessage(window, IDCMP_CLOSEWINDOW, 0, 0)`, which (fire_message, inputhandler_support.c:195) posts only when the window
/// subscribes to CLOSEWINDOW and has a UserPort, and is a no-op otherwise. Every other action is not implemented in AROS either and
/// is a named refusal here (the native window manager would own hide / show / move / size / stacking, but nothing asks for them yet).
pub fn window_action<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (h, action) = (cpu.areg(0), cpu.dreg(0));
    open_window(a, "WindowAction", h)?;
    if action != WAC_SENDIDCMPCLOSE { return Err(Refusal(format!("WindowAction {h:#x}: action {action} is not implemented (only WAC_SENDIDCMPCLOSE, as in AROS)"))); }
    let d = crate::idcmp::post(a, h, crate::idcmp::IDCMP_CLOSEWINDOW, 0, 0)?;
    Ok(format!("[intuition] WindowAction {h:#x} WAC_SENDIDCMPCLOSE -> {d:?}"))
}
