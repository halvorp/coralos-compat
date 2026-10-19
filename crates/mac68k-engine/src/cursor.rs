//! The cursor traps (Inside Macintosh I, "QuickDraw: Cursors"): `SetCursor`, `HideCursor`, `ShowCursor`, `ObscureCursor`.
//!
//! The pointer is the HOST's: the guest cannot draw it, and the seam (`guestui-abi`) has no cursor call. So the engine keeps
//! the shape the program set (a 68-byte `Cursor`: 16 words of data, 16 words of mask, the hot spot) and the hide level, as
//! guest-visible state, and names in the log that the host shows its own pointer. `ShowCursor` never raises the level above 0
//! (the book: the level is balanced by the calls, it cannot go positive).
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// sizeof(Cursor): `data` 32 bytes, `mask` 32 bytes, `hotSpot` Point 4 bytes.
pub const CURSOR_SIZE: u32 = 68;

#[derive(Default)]
pub struct CursorState {
    /// The last `SetCursor` shape (`None` = the arrow, until the program sets one).
    pub shape: Option<[u8; CURSOR_SIZE as usize]>,
    /// 0 = visible; each `HideCursor` lowers it, each `ShowCursor` raises it (never above 0).
    pub level: i32,
    pub obscured: bool,
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA851, &[Arg::P], None, set_cursor::<U>)?;
    m.register(0xA852, &[], None, hide_cursor::<U>)?;
    m.register(0xA853, &[], None, show_cursor::<U>)?;
    m.register(0xA856, &[], None, obscure_cursor::<U>)
}

fn set_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let at = c.args.get(0);
    let sp = m.mem.span(at, CURSOR_SIZE).map_err(Refusal::mem("Cursor"))?;
    let mut shape = [0u8; CURSOR_SIZE as usize];
    shape.copy_from_slice(m.mem.bytes(sp).map_err(Refusal::mem("Cursor"))?);
    let (v, h) = (i16::from_be_bytes([shape[64], shape[65]]), i16::from_be_bytes([shape[66], shape[67]]));
    m.cursor.shape = Some(shape);
    m.cursor.obscured = false;
    Ok(Done { result: 0, log: format!("[toolbox] SetCursor {at:#x}: 16x16 shape recorded, hot spot ({h},{v}); the host draws the pointer") })
}

fn hide_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.cursor.level -= 1;
    Ok(Done { result: 0, log: format!("[toolbox] HideCursor -> level {}", m.cursor.level) })
}

fn show_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.cursor.level = (m.cursor.level + 1).min(0);
    m.cursor.obscured = false;
    Ok(Done { result: 0, log: format!("[toolbox] ShowCursor -> level {}", m.cursor.level) })
}

fn obscure_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.cursor.obscured = true;
    Ok(Done { result: 0, log: "[toolbox] ObscureCursor (until the mouse moves)".into() })
}
