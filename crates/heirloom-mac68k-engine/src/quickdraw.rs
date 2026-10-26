//! QuickDraw's global state and the Init family every classic program calls first.
//!
//! `InitGraf(@qd.thePort)`: the program passes the address of the LAST field of the QuickDraw globals, which live
//! below A5. The routine stores that pointer in the long at A5 (how QuickDraw finds its globals) and fills the
//! public globals in place. Layout of `QDGlobals` (Inside Macintosh; `publics[130]` + `privates[76]` confirmed by
//! metamage_1 `68k/modules/ams-qd/InitGraf.cc`), as offsets from the `thePort` address `P`:
//!
//! ```text
//! P-202 privates (76 bytes)   P-126 randSeed (long)   P-122 screenBits (BitMap, 14)   P-108 arrow (Cursor, 68)
//! P-40 dkGray  P-32 ltGray  P-24 gray  P-16 black  P-8 white (Patterns, 8 each)   P+0 thePort (GrafPtr)
//! ```
//! This file is the globals and the Init family plus `SetPort`/`GetPort`; the drawing itself is in [`crate::qd`].
use crate::lowmem::{CRSR_PIN, CURRENT_A5, SCRN_BASE, SCREEN_ROW};
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const PRIVATES: u32 = 76;
pub const PUBLICS: u32 = 130;
pub const OFF_RAND_SEED: u32 = 126;
pub const OFF_SCREEN_BITS: u32 = 122;
pub const OFF_ARROW: u32 = 108;
pub const OFF_DK_GRAY: u32 = 40;
pub const OFF_LT_GRAY: u32 = 32;
pub const OFF_GRAY: u32 = 24;
pub const OFF_BLACK: u32 = 16;
pub const OFF_WHITE: u32 = 8;

/// The standard arrow cursor: 16 rows of data, 16 rows of mask, hot spot (v=1, h=1). Inside Macintosh "Cursor
/// Utilities"; the bitmaps agree with metamage_1's Chiaroscuro Arrow_Cursor.cc.
pub static ARROW: [u16; 32] = [
    0x0000, 0x4000, 0x6000, 0x7000, 0x7800, 0x7C00, 0x7E00, 0x7F00, 0x7F80, 0x7C00, 0x6C00, 0x4600, 0x0600, 0x0300, 0x0300, 0x0000,
    0xC000, 0xE000, 0xF000, 0xF800, 0xFC00, 0xFE00, 0xFF00, 0xFF80, 0xFFC0, 0xFFE0, 0xFE00, 0xEF00, 0xCF00, 0x8780, 0x0780, 0x0380,
];

/// Fill the QuickDraw globals for a `thePort` address `p`.
pub fn fill_globals<U: GuestUi>(m: &mut Mac<U>, p: u32) -> Result<(), Refusal> {
    let e = Refusal::mem("QuickDraw globals");
    let base = p.checked_sub(PRIVATES + PUBLICS - 4).ok_or_else(|| Refusal(format!("InitGraf: thePort address {p:#x} leaves no room below it for the globals")))?;
    m.mem.zero(base, PRIVATES + PUBLICS).map_err(&e)?;
    m.mem.write_u32(p - OFF_RAND_SEED, 1).map_err(&e)?;
    // screenBits: baseAddr, rowBytes, bounds (the screen), copied from the low-memory screen description
    let (sb, row) = (m.mem.read_u32(SCRN_BASE).map_err(&e)?, m.mem.read_u16(SCREEN_ROW).map_err(&e)?);
    m.mem.write_u32(p - OFF_SCREEN_BITS, sb).map_err(&e)?;
    m.mem.write_u16(p - OFF_SCREEN_BITS + 4, row).map_err(&e)?;
    for i in 0..4 { let v = m.mem.read_u16(CRSR_PIN + 2 * i).map_err(&e)?; m.mem.write_u16(p - OFF_SCREEN_BITS + 6 + 2 * i, v).map_err(&e)?; }
    for (i, w) in ARROW.iter().enumerate() { m.mem.write_u16(p - OFF_ARROW + 2 * i as u32, *w).map_err(&e)?; }
    m.mem.write_u16(p - OFF_ARROW + 64, 1).map_err(&e)?; // hotSpot.v
    m.mem.write_u16(p - OFF_ARROW + 66, 1).map_err(&e)?; // hotSpot.h
    for (off, pat) in [(OFF_DK_GRAY, 0x77DD_77DDu32), (OFF_LT_GRAY, 0x8822_8822), (OFF_GRAY, 0xAA55_AA55), (OFF_BLACK, 0xFFFF_FFFF), (OFF_WHITE, 0)] {
        m.mem.write_u32(p - off, pat).map_err(&e)?;
        m.mem.write_u32(p - off + 4, pat).map_err(&e)?;
    }
    Ok(())
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA86E, &[Arg::L], None, init_graf::<U>)?; // InitGraf(globalPtr)
    m.register(0xA873, &[Arg::P], None, set_port::<U>)?; // SetPort(gp)
    m.register(0xA874, &[Arg::L], None, get_port::<U>)?; // GetPort(VAR gp)
    // PROCEDURE InitFonts; InitCursor; SystemTask: nothing to keep yet (TEInit and TEIdle are the TextEdit module's)
    for t in [0xA8FE, 0xA850, 0xA9B4] { m.register(t, &[], None, accept::<U>)?; }
    m.register(0xA97B, &[Arg::L], None, accept::<U>) // InitDialogs(resumeProc)
}

pub(crate) fn accept<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[toolbox] {} (accepted)", c.name) })
}

fn init_graf<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = c.args.get(0);
    fill_globals(m, p)?;
    let a5 = m.mem.read_u32(CURRENT_A5).map_err(Refusal::mem("CurrentA5"))?;
    if a5 != 0 { m.mem.write_u32(a5, p).map_err(Refusal::mem("A5 world"))?; }
    m.qd_globals = p;
    m.init_devices()?; // Color QuickDraw machines: the main screen device exists from here on
    Ok(Done { result: 0, log: format!("[toolbox] InitGraf thePort at {p:#x}") })
}

/// thePort's slot is the `thePort` field the program handed to InitGraf.
fn port_slot<U: GuestUi>(m: &Mac<U>, what: &str) -> Result<u32, Refusal> {
    if m.qd_globals == 0 { Err(Refusal(format!("{what}: InitGraf has not run"))) } else { Ok(m.qd_globals) }
}

pub(crate) fn set_the_port<U: GuestUi>(m: &mut Mac<U>, port: u32) -> Result<(), Refusal> {
    let slot = port_slot(m, "SetPort")?;
    m.mem.write_u32(slot, port).map_err(Refusal::mem("thePort"))
}

fn set_port<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = c.args.get(0);
    set_the_port(m, port)?;
    Ok(Done { result: 0, log: format!("[toolbox] SetPort {port:#x}") })
}

fn get_port<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let slot = port_slot(m, "GetPort")?;
    let port = m.mem.read_u32(slot).map_err(Refusal::mem("thePort"))?;
    m.mem.write_u32(c.args.get(0), port).map_err(Refusal::mem("GetPort result"))?;
    Ok(Done { result: 0, log: format!("[toolbox] GetPort -> {port:#x}") })
}
