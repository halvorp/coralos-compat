//! IDCMP: the IntuiMessages the engine posts to a window's UserPort, and ModifyIDCMP.
//!
//! `struct IntuiMessage` (52 bytes; offsets by m68k gcc -mno-align-int over AROS intuition/intuition.h): ExecMessage 0
//! (a struct Message: ln_Type 8, mn_Length 18), Class 20, Code 24, Qualifier 26, IAddress 28, MouseX 32, MouseY 34,
//! Seconds 36, Micros 40, IDCMPWindow 44, SpecialLink 48.
//!
//! A native event ([`GuestUiEvent`]) becomes a message only if the window's IDCMPFlags subscribe to that class; the
//! window state the event changes (a checkbox's SELECTED flag, a string buffer) changes either way, as on AmigaOS.
//! Delivery is transactional: the message block is reserved BEFORE any state changes, and every failure after that
//! gives it back, so a refused delivery leaves the queue, the signal bits and the window untouched.
use alloc::string::String;
use alloc::format;
use crate::{exec_wait, gt_kinds, machine::Amiga, memory, menu_nav, ports, run::Delivered, Refusal};
use guestui_abi::{GuestUi, GuestUiEvent};
use retro_m68k::Cpu;

pub const LVO_MODIFY_IDCMP: i32 = -150;

pub const IDCMP_MOUSEMOVE: u32 = 0x10;
pub const IDCMP_GADGETUP: u32 = 0x40;
pub const IDCMP_MENUPICK: u32 = 0x100;
pub const IDCMP_CLOSEWINDOW: u32 = 0x200;
pub const INTUIMESSAGE_SIZE: u32 = 52;
/// Intuition's "no menu selected" number.
pub const MENUNULL: u16 = 0xFFFF;

const MSGPORT_SIZE: u32 = 34;
const NT_MESSAGE: u8 = 5;
// struct Window offsets.
pub const WD_IDCMP: u32 = 82;
pub const WD_USERPORT: u32 = 86;
// struct IntuiMessage offsets.
const IM_LENGTH: u32 = 18;
const IM_CLASS: u32 = 20;
const IM_CODE: u32 = 24;
const IM_IADDRESS: u32 = 28;
const IM_SECONDS: u32 = 36;
const IM_MICROS: u32 = 40;
const IM_WINDOW: u32 = 44;

/// Give `win` a UserPort with its own signal bit.
pub fn create_user_port<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    let bit = ports::alloc_signal_bit(a, -1)?.ok_or_else(|| Refusal("OpenWindow: no free signal for the UserPort".into()))?;
    let port = match memory::alloc_zeroed(a, MSGPORT_SIZE, "UserPort") {
        Ok(p) => p,
        Err(e) => { ports::release_signal_bit(a, bit as i32)?; return Err(e); }
    };
    ports::init_port(a, port, bit)?;
    a.mem.write_u32(win + WD_USERPORT, port).map_err(Refusal::mem("Window.UserPort"))
}

/// Take `win`'s UserPort and its signal bit back (its queued messages must already be gone: `drop_window_msgs`).
pub fn free_user_port<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    let port = a.mem.read_u32(win + WD_USERPORT).map_err(Refusal::mem("Window.UserPort"))?;
    if port == 0 { return Ok(()); }
    let bit = a.mem.read_u8(port + 15).map_err(Refusal::mem("mp_SigBit"))? as i32;
    ports::release_signal_bit(a, bit)?;
    a.heap.free(port, MSGPORT_SIZE).map_err(|e| Refusal(format!("UserPort: {e}")))?;
    a.mem.write_u32(win + WD_USERPORT, 0).map_err(Refusal::mem("Window.UserPort"))
}

/// ModifyIDCMP(a0 = window, d0 = flags) -> TRUE. Going from none to some creates the UserPort, from some to none
/// takes it away (a message the program received and has not replied is its bug: refused, nothing changed).
pub fn modify_idcmp<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, flags) = (cpu.areg(0), cpu.dreg(0));
    if !a.windows.contains_key(&win) { return Err(Refusal(format!("ModifyIDCMP: {win:#x} is not an open window"))); }
    let port = a.mem.read_u32(win + WD_USERPORT).map_err(Refusal::mem("Window.UserPort"))?;
    if flags != 0 && port == 0 { create_user_port(a, win)?; }
    if flags == 0 && port != 0 { a.drop_window_msgs(win, port)?; free_user_port(a, win)?; }
    a.mem.write_u32(win + WD_IDCMP, flags).map_err(Refusal::mem("Window.IDCMPFlags"))?;
    cpu.set_d0(1);
    Ok(format!("[intuition] ModifyIDCMP {win:#x} flags={flags:#x}"))
}

/// Does `win` subscribe to `class`?
pub fn subscribed<U: GuestUi>(a: &Amiga<U>, win: u32, class: u32) -> Result<bool, Refusal> {
    Ok(a.mem.read_u32(win + WD_IDCMP).map_err(Refusal::mem("Window.IDCMPFlags"))? & class != 0)
}

/// A zeroed message block, not yet visible to the guest.
pub fn reserve<U: GuestUi>(a: &mut Amiga<U>) -> Result<u32, Refusal> { memory::alloc_zeroed(a, INTUIMESSAGE_SIZE, "deliver: IntuiMessage") }

/// Fill the reserved `msg` and link it on `win`'s UserPort (signalling the task). Any failure gives the block back.
pub fn queue<U: GuestUi>(a: &mut Amiga<U>, win: u32, msg: u32, class: u32, code: u16, iaddress: u32) -> Result<Delivered, Refusal> {
    let r = fill_and_link(a, win, msg, class, code, iaddress);
    if r.is_err() { let _ = a.heap.free(msg, INTUIMESSAGE_SIZE); }
    r.map(|()| Delivered::Queued(msg))
}

fn fill_and_link<U: GuestUi>(a: &mut Amiga<U>, win: u32, msg: u32, class: u32, code: u16, iaddress: u32) -> Result<(), Refusal> {
    let m = Refusal::mem;
    let port = a.mem.read_u32(win + WD_USERPORT).map_err(m("Window.UserPort"))?;
    if port == 0 { return Err(Refusal(format!("deliver: window {win:#x} has no UserPort"))); }
    a.mem.write_u16(msg + IM_LENGTH, INTUIMESSAGE_SIZE as u16).map_err(m("mn_Length"))?;
    a.mem.write_u32(msg + IM_CLASS, class).map_err(m("Class"))?;
    a.mem.write_u16(msg + IM_CODE, code).map_err(m("Code"))?;
    a.mem.write_u32(msg + IM_IADDRESS, iaddress).map_err(m("IAddress"))?;
    a.mem.write_u32(msg + IM_SECONDS, (a.clock_us / 1_000_000) as u32).map_err(m("Seconds"))?;
    a.mem.write_u32(msg + IM_MICROS, (a.clock_us % 1_000_000) as u32).map_err(m("Micros"))?;
    a.mem.write_u32(msg + IM_WINDOW, win).map_err(m("IDCMPWindow"))?;
    exec_wait::put_msg(a, port, msg, NT_MESSAGE)?;
    a.engine_msgs.insert(msg, false);
    Ok(())
}

/// Post one message if `win` subscribes to `class`.
pub fn post<U: GuestUi>(a: &mut Amiga<U>, win: u32, class: u32, code: u16, iaddress: u32) -> Result<Delivered, Refusal> {
    if !subscribed(a, win, class)? { return Ok(Delivered::IgnoredUnsubscribed); }
    let msg = reserve(a)?;
    queue(a, win, msg, class, code, iaddress)
}

/// The guest window a native window is.
pub fn guest_window<U: GuestUi>(a: &Amiga<U>, native: guestui_abi::UiObjectRef) -> Option<u32> {
    a.windows.iter().find(|(_, &n)| n == native).map(|(&w, _)| w)
}

/// A native event for the guest ([`Amiga::deliver`]).
pub fn deliver<U: GuestUi>(a: &mut Amiga<U>, ev: GuestUiEvent) -> Result<Delivered, Refusal> {
    if let Some(d) = crate::mui::try_deliver(a, &ev)? { return Ok(d); }
    match ev {
        GuestUiEvent::CloseRequested { win, .. } => match guest_window(a, win) {
            Some(w) => post(a, w, IDCMP_CLOSEWINDOW, 0, w),
            None => Ok(Delivered::NotForAGuestWindow),
        },
        GuestUiEvent::MenuCommand { win, token } => match guest_window(a, win) {
            Some(w) => menu_nav::menu_pick(a, w, token),
            None => Ok(Delivered::NotForAGuestWindow),
        },
        GuestUiEvent::ControlChanged { win, control, token, value } => match guest_window(a, win) {
            Some(w) => gt_kinds::control_changed(a, w, win, control, token, value),
            None => Ok(Delivered::NotForAGuestWindow),
        },
        ev @ GuestUiEvent::Resized { .. } => Err(Refusal(format!("deliver: {ev:?} not bridged yet"))),
    }
}
