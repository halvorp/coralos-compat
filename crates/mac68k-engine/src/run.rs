//! Driving the guest, same shape as amiga68k-engine's run.rs: the guest is a resumable state machine driven by the
//! HOST's loop (no second thread, no second loop).
//!
//! - [`Mac::launch`] (segment.rs) loads an application from its resource fork; [`Mac::start`] places a flat code image
//!   at [`CODE_BASE`] (test programs without a fork).
//! - [`Mac::step`] runs at most `budget` instructions; a guest waiting for an event reports `Step::Blocked` without
//!   running until a matching event can be delivered (events.rs).
//! - [`Mac::deliver`] turns a native event into what the guest expects. It never runs guest code.
//! - [`Mac::run_program`] is one driver over those (tests, a census runner).
use alloc::format;
use alloc::string::{String, ToString};
use crate::machine::{Mac, CODE_BASE, EXIT_STUB, STACK_TOP};
use crate::{Step};
use guestui_abi::GuestUi;
use retro_m68k::RunStats;

pub struct RunReport { pub stats: RunStats }

impl<U: GuestUi> Mac<U> {
    pub fn start(&mut self, code: &[u8], entry: u32) -> Result<(), String> {
        if entry as usize >= code.len() { return Err(format!("entry {entry:#x} is outside the {}-byte image", code.len())); }
        self.mem.write_bytes(CODE_BASE, code).map_err(|e| e.to_string())?;
        self.mem.write_u32(STACK_TOP - 4, EXIT_STUB).map_err(|e| e.to_string())?; // return address for the program's final rts
        self.ev.waiting = None; // a new program replaces the old one, including its blocked event call
        self.tr.dlg.modal = None; // ... and its blocked ModalDialog
        self.tr.dlg.filter_wait = None;
        if let Some(a) = self.tr.dlg.asked.take() { let _ = self.ui.close_request(a.req); } // ... and an alert it asked and nobody answered
        self.ctx = Some(retro_m68k::start_model(self, CODE_BASE + entry, STACK_TOP - 4, &[], self.profile.cpu));
        Ok(())
    }

    /// Run at most `budget` instructions; the stats of what ran come back too.
    pub fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let idle = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: true, stalled_at: None };
        if self.ctx.is_none() { return (Step::Exited, RunStats { blocked: false, halted: true, ..idle }); }
        if !self.complete_wait() { return (Step::Blocked, idle); }
        let Some(mut ctx) = self.ctx.take() else { return (Step::Exited, RunStats { blocked: false, halted: true, ..idle }) };
        let stats = retro_m68k::resume(self, &mut ctx, budget);
        crate::qd::flush(self);
        let step = if stats.halted { Step::Exited } else if stats.blocked { Step::Blocked }
            else if let Some(pc) = stats.stalled_at { Step::Fault(format!("the 68k stopped making progress at pc={pc:#x} (STOP, or halted after a double fault)")) }
            else { Step::BudgetSpent };
        if step != Step::Exited { self.ctx = Some(ctx); }
        (step, stats)
    }

    /// Load and run to the end or `max_insns`.
    pub fn run_program(&mut self, code: &[u8], entry: u32, max_insns: u64) -> Result<RunReport, String> {
        self.start(code, entry)?;
        self.run_loop(max_insns)
    }

    /// Step until the guest exits, blocks for an event, faults or `max_insns` is spent.
    pub fn run_loop(&mut self, max_insns: u64) -> Result<RunReport, String> {
        let mut total = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: false, stalled_at: None };
        loop {
            let (step, s) = self.step(max_insns.saturating_sub(total.insns));
            total.insns += s.insns; total.bus_faults += s.bus_faults;
            match step {
                Step::Exited => { total.halted = true; break; }
                Step::Blocked => { total.blocked = true; break; }
                Step::BudgetSpent => if total.insns >= max_insns { break },
                Step::Fault(e) => return Err(e),
            }
        }
        Ok(RunReport { stats: total })
    }
}
