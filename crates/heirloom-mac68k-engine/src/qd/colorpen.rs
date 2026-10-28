//! The classic eight-colour pens: `ForeColor`, `BackColor`, `ColorBit` (Inside Macintosh V, "Color QuickDraw": they exist for
//! printing and colour screens in the original QuickDraw too). The colour is stored in the port's `fgColor` (80) / `bkColor` (84)
//! / `colrBit` (88) fields where a program can read it back. In a classic GrafPort the pixels are 1 bit deep, so the ink is still
//! black or white (a colour is not rendered, and the call says so in the log); in a CGrafPort the colour constant sets `rgbFgColor` /
//! `rgbBkColor` to the matching RGB ([`crate::cqd::color::standard_color`]) and is drawn in colour.
use super::port::{is_cport, P_BK, P_FG};
use super::target::current_port;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const COLR_BIT: u32 = 88;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA862, "ForeColor", &[Arg::L], None, fore_color::<U>)?;
    super::reg(m, 0xA863, "BackColor", &[Arg::L], None, back_color::<U>)?;
    super::reg(m, 0xA864, "ColorBit", &[Arg::W], None, color_bit::<U>)
}

fn set<U: GuestUi>(m: &mut Mac<U>, c: &Call, off: u32) -> Result<Done, Refusal> {
    let (port, color) = (current_port(m)?, c.args.get(0));
    let name = crate::cqd::color::STANDARD_COLORS.iter().find(|(v, ..)| *v == color).map(|&(_, n, _)| n).ok_or_else(|| Refusal(format!("{}: {color} is not one of the eight standard colours", c.name)))?;
    m.mem.write_u32(port.wrapping_add(off), color).map_err(Refusal::mem("GrafPort colour"))?;
    if is_cport(&m.mem, port) {
        let rgb = crate::cqd::pens::constant_rgb(color).ok_or_else(|| Refusal(format!("{}: {color} is not one of the eight standard colours", c.name)))?;
        crate::cqd::pens::set_pen_color(m, port, off == P_FG, rgb)?;
        return Ok(Done { result: 0, log: format!("[quickdraw] {} {name} (a colour port: rgb{}Color set)", c.name, if off == P_FG { "Fg" } else { "Bk" }) });
    }
    Ok(Done { result: 0, log: format!("[quickdraw] {} {name} (stored in the port; the pixels are 1 bit: black or white ink only)", c.name) })
}
fn fore_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set(m, c, P_FG) }
fn back_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set(m, c, P_BK) }
fn color_bit<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (port, bit) = (current_port(m)?, c.args.get(0) as u16);
    m.mem.write_u16(port.wrapping_add(COLR_BIT), bit).map_err(Refusal::mem("GrafPort colrBit"))?;
    Ok(Done { result: 0, log: format!("[quickdraw] ColorBit {bit} (stored; no colour planes in a 1-bit port)") })
}
