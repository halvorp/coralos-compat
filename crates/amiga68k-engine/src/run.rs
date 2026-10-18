//! Driving the guest: start it, step it, deliver host events to it (docs/SLICE-2-ENGINE.md, Astra 2026-10-17
//! option a: the guest is a resumable state machine driven by the HOST's loop, no second thread and no second loop).
//!
//! - [`Amiga::start_hunk`] loads a Hunk executable and positions the CPU at its entry, as an AmigaDOS CLI start
//!   (A0 = the argument line, D0 = its length), returning to the exit slot.
//! - [`Amiga::step`] runs at most `budget` instructions. A guest blocked in Wait / WaitPort executes NOTHING until its
//!   wait is satisfied; then D0 is set to the call's result and the guest continues after the trap.
//! - [`Amiga::deliver`] turns a native event into what the guest expects (an IntuiMessage on the window's UserPort).
//!   It never runs guest code: the next `step` does.
//! - [`Amiga::run_hunk`] is one driver over those (tests, the census runner): it steps, and while the guest is blocked
//!   with only timers pending, advances the virtual clock to the next deadline.
use alloc::string::{String, ToString};
use alloc::format;
use crate::machine::{Amiga, RunReport, EMPTY_ARGS, EXEC_BASE, EXIT_SLOT, LOAD_BASE, STACK_SIZE, STACK_TOP};
use crate::{dos, exec_wait, process, screen, Refusal, Step};
use guestui_abi::{GuestUi, GuestUiEvent};
use retro_hunk::Loaded;
use retro_m68k::RunStats;

/// What [`Amiga::deliver`] did with an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered { Queued(u32), IgnoredUnsubscribed, NotForAGuestWindow, /// A MUI object took it (mui::try_deliver).
    Handled }

use crate::idcmp::INTUIMESSAGE_SIZE;

impl<U: GuestUi> Amiga<U> {
    pub fn start_hunk(&mut self, file: &[u8]) -> Result<Loaded, String> {
        let loaded = retro_hunk::load(&mut self.mem, file, LOAD_BASE).map_err(|e| e.to_string())?;
        self.mem.write_u32(STACK_TOP - 4, EXIT_SLOT).map_err(|e| e.to_string())?; // return address for the final rts
        process::setup(&mut self.mem, EXEC_BASE, STACK_TOP - STACK_SIZE, STACK_TOP, "program", EMPTY_ARGS).map_err(|r| r.0)?;
        dos::setup(&mut self.mem).map_err(|r| r.0)?;
        screen::setup(&mut self.mem).map_err(|r| r.0)?;
        crate::graphics::setup(self).map_err(|r| r.0)?;
        crate::doslist::sync(self).map_err(|r| r.0)?; // the volumes are mounted by now: publish them in DosInfo
        let regs = [(retro_m68k::REG_A0, process::ARGS), (retro_m68k::REG_D0, EMPTY_ARGS.len() as u32)];
        self.ctx = Some(retro_m68k::start_with(self, loaded.entry, STACK_TOP - 4, &regs));
        self.waiting = None;
        Ok(loaded)
    }

    /// Run at most `budget` instructions; the stats of what ran come back too.
    pub fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let idle = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: true, stalled_at: None };
        let Some(mut ctx) = self.ctx.take() else { return (Step::Exited, RunStats { blocked: false, halted: true, ..idle }) };
        if let Some(w) = self.waiting {
            match self.try_satisfy(w) {
                // Pointer input for a guest gadget: send the guest back through its own Wait trap (boopsi_input.rs).
                Ok(None) if self.input.pending() && self.input.wait_trap != 0 => {
                    // the guest calls in between trashed D0/A0, which carry the wait's argument: put it back
                    match w { exec_wait::Waiting::Signals(set) => ctx.set_reg(retro_m68k::REG_D0, set), exec_wait::Waiting::Port(p) => ctx.set_reg(retro_m68k::REG_A0, p) }
                    ctx.set_reg(retro_m68k::REG_PC, self.input.wait_trap);
                    self.waiting = None;
                }
                Ok(None) => { self.ctx = Some(ctx); return (Step::Blocked, idle); }
                Ok(Some(v)) => { ctx.set_reg(retro_m68k::REG_D0, v); self.waiting = None; }
                Err(Refusal(e)) => { self.ctx = Some(ctx); return (Step::Fault(e), idle); }
            }
        }
        let stats = retro_m68k::resume(self, &mut ctx, budget);
        crate::graphics::content::flush(self);
        let step = if stats.halted { Step::Exited } else if stats.blocked { Step::Blocked }
            else if let Some(pc) = stats.stalled_at { Step::Fault(format!("the 68k stopped making progress at pc={pc:#x} (STOP, or halted after a double fault)")) }
            else { Step::BudgetSpent };
        if step != Step::Exited { self.ctx = Some(ctx); }
        (step, stats)
    }

    /// A native event for the guest (idcmp.rs): a close request, a menu command or a control change becomes an
    /// IntuiMessage on the window's UserPort if the window subscribes to that class, as on AmigaOS.
    pub fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivered, Refusal> { crate::idcmp::deliver(self, ev) }

    /// GetMsg handed `msg` to the guest.
    pub(crate) fn delivered_msg(&mut self, msg: u32) { if let Some(d) = self.engine_msgs.get_mut(&msg) { *d = true; } }

    /// ReplyMsg of an engine message: give it back. Ok(false) = not an engine message.
    pub(crate) fn reclaim_engine_msg(&mut self, msg: u32) -> Result<bool, Refusal> {
        match self.engine_msgs.get(&msg) {
            None => Ok(false),
            Some(false) => Err(Refusal(format!("ReplyMsg {msg:#x}: still queued, never received with GetMsg"))),
            Some(true) => {
                self.engine_msgs.remove(&msg);
                self.heap.free(msg, INTUIMESSAGE_SIZE).map_err(|e| Refusal(format!("ReplyMsg: {e}")))?;
                Ok(true)
            }
        }
    }

    /// A window is closing: its queued (never received) engine messages go back; a received but unreplied one is the
    /// guest's bug, refused by name rather than freed under it.
    pub(crate) fn drop_window_msgs(&mut self, win: u32, port: u32) -> Result<(), Refusal> {
        for (&msg, &received) in &self.engine_msgs {
            if received && self.mem.read_u32(msg + 44).map_err(Refusal::mem("IDCMPWindow"))? == win {
                return Err(Refusal(format!("CloseWindow: IntuiMessage {msg:#x} received but not replied")));
            }
        }
        loop {
            let head = exec_wait::port_head(self, port)?;
            if head == 0 { break; }
            exec_wait::remove(self, head)?;
            if self.engine_msgs.remove(&head).is_some() { self.heap.free(head, INTUIMESSAGE_SIZE).map_err(|e| Refusal(format!("CloseWindow: {e}")))?; }
        }
        Ok(())
    }

    /// Load and run to the end or `max_insns`, advancing the virtual clock while the guest waits only on timers.
    pub fn run_hunk(&mut self, file: &[u8], max_insns: u64) -> Result<RunReport, String> {
        let loaded = self.start_hunk(file)?;
        let mut total = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: false, stalled_at: None };
        loop {
            let (step, s) = self.step(max_insns.saturating_sub(total.insns));
            total.insns += s.insns; total.bus_faults += s.bus_faults;
            match step {
                Step::Exited => { total.halted = true; break; }
                Step::Blocked => match self.next_timer_deadline() {
                    Some(t) if total.insns < max_insns => self.advance_clock_to(t).map_err(|r| r.0)?,
                    _ => { total.blocked = true; break; }
                },
                Step::BudgetSpent => break,
                Step::Fault(e) => return Err(e),
            }
            if total.insns >= max_insns { break; }
        }
        Ok(RunReport { stats: total, loaded })
    }
}
