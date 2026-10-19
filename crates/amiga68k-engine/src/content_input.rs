//! Plain input for a window's CONTENT (ruling 17, piece 8): a pointer press, move or key the guest drew its own UI for becomes the
//! IntuiMessage Intuition would have sent, so a content plane is fully usable before anything is known about what it shows. Only
//! windows that asked for the class get it (IDCMPFlags); the window fields a program may read instead (`Window.MouseX/MouseY`)
//! follow the pointer either way.
//!
//! | host event | message | Code | notes |
//! |---|---|---|---|
//! | primary Down / Up | IDCMP_MOUSEBUTTONS | SELECTDOWN 0x68 / SELECTUP 0xE8 | Qualifier carries the held buttons |
//! | middle Down / Up | IDCMP_MOUSEBUTTONS | MIDDLEDOWN 0x6A / MIDDLEUP 0xEA | |
//! | secondary Down / Up | IDCMP_MOUSEBUTTONS | MENUDOWN 0x69 / MENUUP 0xE9 | only with WFLG_RMBTRAP; otherwise the menu system owns the button (nothing for the guest) |
//! | Move | IDCMP_MOUSEMOVE | 0 | only with WFLG_REPORTMOUSE |
//! | key down | IDCMP_VANILLAKEY | the character (Latin-1) | when the key types one |
//! | key down / up | IDCMP_RAWKEY | the Amiga raw code (+0x80 on release) | `code` is taken as the Amiga raw key code; one past 0x7F has none and is refused by name |
//!
//! MouseX / MouseY are window-relative (the event's own coordinates), Seconds / Micros the virtual clock. Facts: AROS
//! rom/intuition inputhandler, intuition/intuition.h (IDCMP_*, IEQUALIFIER_*, WFLG_REPORTMOUSE / RMBTRAP), devices/inputevent.h.
use alloc::format;
use crate::idcmp::{self, Stamp, WD_MOUSE_X, WD_MOUSE_Y};
use crate::{machine::Amiga, run::Delivered, Refusal};
use guestui_abi::{GuestUi, Modifiers, PointerButton, PointerPhase};

pub const IDCMP_MOUSEBUTTONS: u32 = 0x8;
pub const IDCMP_RAWKEY: u32 = 0x400;
pub const IDCMP_VANILLAKEY: u32 = 0x20_0000;

const SELECTDOWN: u16 = 0x68;
const MENUDOWN: u16 = 0x69;
const MIDDLEDOWN: u16 = 0x6A;
const UP: u16 = 0x80;

const IEQUALIFIER_LSHIFT: u16 = 0x1;
const IEQUALIFIER_CONTROL: u16 = 0x8;
const IEQUALIFIER_LALT: u16 = 0x10;
const IEQUALIFIER_LCOMMAND: u16 = 0x40;
const IEQUALIFIER_MIDBUTTON: u16 = 0x1000;
const IEQUALIFIER_RBUTTON: u16 = 0x2000;
const IEQUALIFIER_LEFTBUTTON: u16 = 0x4000;

const WD_FLAGS: u32 = 24;
const WFLG_REPORTMOUSE: u32 = 0x200;
const WFLG_RMBTRAP: u32 = 0x1_0000;

fn qualifier(m: Modifiers) -> u16 {
    [(Modifiers::SHIFT, IEQUALIFIER_LSHIFT), (Modifiers::CTRL, IEQUALIFIER_CONTROL), (Modifiers::ALT, IEQUALIFIER_LALT), (Modifiers::META, IEQUALIFIER_LCOMMAND)]
        .iter().filter(|(bit, _)| m.has(*bit)).fold(0, |q, (_, v)| q | v)
}

fn button_bit(b: PointerButton) -> u16 {
    match b { PointerButton::Primary => IEQUALIFIER_LEFTBUTTON, PointerButton::Secondary => IEQUALIFIER_RBUTTON, PointerButton::Middle => IEQUALIFIER_MIDBUTTON, PointerButton::None => 0 }
}

/// Post `class` to `win` if it listens, stamped with the pointer position and qualifier.
fn post<U: GuestUi>(a: &mut Amiga<U>, win: u32, class: u32, code: u16, stamp: Stamp) -> Result<Delivered, Refusal> {
    if !idcmp::subscribed(a, win, class)? { return Ok(Delivered::IgnoredUnsubscribed); }
    let msg = idcmp::reserve(a)?;
    idcmp::queue_stamped(a, win, msg, class, code, 0, stamp)
}

/// The IDCMP message a pointer event is, as Intuition's input handler makes it: `None` = no message (a move nobody asked for, the menu
/// button, which the menu system owns unless the window traps it). `reports_move` = WFLG_REPORTMOUSE, `traps_menu` = WFLG_RMBTRAP.
pub fn pointer_message(button: PointerButton, phase: PointerPhase, reports_move: bool, traps_menu: bool) -> Result<Option<(u32, u16)>, Refusal> {
    let down = match button { PointerButton::Primary => SELECTDOWN, PointerButton::Middle => MIDDLEDOWN, PointerButton::Secondary => MENUDOWN, PointerButton::None => 0 };
    match phase {
        PointerPhase::Move if reports_move => Ok(Some((idcmp::IDCMP_MOUSEMOVE, 0))),
        PointerPhase::Move => Ok(None),
        _ if button == PointerButton::Secondary && !traps_menu => Ok(None),
        _ if button == PointerButton::None => Err(Refusal(format!("deliver: a {phase:?} pointer event with no button"))),
        PointerPhase::Down => Ok(Some((IDCMP_MOUSEBUTTONS, down))),
        PointerPhase::Up => Ok(Some((IDCMP_MOUSEBUTTONS, down | UP))),
    }
}

/// Track the buttons held over window `key` and give the qualifier the message carries.
pub fn hold_buttons<U: GuestUi>(a: &mut Amiga<U>, key: u32, button: PointerButton, phase: PointerPhase) -> Stamp {
    let held = a.input.buttons.entry(key).or_insert(0);
    match phase {
        PointerPhase::Down => *held |= button_bit(button),
        PointerPhase::Up => *held &= !button_bit(button),
        PointerPhase::Move => {}
    }
    Stamp { qualifier: *held | qualifier(a.input.modifiers) }
}

/// A pointer event on the window's own content (no guest-drawn gadget owns it).
pub fn pointer<U: GuestUi>(a: &mut Amiga<U>, win: u32, x: i32, y: i32, button: PointerButton, phase: PointerPhase) -> Result<Delivered, Refusal> {
    let m = Refusal::mem("Window.MouseX/MouseY");
    a.mem.write_u16(win + WD_MOUSE_X, x as u16).map_err(&m)?;
    a.mem.write_u16(win + WD_MOUSE_Y, y as u16).map_err(&m)?;
    let stamp = hold_buttons(a, win, button, phase);
    let flags = a.mem.read_u32(win + WD_FLAGS).map_err(Refusal::mem("Window.Flags"))?;
    match pointer_message(button, phase, flags & WFLG_REPORTMOUSE != 0, flags & WFLG_RMBTRAP != 0)? {
        Some((class, code)) => post(a, win, class, code, stamp),
        None => Ok(Delivered::Handled),
    }
}

/// The qualifier of the input state right now: the modifiers held plus the pointer buttons held over `win`.
pub fn current_stamp<U: GuestUi>(a: &Amiga<U>, win: u32) -> Stamp {
    Stamp { qualifier: qualifier(a.input.modifiers) | a.input.buttons.get(&win).copied().unwrap_or(0) }
}

/// A key for the window, the way Intuition's input handler routes it: a key DOWN that types one character is VANILLAKEY when the
/// window listens for it, and that is all (AROS rom/intuition/inputhandler.c: `MapRawKey(..) == 1` fires VANILLAKEY and breaks); a
/// key that types nothing, a release, or a window without VANILLAKEY falls through to RAWKEY. A raw code past 0x7F is refused by
/// name BEFORE anything is queued.
pub fn key<U: GuestUi>(a: &mut Amiga<U>, win: u32, code: u32, text: &str, mods: Modifiers, down: bool) -> Result<Delivered, Refusal> {
    a.input.modifiers = mods;
    let stamp = current_stamp(a, win);
    if down && idcmp::subscribed(a, win, IDCMP_VANILLAKEY)? {
        if let Some(c) = text.chars().next().filter(|c| (*c as u32) < 0x100) {
            return post(a, win, IDCMP_VANILLAKEY, c as u16, stamp);
        }
    }
    if !idcmp::subscribed(a, win, IDCMP_RAWKEY)? { return Ok(Delivered::IgnoredUnsubscribed); }
    if code > 0x7F { return Err(Refusal(format!("deliver: key code {code:#x} has no Amiga raw key code (0..=0x7F)"))); }
    post(a, win, IDCMP_RAWKEY, code as u16 | if down { 0 } else { UP }, stamp)
}
