//! `PaletteDispatch` ($AAA2): the Palette Manager's selector-dispatched routines, of which Color QuickDraw programs need
//! the display-depth pair first: `HasDepth` ($0A14) and `SetDepth` ($0A13) (Imaging With QuickDraw, "Graphics Devices";
//! executor `src/quickdraw/qGDevice.cpp` `C_HasDepth` / `C_SetDepth` for the arguments and the main-device rule).
//!
//! `D0.W` is the selector: the high byte is the byte count of the arguments, the low byte the routine. The frame is the
//! arguments (the last nearest SP) and above them the function's result slot, so a routine this engine does not model has a
//! frame of KNOWN size, but its result and effect are not: it is a named refusal that stops the guest, never a made-up
//! answer.
//!
//! `FUNCTION HasDepth(gd: GDHandle; depth, whichFlags, flags: INTEGER): INTEGER` is non-zero when the device can show `depth`
//! bits per pixel. The screen device is the only one that can, at 1, 2, 4 and 8 bits (the engine's rasteriser depths);
//! another device, or a deeper depth, answers 0. `FUNCTION SetDepth(...): INTEGER` returns an error code: 0 when the device is
//! already at `depth`; switching the screen's depth is not modelled, so any other depth is a refusal (`paramErr` is the
//! answer the caller sees, and the guest goes on).
use super::pixmap::supported_depth;
use crate::machine::Mac;
use crate::osconv::{Regs, PARAM_ERR};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

const SET_DEPTH: u16 = 0x0A13;
const HAS_DEPTH: u16 = 0x0A14;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xAAA2, palette_dispatch::<U>) }

fn palette_dispatch<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let sel = r.d0 as u16;
    let sp = c.areg(7);
    let e = Refusal::mem("PaletteDispatch frame");
    match sel {
        HAS_DEPTH | SET_DEPTH => {
            // gd (sp+6), depth (sp+4), whichFlags (sp+2), flags (sp+0); the INTEGER result lies above, at sp+10
            let (gd, depth) = (m.mem.read_u32(sp + 6).map_err(&e)?, m.mem.read_u16(sp + 4).map_err(&e)?);
            let (name, answer) = if sel == HAS_DEPTH { ("HasDepth", has_depth(m, gd, depth)) } else { ("SetDepth", set_depth(m, gd, depth) as u16) };
            m.mem.write_u16(sp + 10, answer).map_err(&e)?;
            c.set_areg(7, sp + 10);
            r.d0 = 0;
            Ok(format!("[palette] {name} {gd:#x} depth {depth} -> {answer}"))
        }
        other => {
            c.halt();
            Err(Refusal(format!("PaletteDispatch selector {other:#06x} is not a routine this engine implements (the guest is stopped)")))
        }
    }
}


/// `HasDepth(gd, depth, ...)`: non-zero when the device can show `depth` bits per pixel (the PowerPC adapter calls this too).
pub fn has_depth<U: GuestUi>(m: &Mac<U>, gd: u32, depth: u16) -> u16 { u16::from(gd == m.cqd.main_device && gd != 0 && supported_depth(depth)) }

/// `SetDepth(gd, depth, ...)`: 0, or `paramErr` with the refusal recorded by name.
pub fn set_depth<U: GuestUi>(m: &mut Mac<U>, gd: u32, depth: u16) -> i16 {
    if gd != m.cqd.main_device || gd == 0 {
        m.note_refusal("SetDepth", format!("device {gd:#x} is not the main screen device"));
        PARAM_ERR
    } else if u16::from(super::gdevice::SCREEN_DEPTH) == depth {
        0
    } else {
        m.note_refusal("SetDepth", format!("switching the screen to {depth} bits is not modelled (the main device stays {} bits deep)", super::gdevice::SCREEN_DEPTH));
        PARAM_ERR
    }
}
