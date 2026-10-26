//! The polled mouse and the clock traps: `GetMouse`, `Button`, `StillDown`, `WaitMouseUp` (Event Manager) and `Delay`.
//!
//! The low-memory `Mouse` (a global Point, v then h) and `MBState` (bit 7 set = the button is UP) are kept by
//! [`crate::translate::input`] from the host's pointer events; these traps only read them. `GetMouse` converts to the CURRENT
//! port's coordinates: local = global + the top left of `portBits.bounds`. `StillDown` and `WaitMouseUp` answer FALSE as soon as
//! a `mouseUp` is queued after the press (the Event Manager looks at the event queue, not only at the button); `WaitMouseUp`
//! also takes that `mouseUp` out of the queue. `Delay(n)` lets `n` ticks of the virtual clock pass at once (the guest has nothing
//! else to do while it waits, and the vertical-retrace tasks run at the next trap), and returns the new `Ticks` in D0.
use crate::events::MOUSE_UP;
use crate::lowmem::{MB_STATE, MOUSE};
use crate::machine::{Call, Done, Mac};
use crate::osconv::Regs;
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

/// A port's `portBits.bounds` top-left (v, h): GrafPort offset 2 is the BitMap, whose bounds start 6 bytes in.
const PORT_BOUNDS: u32 = 2 + 6;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA972, &[Arg::P], None, get_mouse::<U>)?;
    m.register(0xA974, &[], Some(Arg::B), button::<U>)?;
    m.register(0xA973, &[], Some(Arg::B), still_down::<U>)?;
    m.register(0xA977, &[], Some(Arg::B), wait_mouse_up::<U>)?;
    m.register_os(0xA03B, delay::<U>)
}

impl<U: GuestUi> Mac<U> {
    fn button_down(&self) -> bool { self.mem.read_u8(MB_STATE).is_ok_and(|b| b & 0x80 == 0) }
    fn mouse_up_queued(&self) -> bool { self.ev.queue.iter().any(|e| e.what == MOUSE_UP) }
}

fn get_mouse<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let e = Refusal::mem("GetMouse");
    let (v, h) = (m.mem.read_i16(MOUSE).map_err(&e)?, m.mem.read_i16(MOUSE + 2).map_err(&e)?);
    let port = if m.qd_globals == 0 { 0 } else { m.mem.read_u32(m.qd_globals).map_err(&e)? };
    let (dv, dh) = if port == 0 { (0, 0) } else { (m.mem.read_i16(port + PORT_BOUNDS).map_err(&e)?, m.mem.read_i16(port + PORT_BOUNDS + 2).map_err(&e)?) };
    let (lv, lh) = (v.wrapping_add(dv), h.wrapping_add(dh));
    m.mem.write_u16(c.args.get(0), lv as u16).and_then(|_| m.mem.write_u16(c.args.get(0) + 2, lh as u16)).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[event] GetMouse -> ({lv},{lh}) local, ({v},{h}) global") })
}

fn button<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let down = m.button_down();
    Ok(Done { result: u32::from(down), log: format!("[event] Button -> {down}") })
}

fn still_down<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let down = m.button_down() && !m.mouse_up_queued();
    Ok(Done { result: u32::from(down), log: format!("[event] StillDown -> {down}") })
}

fn wait_mouse_up<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let up_at = m.ev.queue.iter().position(|e| e.what == MOUSE_UP);
    if let Some(i) = up_at { m.ev.queue.remove(i); }
    let down = m.button_down() && up_at.is_none();
    Ok(Done { result: u32::from(down), log: format!("[event] WaitMouseUp -> {down}{}", if up_at.is_some() { " (took the mouseUp)" } else { "" }) })
}

fn delay<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let n = r.a0;
    m.advance_ticks(n);
    r.d0 = m.ev.ticks;
    Ok(format!("[event] Delay {n} ticks -> Ticks {}", m.ev.ticks))
}
