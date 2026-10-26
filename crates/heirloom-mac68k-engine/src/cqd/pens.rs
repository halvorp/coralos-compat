//! Pen colours (Imaging With QuickDraw, "Color Graphics Ports"): `RGBForeColor`, `RGBBackColor`, `GetForeColor`,
//! `GetBackColor`. In a CGrafPort the colour is `rgbFgColor`/`rgbBkColor` (36/42); drawing maps it to a pixel value through
//! the target's colour table ([`crate::qd::target::pens`]). A plain GrafPort stores only one of the eight standard colour
//! constants in `fgColor`/`bkColor`, so an RGB given to it is rounded to the nearest of the eight (and read back as that one).
use super::color::{nearest_standard_code, read_rgb, standard_color, write_rgb, Rgb};
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::qd::port::{is_cport, port_rgb, CP_RGB_BK, CP_RGB_FG, P_BK, P_FG};
use crate::qd::target::current_port;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA14, "RGBForeColor", &[Arg::P], None, rgb_fore::<U>)?;
    reg(m, 0xAA15, "RGBBackColor", &[Arg::P], None, rgb_back::<U>)?;
    reg(m, 0xAA19, "GetForeColor", &[Arg::P], None, get_fore::<U>)?;
    reg(m, 0xAA1A, "GetBackColor", &[Arg::P], None, get_back::<U>)
}

/// Store `c` as the current port's foreground or background colour.
pub fn set_pen_color<U: GuestUi>(m: &mut Mac<U>, port: u32, fore: bool, c: Rgb) -> Result<(), Refusal> {
    if is_cport(&m.mem, port) { return write_rgb(&mut m.mem, port.wrapping_add(if fore { CP_RGB_FG } else { CP_RGB_BK }), c); }
    m.mem.write_u32(port.wrapping_add(if fore { P_FG } else { P_BK }), nearest_standard_code(c)).map_err(Refusal::mem("GrafPort colour"))
}

fn set<U: GuestUi>(m: &mut Mac<U>, c: &Call, fore: bool) -> Result<Done, Refusal> {
    let (port, rgb) = (current_port(m)?, read_rgb(&m.mem, c.args.get(0))?);
    set_pen_color(m, port, fore, rgb)?;
    let kind = if is_cport(&m.mem, port) { "" } else { " (a classic port keeps the nearest of the eight standard colours)" };
    Ok(Done { result: 0, log: format!("[quickdraw] {} ({:04x},{:04x},{:04x}){kind}", c.name, rgb.r, rgb.g, rgb.b) })
}
fn rgb_fore<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set(m, c, true) }
fn rgb_back<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set(m, c, false) }

fn get<U: GuestUi>(m: &mut Mac<U>, c: &Call, fore: bool) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let (fg, bg) = port_rgb(&m.mem, port)?;
    let rgb = if fore { fg } else { bg };
    write_rgb(&mut m.mem, c.args.get(0), rgb)?;
    Ok(Done { result: 0, log: format!("[quickdraw] {} -> ({:04x},{:04x},{:04x})", c.name, rgb.r, rgb.g, rgb.b) })
}
fn get_fore<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { get(m, c, true) }
fn get_back<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { get(m, c, false) }

/// The colour a standard constant (`blackColor` ...) stands for.
pub fn constant_rgb(code: u32) -> Option<Rgb> { standard_color(code) }
