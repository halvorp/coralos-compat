//! `ModalDialog`'s filterProc (Inside Macintosh I, "The Dialog Manager"): `FUNCTION Filter(theDialog: DialogPtr; VAR theEvent:
//! EventRecord; VAR itemHit: INTEGER): BOOLEAN`. The filter is guest code, so the call is a RUN through the completion
//! trampoline ([`crate::guestcall`]), the same machine the user-item draw procedures use ([`super::useritems`]): the program's
//! registers, status register, `thePort` and stack are saved, the function runs with `thePort` = the dialog, and then
//! * TRUE: `ModalDialog` is over. The filter's `itemHit` is stored in the program's own `VAR itemHit`, the trap's arguments are
//!   popped, and the program continues after the trap.
//! * FALSE: the program is put back at the trap word with its arguments, and `ModalDialog` runs again with the filter marked as
//!   called ([`super::dialogs::State::filter_done`]), so the standard handling (the wait for a native control's hit) follows.
//!
//! The event the filter is given is a null event (`what` = 0, `when` = `Ticks`): the host's controls and text fields consume the
//! keys and clicks before the Dialog Manager sees them (see the dialogs module), so there is nothing else to hand over, and the
//! filter is called ONCE per `ModalDialog` call, before the wait; it is not called again while the call stays blocked. That is
//! the limit of this model, and what it can answer is an idle-time or "dismiss at once" filter.
//!
//! A PowerPC caller (no 68k CPU to schedule on) keeps the named refusal: its filterProc is a UPP run by macppc-engine.
use crate::guestcall::{Frame, TRAMPOLINE};
use crate::machine::Mac;
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// A filter call waiting to start (`pending`) or running (the [`Frame`]).
#[derive(Debug, Clone)]
pub(crate) struct Filter {
    win: u32,
    proc: u32,
    /// The program's `VAR itemHit`.
    item_out: u32,
    /// Where the trap word is and where its arguments are (the program resumes there on FALSE).
    trap_pc: u32,
    args_sp: u32,
    saved: Option<(Saved, [u32; 15], u32)>,
    /// Guest scratch: the BOOLEAN result slot, the EventRecord, the itemHit word.
    result_slot: u32,
    event: u32,
    item: u32,
}

use super::useritems::Saved;

impl<U: GuestUi> Mac<U> {
    /// Queue the filter call for the A-line bridge, which starts it once the trap has finished.
    pub(crate) fn schedule_modal_filter(&mut self, cpu: &Cpu, win: u32, proc: u32, item_out: u32) -> Result<(), Refusal> {
        let _ = cpu.pc(); // reading a register marks a CPU-less caller (the PowerPC glue)
        if cpu.touched_registers() { return Err(Refusal(format!("filterProc {proc:#x} is a guest callback the PowerPC glue cannot run from here; the standard filtering applies"))); }
        let trap_pc = self.trap_pc;
        if crate::traptable::in_stub(trap_pc) { return Err(Refusal(format!("filterProc {proc:#x}: ModalDialog was reached through a GetTrapAddress stub, it cannot be run again after the filter; the standard filtering applies"))); }
        self.pending_modal_filter = Some(Filter { win, proc: self.mem.strip(proc), item_out, trap_pc, args_sp: cpu.areg(7), saved: None, result_slot: 0, event: 0, item: 0 });
        Ok(())
    }

    /// The bridge finished the trap that scheduled a filter: save the program and call it.
    pub(crate) fn enter_modal_filter(&mut self, cpu: &mut Cpu, mut f: Filter) {
        let saved = Saved::capture(self, cpu, f.trap_pc, f.args_sp);
        let mut sp = f.args_sp.wrapping_sub(18);
        f.event = sp;
        f.item = sp + 16;
        let ticks = self.mem.read_u32(crate::lowmem::TICKS).unwrap_or(0);
        let _ = self.mem.write_bytes(f.event, &[0u8; 18]).and_then(|_| self.mem.write_u32(f.event + 6, ticks));
        let push = |m: &mut Self, sp: &mut u32, v: u32, long: bool| { *sp = sp.wrapping_sub(if long { 4 } else { 2 }); let _ = if long { m.mem.write_u32(*sp, v) } else { m.mem.write_u16(*sp, v as u16) }; };
        push(self, &mut sp, 0, false);
        f.result_slot = sp;
        push(self, &mut sp, f.win, true);
        push(self, &mut sp, f.event, true);
        push(self, &mut sp, f.item, true);
        push(self, &mut sp, TRAMPOLINE, true);
        if let Err(Refusal(e)) = crate::quickdraw::set_the_port(self, f.win) { self.note_refusal("ModalDialog", e); }
        f.saved = Some((saved.0, saved.1, saved.2));
        cpu.set_areg(7, sp);
        self.log.push(format!("[dialog] ModalDialog {:#x}: the program's filterProc {:#x} runs (null event)", f.win, f.proc));
        cpu.set_pc(f.proc);
        self.call_stack.push(Frame::ModalFilter(f));
    }

    /// The filter returned to the trampoline.
    pub(crate) fn finish_modal_filter(&mut self, cpu: &mut Cpu, f: Filter) {
        let Some((saved, regs, port)) = f.saved.clone() else { return };
        let verdict = self.mem.read_u8(f.result_slot).unwrap_or(0) != 0;
        let item = self.mem.read_u16(f.item).unwrap_or(0);
        saved.restore(self, cpu, &regs, port);
        if verdict {
            if let Err(Refusal(e)) = self.mem.write_u16(f.item_out, item).map_err(Refusal::mem("itemHit")) { self.note_refusal("ModalDialog", e); }
            self.tr.dlg.modal = None;
            self.tr.dlg.filter_done = false;
            cpu.set_areg(7, f.args_sp.wrapping_add(8)); // filterProc and itemHit: the Pascal arguments of ModalDialog
            cpu.set_pc(f.trap_pc.wrapping_add(2));
            self.log.push(format!("[dialog] ModalDialog {:#x}: the filterProc answered TRUE, itemHit {item}", f.win));
        } else {
            self.tr.dlg.filter_done = true;
            self.log.push(format!("[dialog] ModalDialog {:#x}: the filterProc answered FALSE, the standard handling follows", f.win));
        }
    }
}
