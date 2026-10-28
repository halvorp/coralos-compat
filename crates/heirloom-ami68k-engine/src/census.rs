//! Census exploration aid: `amiga68k-run --census --stub graphics.library/OpenFont` makes a named function that has no
//! implementation return a pointer to a zeroed block instead of stopping the run, so a real binary can be driven past
//! another lane's missing library to see what ELSE it needs. It is never used by tests of behaviour: a stub proves
//! nothing, and every stubbed call is still logged as one.
use alloc::format;
use alloc::string::String;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// What a stubbed call returns a pointer to: zeroed, big enough for the structs programs poke right after (TextFont,
/// DrawInfo, ...).
const STUB_BLOCK: u32 = 512;

pub fn stub<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = a.heap.alloc(STUB_BLOCK).unwrap_or(0);
    if p != 0 { a.mem.zero(p, STUB_BLOCK).map_err(Refusal::mem("census stub"))?; }
    cpu.set_d0(p);
    Ok(format!("[census] STUBBED call -> zeroed block {p:#x}"))
}
