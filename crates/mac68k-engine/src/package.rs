//! The Package Manager's calling convention, implemented once. `_Pack0`..`_Pack7` are OS-range register traps whose callee is
//! told WHICH routine by a selector word pushed LAST (Inside Macintosh I-"The Package Manager"): at the trap SP points at the
//! selector, the routine's Pascal arguments lie above it (last argument first), then its result slot. A package routine is
//! therefore a Pascal signature plus the selector; this module reads the selector, looks the routine up, reads the arguments
//! with [`crate::pascal`], runs the handler, pops selector and arguments, and stores the result.
//!
//! A selector the package does not have cannot be popped (its argument size is unknown), so the guest is halted with a NAMED
//! refusal rather than being left with a corrupt stack.
use crate::machine::{Call, Done, Handler, Mac};
use crate::osconv::Regs;
use crate::pascal::{self, Arg};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// One package routine: its Pascal signature and handler.
pub struct Routine<U: GuestUi> { pub name: &'static str, pub args: &'static [Arg], pub result: Option<Arg>, pub run: Handler<U> }
impl<U: GuestUi> Clone for Routine<U> { fn clone(&self) -> Self { *self } }
impl<U: GuestUi> Copy for Routine<U> {}

/// Dispatch one package trap through `lookup` (selector -> routine).
pub fn call<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, r: &mut Regs, pkg: &'static str, lookup: fn(u16) -> Option<Routine<U>>) -> Result<String, Refusal> {
    let sp = cpu.areg(7);
    let sel = m.mem.read_u16(sp).map_err(Refusal::mem("package selector"))?;
    let Some(rt) = lookup(sel) else {
        cpu.halt();
        return Err(Refusal(format!("{pkg} selector {sel:#x} is not a routine this engine implements (the guest is stopped: its argument size is unknown)")));
    };
    let args = pascal::read_args(&m.mem, sp + 2, rt.args)?;
    // A refusal by a known routine still pops its frame (the selector fixes the size): the program goes on with result 0.
    let (result, log) = match (rt.run)(m, cpu, &Call { opcode: r.opcode, pc: 0, name: rt.name, args }) {
        Ok(Done { result, log }) => (result, log),
        Err(Refusal(why)) => { m.note_refusal(rt.name, why); (0, format!("[{pkg}] {} REFUSED", rt.name)) }
    };
    let new_sp = pascal::complete(&mut m.mem, sp + 2, rt.args, rt.result.map(|k| (k, result)))?;
    cpu.set_areg(7, new_sp);
    r.d0 = 0;
    Ok(log)
}

/// `InitAllPacks` and `InitPack(packID)`: every package is resident (there is no disk to load it from), so both are accepted.
pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9E6, &[], None, resident::<U>)?;
    m.register(0xA9E5, &[Arg::W], None, resident::<U>)
}

fn resident<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[pack] {} (packages are resident)", c.name) })
}
