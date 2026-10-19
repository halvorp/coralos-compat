//! intuition.library ModifyProp / NewModifyProp on a classic proportional gadget (GTYP_PROPGADGET): the program hands over the new
//! PropInfo (flags, pots, bodies), the engine stores it in the guest's `struct PropInfo` and the gadget's NATIVE ScrollBar is brought
//! up to date through the same pots-and-bodies conversion the registration uses ([`crate::native_gadgets::scroll_of`]: total 0xFFFF,
//! visible = body, top = pot x (total - visible) / 0xFFFF), so the host control moves exactly where the knob would have.
//! Facts: AROS rom/intuition modifyprop.c:68-89 and newmodifyprop.c:85-123 (APL, re-expressed). Both do nothing unless the gadget's
//! type is GTYP_PROPGADGET with a SpecialInfo and a window is given (a propgclass gadget is GTYP_CUSTOMGADGET: newmodifyprop.c:57-63);
//! NewModifyProp refreshes `numGad` gadgets from the prop gadget (0 = none, not even this one; -1 = all), ModifyProp all the rest of
//! the list.
use alloc::format;
use alloc::string::String;
use crate::boopsi_ids::*;
use crate::native_gadgets::{self, PI_FLAGS, PI_HBODY, PI_HPOT, PI_VBODY, PI_VPOT};
use crate::{boopsi_glist, machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// Store the five PropInfo words, then bring the native controls of the first `num` gadgets from the prop gadget up to date and
/// refresh the BOOPSI ones among them (RefreshGList's job).
fn modify<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, num: i32, what: &'static str, [flags, hpot, vpot, hbody, vbody]: [u32; 5]) -> Result<String, Refusal> {
    let (g, win) = (cpu.areg(0), cpu.areg(1));
    let ty = a.mem.read_u16(g + GAD_TYPE).map(u32::from).map_err(Refusal::mem("Gadget.GadgetType"))?;
    let pi = a.mem.read_u32(g + GAD_SPECIAL).map_err(Refusal::mem("Gadget.SpecialInfo"))?;
    if ty & GTYP_GTYPEMASK != GTYP_PROPGADGET || pi == 0 || win == 0 { return Ok(format!("[intuition] {what} {g:#x}: not a GTYP_PROPGADGET with a PropInfo and a window: nothing to do")); }
    let e = Refusal::mem("PropInfo");
    for (off, v) in [(PI_FLAGS, flags), (PI_HPOT, hpot), (PI_VPOT, vpot), (PI_HBODY, hbody), (PI_VBODY, vbody)] { a.mem.write_u16(pi + off, v as u16).map_err(&e)?; }
    if num == 0 { return Ok(format!("[intuition] {what} {g:#x}: PropInfo stored, numGad 0 refreshes nothing")); }
    for x in boopsi_glist::chain(a, g, num)? { native_gadgets::resync(a, x)?; }
    boopsi_glist::refresh(a, cpu, num, what)
}

/// NewModifyProp(a0 = gadget, a1 = window, a2 = requester, d0 = flags, d1 = horizPot, d2 = vertPot, d3 = horizBody, d4 = vertBody, d5 = numGad).
pub fn new_modify_prop<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let w = |n: u32| cpu.dreg(n) & 0xFFFF;
    let (words, num) = ([w(0), w(1), w(2), w(3), w(4)], cpu.dreg(5) as i16 as i32);
    modify(a, cpu, num, "NewModifyProp", words)
}

/// ModifyProp(a0 = gadget, a1 = window, a2 = requester, d0 = flags, d1 = horizPot, d2 = vertPot, d3 = horizBody, d4 = vertBody).
pub fn modify_prop<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let w = |n: u32| cpu.dreg(n) & 0xFFFF;
    let words = [w(0), w(1), w(2), w(3), w(4)];
    modify(a, cpu, -1, "ModifyProp", words)
}
