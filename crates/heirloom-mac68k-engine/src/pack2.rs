//! `_Pack2`, the Disk Initialization Package (DILoad, DIUnload, DIBadMount, DIFormat, DIVerify, DIZero). A program asks for
//! it to initialise a disk; there is no disk to initialise here, so the package loads and unloads (they are no-ops, as on a
//! Mac where it is already resident) and every call that would touch a disk is a NAMED REFUSAL with an honest result code
//! (`ioErr` for format / verify / zero, `userCanceledErr` for DIBadMount: the user declined to initialise).
//!
//! The Pack traps pass the SELECTOR as the last word pushed, so the frame is `args..., selector`: the handler reads the
//! selector at SP and pops the argument bytes that selector's routine takes (Inside Macintosh IV, "Disk Initialization Package").
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const IO_ERR: i16 = -36;
const USER_CANCELED_ERR: i16 = -128;

/// (selector, name, argument bytes after the selector, has a 2-byte result, result when refused).
const SELECTORS: [(u16, &str, u32, bool, i16); 6] = [
    (0, "DIBadMount", 8, true, USER_CANCELED_ERR), // where: Point (4), evtMessage: LONGINT (4)
    (1, "DILoad", 0, false, 0),
    (2, "DIUnload", 0, false, 0),
    (5, "DIVerify", 2, true, IO_ERR), // drvNum
    (6, "DIFormat", 2, true, IO_ERR), // drvNum
    (7, "DIZero", 6, true, IO_ERR),   // drvNum (2), volName: StringPtr (4)
];

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA9E9, pack2::<U>) }

fn pack2<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let sp = c.areg(7);
    let sel = m.mem.read_u16(sp).map_err(Refusal::mem("Pack2 selector"))?;
    let Some(&(_, name, args, func, refused)) = SELECTORS.iter().find(|s| s.0 == sel) else {
        return Err(Refusal(format!("Pack2 selector {sel} is not a Disk Initialization Package routine")));
    };
    let new_sp = sp + 2 + args;
    if func { m.mem.write_u16(new_sp, refused as u16).map_err(Refusal::mem("Pack2 result"))?; }
    c.set_areg(7, new_sp);
    r.d0 = 0;
    if refused == 0 { return Ok(format!("[toolbox] {name} (the package is resident: nothing to do)")); }
    m.note_refusal(name, format!("disk initialisation is not modelled; the call returns {refused}"));
    Ok(format!("[toolbox] {name} REFUSED -> {refused}"))
}
