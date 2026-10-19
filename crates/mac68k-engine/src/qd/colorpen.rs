//! The classic eight-colour pens: `ForeColor`, `BackColor`, `ColorBit` (Inside Macintosh V, "Color QuickDraw": they exist for
//! printing and colour screens in the original QuickDraw too). The colour is stored in the port's `fgColor` (80) / `bkColor` (84)
//! / `colrBit` (88) fields where a program can read it back. The engine's pixels are 1 bit deep, so the ink is still black or
//! white: a colour is not rendered, and each call says so in the log.
use super::port::{P_BK, P_FG};
use super::target::current_port;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const COLR_BIT: u32 = 88;
/// blackColor 33, whiteColor 30, redColor 205, greenColor 341, blueColor 409, cyanColor 273, magentaColor 137, yellowColor 69.
const NAMES: [(u32, &str); 8] = [(33, "black"), (30, "white"), (205, "red"), (341, "green"), (409, "blue"), (273, "cyan"), (137, "magenta"), (69, "yellow")];

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA862, "ForeColor", &[Arg::L], None, fore_color::<U>)?;
    super::reg(m, 0xA863, "BackColor", &[Arg::L], None, back_color::<U>)?;
    super::reg(m, 0xA864, "ColorBit", &[Arg::W], None, color_bit::<U>)
}

fn set<U: GuestUi>(m: &mut Mac<U>, c: &Call, off: u32) -> Result<Done, Refusal> {
    let (port, color) = (current_port(m)?, c.args.get(0));
    let name = NAMES.iter().find(|(v, _)| *v == color).map(|(_, n)| *n).ok_or_else(|| Refusal(format!("{}: {color} is not one of the eight standard colours", c.name)))?;
    m.mem.write_u32(port.wrapping_add(off), color).map_err(Refusal::mem("GrafPort colour"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] {} {name} (stored in the port; the pixels are 1 bit: black or white ink only)", c.name) })
}
fn fore_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set(m, c, P_FG) }
fn back_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set(m, c, P_BK) }
fn color_bit<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (port, bit) = (current_port(m)?, c.args.get(0) as u16);
    m.mem.write_u16(port.wrapping_add(COLR_BIT), bit).map_err(Refusal::mem("GrafPort colrBit"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] ColorBit {bit} (stored; no colour planes in a 1-bit port)") })
}
