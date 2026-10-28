//! Port and canvas lifecycle: a window's content canvas and the regions/patterns of its GrafPort are created with the
//! window and freed with it; `OpenPort`/`InitPort`/`ClosePort`/`SetPortBits` make an offscreen port over a real BitMap;
//! `GlobalToLocal`/`LocalToGlobal` convert through `portBits.bounds`.
use super::port::*;
use super::regions::WIDE_OPEN;
use super::target::current_port;
use super::update::invalidate_all;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::quickdraw::{set_the_port, OFF_SCREEN_BITS};
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::{Bitmap, Rect, Region};

const WHITE: [u8; 8] = [0; 8];
const BLACK_COLOR: u32 = 33;
const WHITE_COLOR: u32 = 30;

/// Write the default pen/text/pattern state of a fresh port (the bitmap, rect and visRgn are set by the callers).
fn init_fields<U: GuestUi>(m: &mut Mac<U>, port: u32, vis: u32, clip: u32) -> Result<(), Refusal> {
    if !is_cport(&m.mem, port) {
        write_pat(&mut m.mem, port.wrapping_add(P_BK_PAT), &WHITE)?;
        write_pat(&mut m.mem, port.wrapping_add(P_FILL_PAT), &[0xFF; 8])?;
        write_pat(&mut m.mem, port.wrapping_add(P_PN_PAT), &[0xFF; 8])?;
    }
    init_common_fields(m, port, vis, clip)
}

/// The fields a GrafPort and a CGrafPort share (same offsets): regions, pen, text, colour codes.
pub(crate) fn init_common_fields<U: GuestUi>(m: &mut Mac<U>, port: u32, vis: u32, clip: u32) -> Result<(), Refusal> {
    let e = Refusal::mem("GrafPort");
    m.mem.write_u32(port.wrapping_add(P_VIS), vis).map_err(&e)?;
    m.mem.write_u32(port.wrapping_add(P_CLIP), clip).map_err(&e)?;
    set_pen_loc(&mut m.mem, port, 0, 0)?;
    for (off, v) in [(P_PN_SIZE, 1u16), (P_PN_SIZE + 2, 1), (P_PN_MODE, PAT_COPY), (P_PN_VIS, 0), (P_TX_FONT, 0), (P_TX_MODE, SRC_OR), (P_TX_SIZE, 0)] {
        m.mem.write_u16(port.wrapping_add(off), v).map_err(&e)?;
    }
    m.mem.write_u8(port.wrapping_add(P_TX_FACE), 0).map_err(&e)?;
    for (off, v) in [(P_SP_EXTRA, 0), (P_FG, BLACK_COLOR), (P_BK, WHITE_COLOR), (84 + 4, 0), (P_RGN_SAVE, 0)] { m.mem.write_u32(port.wrapping_add(off), v).map_err(&e)?; }
    Ok(())
}

/// A window exists: its canvas (white), regions and drawing state.

/// `PortSize(width, height)`: the current port's `portRect` keeps its top-left and takes the new size (QuickDraw reference).
fn port_size<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = super::target::current_port(m)?;
    let (w, h) = (c.args.get(0) as u16 as i16, c.args.get(1) as u16 as i16);
    let e = Refusal::mem("portRect");
    let top = m.mem.read_i16(port.wrapping_add(P_RECT)).map_err(&e)?;
    let left = m.mem.read_i16(port.wrapping_add(P_RECT + 2)).map_err(&e)?;
    m.mem.write_u16(port.wrapping_add(P_RECT + 4), top.wrapping_add(h) as u16).map_err(&e)?;
    m.mem.write_u16(port.wrapping_add(P_RECT + 6), left.wrapping_add(w) as u16).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[quickdraw] PortSize {w}x{h}") })
}

/// The depth of a window's canvas: 1 for a classic window; a colour window (`NewCWindow`) has the main device's depth.
fn window_depth<U: GuestUi>(m: &Mac<U>, win: u32) -> u8 {
    if m.windows.get(&win).is_some_and(|w| w.color) { crate::cqd::gdevice::SCREEN_DEPTH } else { 1 }
}

pub(crate) fn window_created<U: GuestUi>(m: &mut Mac<U>, win: u32) -> Result<(), Refusal> {
    let r = m.windows.get(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?.rect;
    let (w, h) = (i32::from(r.right) - i32::from(r.left), i32::from(r.bottom) - i32::from(r.top));
    let canvas = Bitmap::new(w, h, window_depth(m, win)).ok_or_else(|| Refusal(format!("window {w}x{h}: no canvas of that size")))?;
    let vis = m.new_region(&Region::from_rect(Rect::new(0, 0, w, h)))?;
    let clip = m.new_region(&Region::from_rect(WIDE_OPEN))?;
    init_fields(m, win, vis, clip)?;
    m.qd.canvases.insert(win, canvas);
    m.qd.present.forget(win);
    invalidate_all(m, win)
}

/// The window just got its native window: whatever was drawn while it was invisible is its first frame.
pub(crate) fn window_shown<U: GuestUi>(m: &mut Mac<U>, win: u32) {
    if let Some(c) = m.qd.canvases.get(&win) { let r = c.rect(); m.qd.present.add_damage(win, r); }
}

/// The window's size changed (native resize): a new blank canvas, a whole-window update.
pub(crate) fn window_resized<U: GuestUi>(m: &mut Mac<U>, win: u32) -> Result<(), Refusal> {
    let r = m.windows.get(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?.rect;
    let (w, h) = (i32::from(r.right) - i32::from(r.left), i32::from(r.bottom) - i32::from(r.top));
    let canvas = Bitmap::new(w, h, window_depth(m, win)).ok_or_else(|| Refusal(format!("window {w}x{h}: no canvas of that size")))?;
    m.qd.canvases.insert(win, canvas);
    m.qd.present.forget(win);
    m.qd.plane.reset(win);
    let vis = m.mem.read_u32(win.wrapping_add(P_VIS)).map_err(Refusal::mem("visRgn"))?;
    m.write_region(vis, &Region::from_rect(Rect::new(0, 0, w, h)))?;
    invalidate_all(m, win)
}

/// The window is gone: canvas, bookkeeping and its two region handles.
pub(crate) fn window_disposed<U: GuestUi>(m: &mut Mac<U>, win: u32) {
    m.free_window_regions(win);
    m.qd.canvases.remove(&win);
    m.qd.update.remove(&win);
    m.qd.saved_vis.remove(&win);
    m.qd.rgn_open.remove(&win);
    m.qd.present.forget(win);
    m.qd.plane.forget(win);
    m.qd.palettes.remove(&win);
    if is_cport(&m.mem, win) { m.release_cport(win); }
    for off in [P_VIS, P_CLIP] {
        if let Ok(h) = m.mem.read_u32(win.wrapping_add(off)) { if h != 0 { let _ = m.dispose_handle(h); } }
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use super::reg;
    reg(m, 0xA876, "PortSize", &[Arg::W, Arg::W], None, port_size::<U>)?;
    reg(m, 0xA86F, "OpenPort", &[Arg::P], None, open_port::<U>)?;
    reg(m, 0xA86D, "InitPort", &[Arg::P], None, init_port::<U>)?;
    reg(m, 0xA87D, "ClosePort", &[Arg::P], None, close_port::<U>)?;
    reg(m, 0xA875, "SetPBits", &[Arg::P], None, set_port_bits::<U>)?;
    reg(m, 0xA871, "GlobalToLocal", &[Arg::L], None, global_to_local::<U>)?;
    reg(m, 0xA870, "LocalToGlobal", &[Arg::L], None, local_to_global::<U>)
}

fn screen_bits_into<U: GuestUi>(m: &mut Mac<U>, port: u32) -> Result<(), Refusal> {
    let e = Refusal::mem("screenBits");
    if m.qd_globals == 0 { return Err(Refusal("OpenPort: InitGraf has not run (no screenBits to copy)".into())); }
    let b = m.mem.bytes(m.mem.span(m.qd_globals - OFF_SCREEN_BITS, 14).map_err(&e)?).map_err(&e)?.to_vec();
    m.mem.write_bytes(port.wrapping_add(P_BITS), &b).map_err(&e)?;
    // portRect = the bitmap's bounds
    let bounds = read_rect(&m.mem, port.wrapping_add(P_BITS + BM_BOUNDS))?;
    write_rect(&mut m.mem, port.wrapping_add(P_RECT), bounds)
}

/// `OpenPort`'s work without making the port current: the record at `port` becomes a fresh port over the screen's bitmap (Inside
/// Macintosh I: `portBits` = `screenBits`, `portRect` = its bounds, `visRgn` that rectangle, `clipRgn` wide open). Returns the
/// port's rectangle.
pub(crate) fn open_screen_port<U: GuestUi>(m: &mut Mac<U>, port: u32) -> Result<Rect, Refusal> {
    m.mem.zero(port, PORT_SIZE).map_err(Refusal::mem("GrafPort"))?;
    screen_bits_into(m, port)?;
    let b = raster(read_rect(&m.mem, port.wrapping_add(P_RECT))?);
    let (vis, clip) = (m.new_region(&Region::from_rect(b))?, m.new_region(&Region::from_rect(WIDE_OPEN))?);
    init_fields(m, port, vis, clip)?;
    Ok(b)
}

fn open_port<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = super::target::checked_port(c.args.get(0))?;
    open_screen_port(m, port)?;
    set_the_port(m, port)?;
    Ok(Done { result: 0, log: format!("[quickdraw] OpenPort {port:#x}") })
}
fn init_port<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = super::target::checked_port(c.args.get(0))?;
    let (vis, clip) = (m.mem.read_u32(port.wrapping_add(P_VIS)).map_err(Refusal::mem("visRgn"))?, m.mem.read_u32(port.wrapping_add(P_CLIP)).map_err(Refusal::mem("clipRgn"))?);
    if vis == 0 || clip == 0 { return Err(Refusal(format!("InitPort {port:#x}: the port has no visRgn/clipRgn handles (use OpenPort first)"))); }
    if m.qd.canvases.contains_key(&port) { return Err(Refusal(format!("InitPort {port:#x}: that is a window's port; its bitmap is the window"))); }
    // IM: portBits := screenBits, portRect := screenBits.bounds, visRgn := RectRgn(screenBits.bounds), clipRgn wide open
    screen_bits_into(m, port)?;
    let b = raster(read_rect(&m.mem, port.wrapping_add(P_RECT))?);
    m.write_region(vis, &Region::from_rect(b))?;
    m.write_region(clip, &Region::from_rect(WIDE_OPEN))?;
    init_fields(m, port, vis, clip)?;
    set_the_port(m, port)?;
    Ok(Done { result: 0, log: format!("[quickdraw] InitPort {port:#x}") })
}
fn close_port<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = super::target::checked_port(c.args.get(0))?;
    if m.qd.canvases.contains_key(&port) { return Err(Refusal(format!("ClosePort {port:#x}: that is a window's port (use DisposeWindow)"))); }
    for off in [P_VIS, P_CLIP] {
        let h = m.mem.read_u32(port.wrapping_add(off)).map_err(Refusal::mem("GrafPort"))?;
        if h != 0 { m.dispose_handle(h).map_err(|e| Refusal(format!("ClosePort {port:#x}: region handle {h:#x}: memory manager error {e}")))?; m.mem.write_u32(port.wrapping_add(off), 0).map_err(Refusal::mem("GrafPort"))?; }
    }
    Ok(Done { result: 0, log: format!("[quickdraw] ClosePort {port:#x}") })
}
fn set_port_bits<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    if is_cport(&m.mem, port) { return Err(Refusal(format!("SetPortBits: {port:#x} is a CGrafPort: its pixels are set with SetPortPix (SetPortBits would overwrite its portPixMap and portVersion)"))); }
    let e = Refusal::mem("BitMap");
    let b = m.mem.bytes(m.mem.span(c.args.get(0), 14).map_err(&e)?).map_err(&e)?.to_vec();
    m.mem.write_bytes(port.wrapping_add(P_BITS), &b).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[quickdraw] SetPortBits {:#x} into {port:#x}", c.args.get(0)) })
}

/// local = global + bounds.topleft (the bounds map the bitmap's top-left to its local coordinates).
fn convert<U: GuestUi>(m: &mut Mac<U>, pt: u32, sign: i16, name: &str) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let e = Refusal::mem("Point");
    let bits = bits_addr(&m.mem, port)?;
    let (bt, bl) = (m.mem.read_i16(bits.wrapping_add(BM_BOUNDS)).map_err(&e)?, m.mem.read_i16(bits.wrapping_add(BM_BOUNDS + 2)).map_err(&e)?);
    let (v, h) = (m.mem.read_i16(pt).map_err(&e)?, m.mem.read_i16(pt + 2).map_err(&e)?);
    m.mem.write_u16(pt, v.wrapping_add(sign * bt) as u16).map_err(&e)?;
    m.mem.write_u16(pt + 2, h.wrapping_add(sign * bl) as u16).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name} ({v},{h})") })
}
fn global_to_local<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { convert(m, c.args.get(0), 1, "GlobalToLocal") }
fn local_to_global<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { convert(m, c.args.get(0), -1, "LocalToGlobal") }

