//! Calling GUEST code from a host handler, and getting its D0 back, without nesting `resume` (Musashi has one CPU
//! and `resume` holds its lock).
//!
//! A handler runs while the CPU sits on a library slot's `ILLEGAL; RTS`. After the handler returns, the slot's RTS
//! pops the return address from the guest stack. So a handler can make that RTS land in guest code by rewriting the
//! top of the stack. [`start`] does this:
//!
//! ```text
//!   before (A7 -> R = the caller's return address)       after, for the RTS to pop
//!   sp0:  R                                              sp0:   R
//!                                                        sp1:   D2..D7, A2..A5   (the caller's registers, 40 bytes)
//!                                                        sp1-4: RET_SLOT         (where the callee returns to)
//!                                                        sp1-8: target           (the RTS jumps here)
//! ```
//!
//! The callee runs as a normal subroutine and returns into `RET_SLOT`, another `ILLEGAL; RTS` trap: [`on_return`]
//! checks the stack is balanced (a callee that leaves A7 elsewhere is a named refusal and halts the guest), takes D0,
//! and runs the pending continuation, which answers with the final D0 ([`Next::Return`]) or another call
//! ([`Next::Call`], so a sequence like "init, then Open" is a chain of continuations). Finally the registers are
//! restored by a 3-instruction guest stub (`MOVEM.L (A7)+,D2-D7/A2-A5; RTS` at `RESTORE_CODE`), so a callee may trash
//! any register, and the trapped call returns to R with the D0 the continuation chose. The callee may itself call
//! bridged functions, even start another guest call: frames are a stack and returns are LIFO.
//!
//! Only A-registers and D0 can be given to a callee (`Cpu` can set no other D register); library init, Open, Close
//! and Expunge need nothing more. A6 is restored to the caller's value at the end.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::libtable::{write_slot, RTS};
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_68k::Cpu;

/// The callee's return trap, and the register-restore stub. (0x500 is the exit slot; both are free low memory.)
pub const RET_SLOT: u32 = 0x510;
pub const RESTORE_CODE: u32 = 0x520;
/// `MOVEM.L (A7)+,D2-D7/A2-A5` = 4CDF 3CFC (mask: D2-D7 = bits 2-7, A2-A5 = bits 10-13), then RTS.
const MOVEM_POP: [u16; 2] = [0x4CDF, 0x3CFC];
const SAVED_BYTES: u32 = 40;

pub type Cont<U> = Box<dyn FnOnce(&mut Amiga<U>, &mut Cpu, u32) -> Result<Next<U>, Refusal>>;

/// What a handler (or a continuation) wants next.
pub enum Next<U: GuestUi> {
    /// Finish the trapped call with this D0.
    Return(u32),
    /// Run guest code at `target` with D0 = `d0` and the given `(A-register number, value)` pairs; when it returns,
    /// `then` is called with its D0.
    Call { target: u32, d0: u32, aregs: Vec<(u32, u32)>, then: Cont<U> },
}

/// Continue `next` with `f`, which receives the D0 `next` ends with: at once when `next` returns, otherwise appended to the last
/// continuation of the chain (so "open the library, then query it, then make the object" is one chain of guest calls).
pub fn then<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, next: Next<U>, f: Cont<U>) -> Result<Next<U>, Refusal> {
    match next {
        Next::Return(d) => f(a, cpu, d),
        Next::Call { target, d0, aregs, then: k } => Ok(Next::Call { target, d0, aregs, then: Box::new(move |a, cpu, d| { let n = k(a, cpu, d)?; then(a, cpu, n, f) }) }),
    }
}

pub struct Frame<U: GuestUi> { sp: u32, caller_a6: u32, then: Cont<U> }

/// Write the return trap and the restore stub into guest RAM.
pub fn install(mem: &mut GuestMem) -> Result<(), Refusal> {
    write_slot(mem, RET_SLOT).map_err(Refusal::mem("guest call return slot"))?;
    mem.write_u16(RESTORE_CODE, MOVEM_POP[0]).map_err(Refusal::mem("guest call restore stub"))?;
    mem.write_u16(RESTORE_CODE + 2, MOVEM_POP[1]).map_err(Refusal::mem("guest call restore stub"))?;
    mem.write_u16(RESTORE_CODE + 4, RTS).map_err(Refusal::mem("guest call restore stub"))
}

fn enter(cpu: &mut Cpu, mem: &mut GuestMem, sp1: u32, target: u32, d0: u32, aregs: &[(u32, u32)]) -> Result<(), Refusal> {
    let m = Refusal::mem("guest call stack");
    mem.write_u32(sp1 - 4, RET_SLOT).map_err(&m)?;
    mem.write_u32(sp1 - 8, target).map_err(&m)?;
    cpu.set_areg(7, sp1 - 8);
    cpu.set_d0(d0);
    for &(r, v) in aregs { cpu.set_areg(r, v); }
    Ok(())
}

/// Begin from inside a handler: `next` is the handler's answer. Returns once the stack is arranged; the trap's own
/// RTS then enters the callee.
pub fn start<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, next: Next<U>) -> Result<(), Refusal> {
    let (target, d0, aregs, then) = match next {
        Next::Return(d) => { cpu.set_d0(d); return Ok(()); }
        Next::Call { target, d0, aregs, then } => (target, d0, aregs, then),
    };
    let sp1 = cpu.areg(7).checked_sub(SAVED_BYTES).ok_or_else(|| Refusal("guest call: stack underflow".into()))?;
    let m = Refusal::mem("guest call register save");
    for i in 0..6 { a.mem.write_u32(sp1 + 4 * i, cpu.dreg(2 + i)).map_err(&m)?; }
    for i in 0..4 { a.mem.write_u32(sp1 + 24 + 4 * i, cpu.areg(2 + i)).map_err(&m)?; }
    let caller_a6 = cpu.areg(6); // before `enter` gives the callee its own A6
    enter(cpu, &mut a.mem, sp1, target, d0, &aregs)?;
    a.calls.push(Frame { sp: sp1, caller_a6, then });
    Ok(())
}

/// Leave through the restore stub with `d0`.
fn finish<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, sp1: u32, a6: u32, d0: u32) -> Result<(), Refusal> {
    a.mem.write_u32(sp1 - 4, RESTORE_CODE).map_err(Refusal::mem("guest call stack"))?;
    cpu.set_areg(7, sp1 - 4);
    cpu.set_areg(6, a6);
    cpu.set_d0(d0);
    Ok(())
}

/// The handler of [`RET_SLOT`]: a guest callee returned.
pub fn on_return<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let Some(frame) = a.calls.pop() else {
        cpu.halt();
        return Err(Refusal("guest call return trap hit with no call outstanding".into()));
    };
    let (sp, a6) = (frame.sp, frame.caller_a6);
    if cpu.areg(7) != sp {
        cpu.halt();
        return Err(Refusal(format!("guest callee returned with A7={:#x}, expected {sp:#x} (unbalanced stack)", cpu.areg(7))));
    }
    let d0 = cpu.dreg(0);
    match (frame.then)(a, cpu, d0) {
        Ok(Next::Return(r)) => { finish(a, cpu, sp, a6, r)?; Ok(format!("[guest] call returned {d0:#x}, trapped call finishes with {r:#x}")) }
        Ok(Next::Call { target, d0: d, aregs, then }) => {
            enter(cpu, &mut a.mem, sp, target, d, &aregs)?;
            a.calls.push(Frame { sp, caller_a6: a6, then });
            Ok(format!("[guest] call returned {d0:#x}, next call at {target:#x}"))
        }
        Err(r) => { finish(a, cpu, sp, a6, 0)?; Err(r) }
    }
}
