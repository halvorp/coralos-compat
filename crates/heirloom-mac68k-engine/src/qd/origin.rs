//! `SetOrigin(h, v)` (Atkinson's QuickDraw, `GrafAsm.a`): "re-define the local coords by adjusting portBits.bounds, portRect, and
//! visRgn". With `dh = h - portRect.left` and `dv = v - portRect.top`, the port's `portBits.bounds`, `portRect` and `visRgn` move
//! by (+dh, +dv); `clipRgn` is NOT touched (the program owns it and keeps it in the coordinates it set it in); a call that
//! leaves the origin where it is does nothing. The window's pending update region is kept in local coordinates here, so it moves
//! with them (a real Window Manager keeps it global, where nothing needs to change).
use super::port::{bits_addr, P_RECT, P_VIS};
use super::target::current_port;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA877, "MovePortTo", &[Arg::W, Arg::W], None, move_port_to::<U>)?;
    super::reg(m, 0xA878, "SetOrigin", &[Arg::W, Arg::W], None, set_origin::<U>)
}

/// `MovePortTo(leftGlobal, topGlobal)` (Inside Macintosh: QuickDraw): repositions `portBits.bounds`
/// (or `portPixMap^^.bounds`) so that `portRect.topLeft` lies at `(leftGlobal, topGlobal)` in global
/// coordinates, without changing `portRect`, `visRgn`, or `clipRgn`.
fn move_port_to<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let (left_g, top_g) = (c.args.get(0) as u16 as i16, c.args.get(1) as u16 as i16);
    let e = Refusal::mem("GrafPort");
    let top = m.mem.read_i16(port.wrapping_add(P_RECT)).map_err(&e)?;
    let left = m.mem.read_i16(port.wrapping_add(P_RECT + 2)).map_err(&e)?;
    let bounds = bits_addr(&m.mem, port)?.wrapping_add(6);
    let btop = m.mem.read_i16(bounds).map_err(&e)?;
    let bleft = m.mem.read_i16(bounds + 2).map_err(&e)?;
    let bbot = m.mem.read_i16(bounds + 4).map_err(&e)?;
    let bright = m.mem.read_i16(bounds + 6).map_err(&e)?;
    let new_top = top.wrapping_sub(top_g);
    let new_left = left.wrapping_sub(left_g);
    let new_bot = new_top.wrapping_add(bbot.wrapping_sub(btop));
    let new_right = new_left.wrapping_add(bright.wrapping_sub(bleft));
    for (i, v) in [new_top, new_left, new_bot, new_right].into_iter().enumerate() {
        m.mem.write_u16(bounds + 2 * i as u32, v as u16).map_err(&e)?;
    }
    Ok(Done { result: 0, log: format!("[quickdraw] MovePortTo ({left_g},{top_g}): bounds -> ({new_top},{new_left},{new_bot},{new_right})") })
}

fn set_origin<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let (h, v) = (c.args.get(0) as u16 as i16, c.args.get(1) as u16 as i16);
    let e = Refusal::mem("GrafPort");
    let (top, left) = (m.mem.read_i16(port.wrapping_add(P_RECT)).map_err(&e)?, m.mem.read_i16(port.wrapping_add(P_RECT + 2)).map_err(&e)?);
    let (dh, dv) = (h.wrapping_sub(left), v.wrapping_sub(top));
    if dh == 0 && dv == 0 { return Ok(Done { result: 0, log: format!("[quickdraw] SetOrigin {h},{v} (already the origin)") }); }
    // portBits.bounds = top, left, bottom, right at +6; portRect likewise at P_RECT: both move by (dv, dh, dv, dh).
    for base in [bits_addr(&m.mem, port)?.wrapping_add(6), port.wrapping_add(P_RECT)] {
        for (i, d) in [dv, dh, dv, dh].into_iter().enumerate() {
            let a = base + 2 * i as u32;
            let old = m.mem.read_i16(a).map_err(&e)?;
            m.mem.write_u16(a, old.wrapping_add(d) as u16).map_err(&e)?;
        }
    }
    let vis = m.mem.read_u32(port.wrapping_add(P_VIS)).map_err(&e)?;
    if vis != 0 {
        let mut g = m.read_region(vis)?;
        g.translate(i32::from(dh), i32::from(dv));
        m.write_region(vis, &g)?;
    }
    if let Some(u) = m.qd.update.get_mut(&port) { u.translate(i32::from(dh), i32::from(dv)); }
    Ok(Done { result: 0, log: format!("[quickdraw] SetOrigin {h},{v}: bounds, portRect and visRgn moved by ({dh},{dv}); clipRgn untouched") })
}
