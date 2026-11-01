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
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

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

/// A stack-selector routine (`_Pack0`'s List Manager, `_TEDispatch` ...) called BY VALUE (Mixed Mode: a PowerPC program passes its C
/// arguments in registers). The frame a 68k caller would have pushed -- the arguments in declaration order ([`pascal::write_args`]),
/// then the selector word -- is built in scratch guest memory and its address handed to `body`, the SAME selector body the 68k trap
/// runs on the program's own stack. The scratch is freed whatever `body` answered.
pub fn by_value<U: GuestUi, T>(m: &mut Mac<U>, sel: u16, layout: &[Arg], values: &[u32], body: impl FnOnce(&mut Mac<U>, u32) -> Result<T, Refusal>) -> Result<T, Refusal> {
    let bytes = 2 + pascal::frame_size(layout) + 4; // selector, arguments, the largest result slot
    let sp = m.mm.new_ptr(&mut m.mem, &mut m.heap, bytes, true).map_err(|e| Refusal(format!("selector {sel:#x} by value: no memory for its {bytes}-byte frame ({e})")))?;
    let out = m.mem.write_u16(sp, sel).map_err(Refusal::mem("selector frame"))
        .and_then(|_| pascal::write_args(&mut m.mem, sp + 2, layout, values))
        .and_then(|_| body(m, sp));
    let _ = m.mm.dispose_ptr(&mut m.heap, sp);
    out
}

/// `InitAllPacks` and `InitPack(packID)`: every package is resident (there is no disk to load it from), so both are accepted.
pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9E6, &[], None, resident::<U>)?;
    m.register(0xA9E5, &[Arg::W], None, resident::<U>)
}

fn resident<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[pack] {} (packages are resident)", c.name) })
}

#[cfg(test)]
mod tests {
    use crate::pascal::frame_size;

    /// A by-value caller pushes exactly the frame the 68k trap pops: every stack-selector table's argument layout must add up to the
    /// frame size the same table gives the trap (a wrong layout would read a by-value caller's arguments from the wrong slots).
    #[test]
    fn every_selector_layout_fills_exactly_the_frame_its_trap_pops() {
        for f in crate::translate::lists::FRAMES { assert_eq!(2 + frame_size(f.args), f.pop, "_Pack0 {}", f.name); }
        for (i, &(sel, name, params)) in crate::textedit::styled::SEL_FRAMES.iter().enumerate() {
            assert_eq!(sel as usize, i);
            assert_eq!(frame_size(crate::textedit::styled::SEL_ARGS[i]), params, "_TEDispatch {name}");
        }
        assert_eq!(crate::translate::lists::frame_named("LSetDrawingMode").map(|f| f.sel), Some(0x2C), "the Carbon name finds LDoDraw");
        assert!(crate::translate::lists::frame_named("LNoSuchCall").is_none());
    }
}
