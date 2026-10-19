//! Pictures (Inside Macintosh I, "QuickDraw: Pictures"): `OpenPicture`, `ClosePicture`, `KillPicture`, `PicComment`, `DrawPicture`.
//!
//! A `Picture` handle is `picSize` (2), `picFrame` (8) and the version-1 opcode stream: `$11 $01` (version), the recorded ops,
//! `$FF` (end). `OpenPicture(frame)` makes the handle and sets the port's `picSave` (offset 92) non-NIL; `ClosePicture` clears it
//! and writes `header + ops + $FF` into the handle; `PicComment` records a short (`$A0`, kind) or long (`$A1`, kind, size, data)
//! comment op, the only op recorded here.
//!
//! **Named limit.** Drawing into an open picture is NOT recorded: the drawing still reaches the port, and `ClosePicture` logs how
//! many drawing calls were made meanwhile and records a refusal naming it, so a stored picture that is empty is never mistaken
//! for a complete one. `DrawPicture` replays only a picture that holds no drawing ops (`$11 $01`, comments, `$FF`); any other op is
//! a named refusal (a PICT opcode interpreter is not part of this module).
use super::target::current_port;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// GrafPort.picSave.
pub const P_PIC_SAVE: u32 = 92;

/// A picture being recorded on one port.
pub struct OpenPic { pub handle: u32, pub frame: [u8; 8], pub ops: Vec<u8>, pub unrecorded: u32 }

/// Count a drawing call made while a picture is open on `port` (called by the drawing paths).
pub(crate) fn note_drawing<U: GuestUi>(m: &mut Mac<U>, port: u32) { if let Some(p) = m.qd.pictures.get_mut(&port) { p.unrecorded += 1; } }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA8F3, "OpenPicture", &[Arg::P], Some(Arg::L), open_picture::<U>)?;
    super::reg(m, 0xA8F4, "ClosePicture", &[], None, close_picture::<U>)?;
    super::reg(m, 0xA8F6, "DrawPicture", &[Arg::P, Arg::P], None, draw_picture::<U>)?;
    super::reg(m, 0xA8F2, "PicComment", &[Arg::W, Arg::W, Arg::P], None, pic_comment::<U>)?;
    super::reg(m, 0xA8F5, "KillPicture", &[Arg::P], None, kill_picture::<U>)
}

fn image(frame: &[u8; 8], ops: &[u8]) -> Vec<u8> {
    let mut b = alloc::vec![0, 0];
    b.extend_from_slice(frame);
    b.extend_from_slice(&[0x11, 0x01]);
    b.extend_from_slice(ops);
    b.push(0xFF);
    let n = b.len() as u16; // picSize is the low 16 bits for a picture over 32 KB
    b[..2].copy_from_slice(&n.to_be_bytes());
    b
}

fn store<U: GuestUi>(m: &mut Mac<U>, h: u32, bytes: &[u8]) -> Result<(), Refusal> {
    m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, bytes.len() as u32).map_err(|e| Refusal(format!("Picture: no memory for {} bytes (memFullErr {e})", bytes.len())))?;
    let p = m.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{h:#x} is not a live Picture handle")))?;
    m.mem.write_bytes(p, bytes).map_err(Refusal::mem("Picture"))
}

fn open_picture<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    if m.qd.pictures.contains_key(&port) { return Err(Refusal("OpenPicture: a picture is already open on this port (nested pictures are not supported)".into())); }
    let r = read_rect(&m.mem, c.args.get(0))?;
    let mut frame = [0u8; 8];
    for (i, v) in [r.top, r.left, r.bottom, r.right].into_iter().enumerate() { frame[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes()); }
    let h = m.mm.new_handle(&mut m.mem, &mut m.heap, 0, false).map_err(|e| Refusal(format!("OpenPicture: no memory (memFullErr {e})")))?;
    store(m, h, &image(&frame, &[]))?;
    m.mem.write_u32(port.wrapping_add(P_PIC_SAVE), h).map_err(Refusal::mem("picSave"))?;
    m.qd.pictures.insert(port, OpenPic { handle: h, frame, ops: Vec::new(), unrecorded: 0 });
    Ok(Done { result: h, log: format!("[quickdraw] OpenPicture ({},{},{},{}) -> {h:#x} (only comments are recorded)", r.top, r.left, r.bottom, r.right) })
}

fn close_picture<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let Some(p) = m.qd.pictures.remove(&port) else { return Err(Refusal("ClosePicture: no picture is open on this port".into())) };
    let bytes = image(&p.frame, &p.ops);
    store(m, p.handle, &bytes)?;
    m.mem.write_u32(port.wrapping_add(P_PIC_SAVE), 0).map_err(Refusal::mem("picSave"))?;
    if p.unrecorded > 0 { m.note_refusal("ClosePicture", format!("{} drawing call(s) were made while the picture was open and are not recorded in it", p.unrecorded)); }
    Ok(Done { result: 0, log: format!("[quickdraw] ClosePicture {:#x}: {} bytes, {} drawing call(s) not recorded", p.handle, bytes.len(), p.unrecorded) })
}

fn pic_comment<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let (kind, size, data) = (c.args.get(0) as u16, c.args.get(1) as u16, c.args.get(2));
    let Some(p) = m.qd.pictures.get_mut(&port) else { return Ok(Done { result: 0, log: format!("[quickdraw] PicComment {kind} (no picture is open: ignored)") }) };
    if size == 0 { p.ops.extend_from_slice(&[0xA0]); p.ops.extend_from_slice(&kind.to_be_bytes()); }
    else {
        let bytes = { let hb = m.mm.block(data).map(|b| (b.ptr, b.size)).ok_or_else(|| Refusal(format!("PicComment: {data:#x} is not a handle")))?; m.mem.span(hb.0, u32::from(size).min(hb.1)).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(Refusal::mem("PicComment data"))? };
        let p = m.qd.pictures.get_mut(&port).ok_or_else(|| Refusal("PicComment: the picture closed".into()))?;
        p.ops.push(0xA1);
        p.ops.extend_from_slice(&kind.to_be_bytes());
        p.ops.extend_from_slice(&size.to_be_bytes());
        p.ops.extend_from_slice(&bytes);
        if bytes.len() % 2 == 1 { p.ops.push(0); }
    }
    Ok(Done { result: 0, log: format!("[quickdraw] PicComment kind {kind}, {size} bytes recorded") })
}

fn kill_picture<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if h == 0 { return Ok(Done { result: 0, log: "[quickdraw] KillPicture NIL".into() }); }
    m.dispose_handle(h).map_err(|e| Refusal(format!("KillPicture: {h:#x} is not a handle (memWZErr {e})")))?;
    Ok(Done { result: 0, log: format!("[quickdraw] KillPicture {h:#x}") })
}

/// `DrawPicture(pic, dstRect)`: a picture with no drawing ops draws nothing (and is accepted); the rest is refused by name.
fn draw_picture<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, dst) = (c.args.get(0), c.args.get(1));
    let bytes = m.handle_bytes(h).ok_or_else(|| Refusal(format!("DrawPicture: {h:#x} is not a Picture handle")))?;
    if bytes.len() < 13 || bytes[10] != 0x11 { return Err(Refusal(format!("DrawPicture {h:#x}: not a version-1 picture (first op {:#04x}); PICT v2 and drawing ops are not interpreted", bytes.get(10).copied().unwrap_or(0)))); }
    let mut at = 12;
    while at < bytes.len() {
        match bytes[at] {
            0xFF => { let r = read_rect(&m.mem, dst)?; return Ok(Done { result: 0, log: format!("[quickdraw] DrawPicture {h:#x} into ({},{},{},{}): no drawing ops", r.top, r.left, r.bottom, r.right) }); }
            0xA0 => at += 3,
            0xA1 => { let n = bytes.get(at + 3..at + 5).map_or(0, |b| usize::from(u16::from_be_bytes([b[0], b[1]]))); at += 5 + n + (n & 1); }
            op => return Err(Refusal(format!("DrawPicture {h:#x}: picture opcode {op:#04x} is not interpreted (only empty pictures replay)"))),
        }
    }
    Err(Refusal(format!("DrawPicture {h:#x}: picture has no end opcode")))
}
