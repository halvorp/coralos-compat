//! Calling guest code from a trap handler, without re-entering the CPU: the handler asks, the A-line bridge arranges it.
//!
//! The 68k is entered as a `JSR` from the trapping instruction would: the return address (the word after the trap) is
//! pushed, the PC is redirected, and the guest routine's `RTS` comes back to the program. A call that must not lose the
//! trap's own result (a File Manager completion routine runs AFTER the trap has produced D0 / A0) returns through a
//! trampoline in low memory ([`TRAMPOLINE`]: `ILLEGAL; RTS`): the `ILLEGAL` is handled by the host, which puts the
//! saved D0 and A0 back, and the `RTS` then returns to the program. Patched traps (`_SetTrapAddress`) use the same
//! entry without the trampoline: the patch routine runs with the trap's registers and stack, and chains to the
//! original by `JMP`ing to the stub `GetTrapAddress` gave it.
use crate::machine::Mac;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// The completion trampoline: `ILLEGAL` (handled by the host) then `RTS`.
pub const TRAMPOLINE: u32 = 0x510;

/// One request to run guest code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestCall {
    /// The routine's address.
    pub target: u32,
    /// Registers the routine is entered with, if any (a completion routine gets D0 = ioResult, A0 = the block).
    pub d0: Option<u32>,
    pub a0: Option<u32>,
    /// D1 = the trap word (a patched OS trap receives it).
    pub d1: Option<u32>,
    /// Return through the trampoline, restoring the caller's D0 and A0 (the trap's own results) afterwards.
    pub restore: Option<(u32, u32)>,
}

/// What the trampoline's `ILLEGAL` undoes when a guest routine returns to it.
#[derive(Debug, Clone)]
pub(crate) enum Frame {
    /// A completion routine: put the trap's own D0 and A0 back.
    D0A0(u32, u32),
    /// A resource decompressor ([`crate::compressed`]): every register and the SR, then free the scratch.
    Decomp(crate::compressed::DecompDone),
    /// A vertical-retrace task ([`crate::vbl`]): every register and the SR.
    Vbl(crate::vbl::VblDone),
    /// An Apple Event handler ([`crate::appleevents`]): the program's frame is rebuilt around its result.
    Ae(crate::appleevents::AeDone),
}

impl<U: GuestUi> Mac<U> {
    /// Write the trampoline into low memory (once, at machine creation).
    pub(crate) fn install_trampoline(&mut self) -> Result<(), crate::Refusal> {
        self.mem.write_u16(TRAMPOLINE, 0x4AFC).and_then(|_| self.mem.write_u16(TRAMPOLINE + 2, 0x4E75)).map_err(crate::Refusal::mem("completion trampoline"))
    }

    /// Enter `call` as a JSR from the instruction at `pc` (return address `pc + 2`).
    pub(crate) fn enter_guest(&mut self, cpu: &mut Cpu, pc: u32, call: GuestCall) {
        let mut sp = cpu.areg(7);
        let mut push = |m: &mut Self, v: u32| { sp = sp.wrapping_sub(4); let _ = m.mem.write_u32(sp, v); };
        push(self, pc.wrapping_add(2));
        if let Some((d0, a0)) = call.restore { self.call_stack.push(Frame::D0A0(d0, a0)); push(self, TRAMPOLINE); }
        cpu.set_areg(7, sp);
        if let Some(v) = call.d0 { cpu.set_d0(v); }
        if let Some(v) = call.a0 { cpu.set_areg(0, v); }
        if let Some(v) = call.d1 { cpu.set_dreg(1, v); }
        cpu.set_pc(call.target);
    }

    /// The trampoline's `ILLEGAL`: undo what the call that returned here changed (its [`Frame`]).
    pub(crate) fn leave_guest(&mut self, cpu: &mut Cpu) -> bool {
        match self.call_stack.pop() {
            None => false,
            Some(Frame::D0A0(d0, a0)) => { cpu.set_d0(d0); cpu.set_areg(0, a0); true }
            Some(Frame::Decomp(done)) => { self.finish_decompress(cpu, done); true }
            Some(Frame::Vbl(done)) => { self.finish_vbl(cpu, done); true }
            Some(Frame::Ae(done)) => { self.finish_ae(cpu, done); true }
        }
    }
}
