//! The cursor traps (Inside Macintosh I, "QuickDraw: Cursors"): `SetCursor`, `HideCursor`, `ShowCursor`, `ObscureCursor`, and the
//! Color QuickDraw cursor calls `GetCCursor`, `SetCCursor`, `DisposeCCursor`, `AllocCursor`.
//!
//! The pointer is the HOST's: the guest cannot draw it, and the seam (`heirloom-ui-abi`) has no cursor call. So the engine keeps
//! the shape the program set (a 68-byte `Cursor`: 16 words of data, 16 words of mask, the hot spot) and the hide level, as
//! guest-visible state, and names in the log that the host shows its own pointer. `ShowCursor` never raises the level above 0
//! (the book: the level is balanced by the calls, it cannot go positive).
//!
//! A color cursor (`'crsr'`, Imaging With QuickDraw "Color Cursors") is a `CCrsr` record: `crsrType` 0 ($8000 black and white,
//! $8001 color), `crsrMap` 2, `crsrData` 6, `crsrXData` 10, `crsrXValid` 14, `crsrXHandle` 16, `crsr1Data` 20 (a 16 x 16
//! 1-bit image), `crsrMask` 52, `crsrHotSpot` 84, `crsrXTable` 88, `crsrID` 92 (96 bytes). `GetCCursor` hands out the resource
//! as a CCrsrHandle; `SetCCursor` records its 1-bit image, mask and hot spot like `SetCursor` does: the pointer is the host's,
//! and the seam has no colour pointer, so the host shows the 1-bit form (named in the log).
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

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
    m.register(0xA856, &[], None, obscure_cursor::<U>)?;
    m.register(0xA855, &[Arg::L, Arg::L], None, shield_cursor::<U>)?;
    m.register(0xAA1B, &[Arg::W], Some(Arg::L), get_c_cursor::<U>)?;
    m.register(0xAA1C, &[Arg::L], None, set_c_cursor::<U>)?;
    m.register(0xAA26, &[Arg::L], None, dispose_c_cursor::<U>)?;
    m.register(0xAA1D, &[], None, |_, _, _| Ok(Done { result: 0, log: "[toolbox] AllocCursor (nothing to preallocate: the host owns the pointer)".into() }))
}

/// `PROCEDURE ShieldCursor(shieldRect: Rect; offsetPt: Point)` (Inside Macintosh V-?; the cursor is hidden while it is in `shieldRect`,
/// which a program asks for before it draws there, because the ROM draws the pointer INTO the screen bits and would be smeared). The
/// host owns the pointer here and never draws it into the program's pixels, so there is nothing to protect; the rectangle is read (a wild
/// pointer is still a named refusal) and the call is logged.
fn shield_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = crate::pascal::read_rect(&m.mem, c.args.get(0))?;
    let pt = c.args.get(1);
    Ok(Done { result: 0, log: format!("[toolbox] ShieldCursor ({},{},{},{}) offset ({},{}) (the host draws the pointer outside the program's pixels: nothing to shield)", r.top, r.left, r.bottom, r.right, (pt >> 16) as i16, pt as i16) })
}

/// sizeof(CCrsr).
const CCRSR_SIZE: u32 = 96;
const CCRSR_1DATA: u32 = 20;

fn get_c_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(r) = m.get_resource(*b"crsr", id, false) else { return Ok(Done { result: 0, log: format!("[toolbox] GetCCursor {id}: no 'crsr' resource (NIL)") }) };
    let bytes = m.handle_bytes(r).unwrap_or_default();
    if bytes.len() < CCRSR_SIZE as usize { return Err(Refusal(format!("GetCCursor {id}: the 'crsr' holds {} bytes, less than a CCrsr ({CCRSR_SIZE})", bytes.len()))); }
    let (h, p) = m.alloc_handle(bytes.len() as u32, "GetCCursor")?;
    m.mem.write_bytes(p, &bytes).map_err(Refusal::mem("CCrsr"))?;
    Ok(Done { result: h, log: format!("[toolbox] GetCCursor {id} -> {h:#x}") })
}

fn set_c_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let p = m.deref_handle(h, "SetCCursor")?;
    let sp = m.mem.span(p + CCRSR_1DATA, CURSOR_SIZE).map_err(Refusal::mem("CCrsr"))?;
    let mut shape = [0u8; CURSOR_SIZE as usize];
    shape.copy_from_slice(m.mem.bytes(sp).map_err(Refusal::mem("CCrsr"))?); // crsr1Data, crsrMask, crsrHotSpot lie in Cursor order
    m.cursor.shape = Some(shape);
    m.cursor.obscured = false;
    Ok(Done { result: 0, log: format!("[toolbox] SetCCursor {h:#x}: its 1-bit image recorded; the host draws the pointer (no colour pointer in the seam)") })
}

fn dispose_c_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    m.deref_handle(h, "DisposeCCursor")?;
    m.dispose_handle(h).map_err(|e| Refusal(format!("DisposeCCursor {h:#x}: memory manager error {e}")))?;
    Ok(Done { result: 0, log: format!("[toolbox] DisposeCCursor {h:#x}") })
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
