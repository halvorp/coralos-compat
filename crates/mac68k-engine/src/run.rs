//! Driving the guest, same shape as amiga68k-engine's run.rs: the guest is a resumable state machine driven by the
//! HOST's loop (no second thread, no second loop).
//!
//! - [`Mac::start`] places a flat code image at [`CODE_BASE`] and positions the CPU at `CODE_BASE + entry`, returning
//!   to the exit stub. (A resource-fork `CODE` loader replaces this.)
//! - [`Mac::step`] runs at most `budget` instructions.
//! - [`Mac::deliver`] turns a native event into what the guest expects. It never runs guest code.
//! - [`Mac::run_program`] is one driver over those (tests, a census runner).
use alloc::string::{String, ToString};
use alloc::format;
use crate::machine::{Mac, CODE_BASE, EXIT_STUB, STACK_TOP};
use crate::{Refusal, Step};
use guestui_abi::{GuestUi, GuestUiEvent};
use retro_m68k::RunStats;

/// What [`Mac::deliver`] did with an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered { Queued, NotForAGuestWindow }

/// Event codes (Inside Macintosh): mouseDown = 1. A close request is a mouseDown in the go-away box; the real
/// EventRecord (where, modifiers, FindWindow's inGoAway) is built when GetNextEvent lands.
pub const MOUSE_DOWN: u16 = 1;

pub struct RunReport { pub stats: RunStats }

impl<U: GuestUi> Mac<U> {
    pub fn start(&mut self, code: &[u8], entry: u32) -> Result<(), String> {
        if entry as usize >= code.len() { return Err(format!("entry {entry:#x} is outside the {}-byte image", code.len())); }
        self.mem.write_bytes(CODE_BASE, code).map_err(|e| e.to_string())?;
        self.mem.write_u32(STACK_TOP - 4, EXIT_STUB).map_err(|e| e.to_string())?; // return address for the program's final rts
        self.ctx = Some(retro_m68k::start_with(self, CODE_BASE + entry, STACK_TOP - 4, &[]));
        Ok(())
    }

    /// Run at most `budget` instructions; the stats of what ran come back too.
    pub fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let idle = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: true };
        let Some(mut ctx) = self.ctx.take() else { return (Step::Exited, RunStats { blocked: false, halted: true, ..idle }) };
        let stats = retro_m68k::resume(self, &mut ctx, budget);
        let step = if stats.halted { Step::Exited } else if stats.blocked { Step::Blocked } else { Step::BudgetSpent };
        if step != Step::Exited { self.ctx = Some(ctx); }
        (step, stats)
    }

    /// A native event for the guest. Only CloseRequested is bridged so far, queued as (mouseDown, WindowPtr).
    pub fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivered, Refusal> {
        let GuestUiEvent::CloseRequested { win: native, .. } = ev else { return Err(Refusal(format!("deliver: {ev:?} not bridged yet"))) };
        let Some((&win, _)) = self.windows.iter().find(|(_, w)| w.native == native) else { return Ok(Delivered::NotForAGuestWindow) };
        self.pending.push((MOUSE_DOWN, win));
        Ok(Delivered::Queued)
    }

    /// Load and run to the end or `max_insns`.
    pub fn run_program(&mut self, code: &[u8], entry: u32, max_insns: u64) -> Result<RunReport, String> {
        self.start(code, entry)?;
        let mut total = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: false };
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
