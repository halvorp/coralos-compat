//! diskfont.library OpenDiskFont: a resident font is returned as OpenFont would; otherwise a named refusal until the
//! FONTS: loader lands in this file.
use super::fonts;
use crate::{machine::Amiga, Refusal};
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub fn open_disk_font<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ta = cpu.areg(0);
    let m = Refusal::mem("TextAttr");
    let name = a.mem.read_cstr(a.mem.read_u32(ta).map_err(&m)?).map_err(&m)?;
    let y = a.mem.read_u16(ta + 4).map_err(&m)?;
    match fonts::find_resident(a, &name, y)? {
        Some(tf) => { cpu.set_d0(tf); Ok(format!("[diskfont] OpenDiskFont {name:?} {y} -> resident {tf:#x}")) }
        None => { cpu.set_d0(0); Ok(format!("[diskfont] OpenDiskFont {name:?} {y} -> NULL (no such font)")) }
    }
}
