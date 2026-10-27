//! `CActiveScheduler::Start()` as a suspension, not a Rust loop (consult-2026-10-26-symbian-runapplication.md, slice 4). `Start`
//! parks the caller: a frame marked [`Continuation::SchedWait`] holds the CPU it returns to, and while that frame is the top of the
//! callback stack the run loop calls [`pump`] instead of executing instructions. The pump is the scheduler's loop body
//! (`CActiveScheduler::Start` -> `WaitForAnyRequest` -> `RunIfReady`, e32base.h:2866-2871): with no completed request it returns
//! [`Pump::Blocked`] (the step ends `Blocked`, nothing polls); with one it consumes ONE signal of the thread's request semaphore
//! (`request.rs`), picks the first ready object in priority order (the queue `sched.rs` keeps), clears its active flag and calls its
//! `RunL` through the callback machine. `Stop` marks the innermost activation; the pump ends it when the current `RunL` has returned,
//! and the guest resumes after the `Start` that parked it. A nested `Start` (a dialog's wait loop) is another frame on the same stack.
use alloc::format;
use alloc::string::String;
use crate::appstart::{anchor, SlotSpec};
use crate::callback::{Continuation, Frame, MAX_CALLBACK_DEPTH};
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::request::KREQUEST_PENDING;
use crate::run::Outcome;
use crate::sched::{ACTIVE_I_FLAGS, ACTIVE_I_NEXT, ACTIVE_I_STATUS, FLAG_ACTIVE};

/// `CActive::RunL` (e32base.h:1631-1650): CBase's ~CBase 0,1 and Extension_ 2 (e32base.h:48, :56), then `DoCancel` 3 and `RunL` 4 (both
/// pure, :1640+), `RunError` 5. The window-server event object (`CCoeEnv`) is engine-built and holds our stub; every other
/// active object is the guest's own and holds its code.
pub const SLOT_ACTIVE_RUN_L: u32 = 4;
const INHERITED_RUN_L: &[&str] = &[crate::wsevent::ENV_RUN_L];

pub(crate) enum Pump {
    /// Something ran or ended: look again.
    Continue,
    /// No completed request: the thread waits for one.
    Blocked,
}

impl Process {
    /// `CActiveScheduler::Start()`: park the caller (its CPU is the one in `self.cpu` now) and let the pump run the scheduler.
    pub(crate) fn sched_start(&mut self) -> Result<(), String> {
        if self.frames.len() >= MAX_CALLBACK_DEPTH {
            return Err(format!("scheduler activation nested deeper than {MAX_CALLBACK_DEPTH}"));
        }
        self.frames.push(Frame { saved: self.cpu.clone(), cont: Continuation::SchedWait { stop: false } });
        self.log.push(String::from("[sched] CActiveScheduler::Start: the thread is in its scheduler"));
        Ok(())
    }

    /// `CActiveScheduler::Stop()`: the innermost activation ends once the object that called it has finished running.
    fn sched_stop(&mut self) -> Result<(), String> {
        match self.frames.iter_mut().rev().find(|f| matches!(f.cont, Continuation::SchedWait { .. })) {
            Some(f) => { f.cont = Continuation::SchedWait { stop: true }; Ok(()) }
            None => Err(String::from("Stop() with no scheduler running (E32USER-CBase 44 on a device)")),
        }
    }

    /// The first object in the scheduler's queue (priority order) that is active and whose request has completed.
    fn sched_ready(&self) -> Result<Option<u32>, String> {
        let Some(head) = self.nat.sched_head else { return Ok(None) };
        let mut at = self.rd(head)?;
        while at != head {
            let ao = at.wrapping_sub(ACTIVE_I_NEXT);
            if self.rd(ao + ACTIVE_I_FLAGS)? & FLAG_ACTIVE != 0 && self.rd(ao + ACTIVE_I_STATUS)? != KREQUEST_PENDING {
                return Ok(Some(ao));
            }
            at = self.rd(at)?;
        }
        Ok(None)
    }
}

/// One turn of the scheduler loop; called by the run loop while the top frame is a `SchedWait`.
pub(crate) fn pump(p: &mut Process) -> Result<Pump, Outcome> {
    if matches!(p.frames.last(), Some(Frame { cont: Continuation::SchedWait { stop: true }, .. })) {
        if let Some(f) = p.frames.pop() {
            p.cpu = f.saved;
            p.cpu.regs[0] = 0;
            p.log.push(String::from("[sched] activation stopped"));
        }
        return Ok(Pump::Continue);
    }
    if p.nat.requests.signals == 0 {
        return Ok(Pump::Blocked);
    }
    let Some(ao) = p.sched_ready().map_err(Outcome::Fault)? else {
        return Err(Outcome::Fault(format!("E32USER-CBase 46 (stray signal): {} request(s) completed but no active object is ready", p.nat.requests.signals)));
    };
    p.nat.requests.signals -= 1;
    let flags = p.rd(ao + ACTIVE_I_FLAGS).map_err(Outcome::Fault)?;
    p.wr(ao + ACTIVE_I_FLAGS, flags & !FLAG_ACTIVE).map_err(Outcome::Fault)?;
    let spec = SlotSpec { class: "CActive", method: "RunL", slot: SLOT_ACTIVE_RUN_L, inherited: INHERITED_RUN_L };
    let target = anchor(p, ao, &spec).map_err(Outcome::Fault)?;
    p.call_guest(target, &[ao], Continuation::SchedRun { ao }).map_err(Outcome::Fault)?;
    Ok(Pump::Continue)
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let r = match name {
        // static void CActiveScheduler::Start() / Stop() (e32base.h:2867-2868)
        "_ZN16CActiveScheduler5StartEv" => p.sched_start(),
        "_ZN16CActiveScheduler4StopEv" => p.sched_stop(),
        _ => return None,
    };
    Some(match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) })
}
