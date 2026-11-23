//! Active objects (`CActive`) and the active scheduler's queue (`CActiveScheduler`, e32base.h).
//!
//! `CActive` layout (e32base.h:1700-1711): vptr 0 (CBase), `TRequestStatus iStatus` 4 (`iStatus` 4, `iFlags` 8: `EActive` 1,
//! `ERequestPending` 2, EKA2L1 utils/reqsts.h, facts), `TPriQueLink iLink` 12 (`iNext` 12, `iPrev` 16, `iPriority` 20), `iSpare` 24;
//! 28 bytes, which `CCoeEnv`'s `iAppUi` at 36 (after the 8-byte `MObjectProvider`) confirms. The guest reads three of them inline:
//! `IsActive()` (`iFlags & EActive`), `IsAdded()` (`iLink.iNext != NULL`) and `Priority()` (`iLink.iPriority`, e32base.inl).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const ACTIVE_I_STATUS: u32 = 4;
pub const ACTIVE_I_FLAGS: u32 = 8;
pub const ACTIVE_I_NEXT: u32 = 12;
pub const ACTIVE_I_PREV: u32 = 16;
pub const ACTIVE_I_PRIORITY: u32 = 20;
pub const ACTIVE_SIZE: u32 = 28;
pub const FLAG_ACTIVE: u32 = 1;
/// `CIdle` callback fields stored after `CActive` (28 bytes): `iCallBack.iFunction` (28), `iCallBack.iPtr` (32).
const IDLE_I_FUNC: u32 = 28;
const IDLE_I_PTR: u32 = 32;
const IDLE_SIZE: u32 = 40;
pub const IDLE_RUN_L: &str = "_ZN5CIdle4RunLEv";
const IDLE_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN5CIdleD1Ev"),
    (1, "_ZN5CIdleD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZN5CIdle8DoCancelEv"),
    (4, IDLE_RUN_L),
];
pub const PERIODIC_RUN_L: &str = "_ZN9CPeriodic4RunLEv";
const PERIODIC_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN9CPeriodicD1Ev"),
    (1, "_ZN9CPeriodicD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZN9CPeriodic8DoCancelEv"),
    (4, PERIODIC_RUN_L),
];

/// Host state for one active `CPeriodic` timer (`sched.rs`).
#[derive(Debug, Clone, Copy)]
pub struct PeriodicTimer {
    pub interval_us: u64,
    pub deadline_us: u64,
    pub armed: bool,
}
/// The queue's sentinel node: `iNext`, `iPrev`, `iPriority` (a `TPriQueLink` without the CBase part), a circular list head.
const HEAD_BYTES: u32 = 12;

impl Process {
    pub(crate) fn rd(&self, a: u32) -> Result<u32, String> {
        self.mem.vm.read_u32(a).map_err(|_| format!("{a:#x} is unreadable"))
    }
    /// The guest word at `addr` (host diagnostics and tests: a read-only look at guest memory), or None when it is unreadable.
    pub fn guest_word(&self, addr: u32) -> Option<u32> {
        self.mem.vm.read_u32(addr).ok()
    }
    pub(crate) fn wr(&mut self, a: u32, v: u32) -> Result<(), String> {
        self.mem.vm.write_u32(a, v).map_err(|_| format!("{a:#x} is unwritable"))
    }

    /// The guest address of the scheduler queue's sentinel, allocated on first use (an empty circular list: both links point to it).
    fn sched_head(&mut self) -> Result<u32, String> {
        if let Some(h) = self.nat.sched_head {
            return Ok(h);
        }
        let h = self.alloc_zeroed(HEAD_BYTES).ok_or_else(|| String::from("no guest heap for the scheduler queue"))?;
        self.wr(h, h)?;
        self.wr(h + 4, h)?;
        self.nat.sched_head = Some(h);
        Ok(h)
    }

    /// `CActiveScheduler::Add(CActive*)`: insert behind every item of equal or higher priority (`TPriQue::Add`). The list threads
    /// through the `iLink` members (`TDblQueLinkBase::iNext`/`iPrev` point at other links, not at objects).
    pub(crate) fn sched_add(&mut self, ao: u32) -> Result<(), String> {
        if ao == 0 {
            return Err(String::from("Add(NULL): E32USER-CBase 49 on a device"));
        }
        let link = ao + ACTIVE_I_NEXT;
        if self.rd(link)? != 0 {
            return Err(format!("the active object {ao:#x} is already added (E32USER-CBase 48 on a device)"));
        }
        let head = self.sched_head()?;
        let prio = self.rd(link + 8)? as i32;
        let mut at = self.rd(head)?; // first link
        while at != head && self.rd(at + 8)? as i32 >= prio {
            at = self.rd(at)?;
        }
        // insert `link` before `at`
        let prev = self.rd(at + 4)?;
        self.wr(link, at)?;
        self.wr(link + 4, prev)?;
        self.wr(prev, link)?;
        self.wr(at + 4, link)
    }

    /// Take `ao` out of the queue (`CActive::Deque`): unlinked and its links cleared, so `IsAdded()` is false again.
    fn sched_remove(&mut self, ao: u32) -> Result<(), String> {
        let link = ao + ACTIVE_I_NEXT;
        let (next, prev) = (self.rd(link)?, self.rd(link + 4)?);
        if next == 0 {
            return Ok(());
        }
        self.wr(prev, next)?;
        self.wr(next + 4, prev)?;
        self.wr(link, 0)?;
        self.wr(link + 4, 0)
    }

    fn sched_cancel(&mut self, ao: u32) -> Result<(), String> {
        if let Some(t) = self.nat.periodic_timers.get_mut(&ao) {
            t.armed = false;
        }
        let flags = self.rd(ao + ACTIVE_I_FLAGS)?;
        if flags & FLAG_ACTIVE != 0 {
            let st = self.rd(ao + ACTIVE_I_STATUS)?;
            self.wr(ao + ACTIVE_I_FLAGS, flags & !FLAG_ACTIVE)?;
            if st != crate::request::KREQUEST_PENDING && self.nat.requests.signals > 0 {
                self.nat.requests.signals -= 1;
            }
            self.wr(ao + ACTIVE_I_STATUS, (-3i32) as u32)?; // KErrCancel
        }
        Ok(())
    }

    fn arm_idle(&mut self, ao: u32) -> Result<(), String> {
        let flags = self.rd(ao + ACTIVE_I_FLAGS)?;
        if flags & FLAG_ACTIVE == 0 {
            self.wr(ao + ACTIVE_I_FLAGS, flags | FLAG_ACTIVE)?;
            self.wr(ao + ACTIVE_I_STATUS, 0)?; // KErrNone
            self.nat.requests.signals += 1;
        }
        Ok(())
    }

    fn arm_periodic(&mut self, ao: u32, delay_us: u64, interval_us: u64) -> Result<(), String> {
        let flags = self.rd(ao + ACTIVE_I_FLAGS)?;
        self.wr(ao + ACTIVE_I_FLAGS, flags | FLAG_ACTIVE)?;
        self.wr(ao + ACTIVE_I_STATUS, crate::request::KREQUEST_PENDING)?;
        let deadline_us = self.nat.clock_us.saturating_add(delay_us);
        self.nat.periodic_timers.insert(ao, PeriodicTimer { interval_us, deadline_us, armed: true });
        self.complete_due_timers()
    }

    /// Complete any armed `CPeriodic` timers whose deadline has been reached by `self.nat.clock_us`.
    pub(crate) fn complete_due_timers(&mut self) -> Result<(), String> {
        let now = self.nat.clock_us;
        let due: alloc::vec::Vec<u32> = self.nat.periodic_timers.iter()
            .filter(|(_, t)| t.armed && t.deadline_us <= now)
            .map(|(&ao, _)| ao)
            .collect();
        for ao in due {
            let flags = self.rd(ao + ACTIVE_I_FLAGS)?;
            let status = self.rd(ao + ACTIVE_I_STATUS)?;
            if flags & FLAG_ACTIVE != 0 && status == crate::request::KREQUEST_PENDING {
                self.wr(ao + ACTIVE_I_STATUS, 0)?; // KErrNone
                self.nat.requests.signals += 1;
            }
        }
        Ok(())
    }

    /// Advance the process virtual clock to `virtual_us` and complete any due timers.
    pub fn advance_clock_to(&mut self, virtual_us: u64) -> Result<(), String> {
        if virtual_us > self.nat.clock_us {
            self.nat.clock_us = virtual_us;
        }
        self.complete_due_timers()
    }

    /// Earliest virtual-clock deadline (in microseconds) of any armed, pending `CPeriodic` timer.
    pub fn next_timer_deadline(&self) -> Option<u64> {
        self.nat.periodic_timers.iter()
            .filter(|(&ao, t)| {
                t.armed
                    && self.rd(ao + ACTIVE_I_FLAGS).unwrap_or(0) & FLAG_ACTIVE != 0
                    && self.rd(ao + ACTIVE_I_STATUS).unwrap_or(0) == crate::request::KREQUEST_PENDING
            })
            .map(|(_, t)| t.deadline_us)
            .min()
    }
}

/// Called when a `CIdle` `TCallBack` returns `r0`: nonzero re-arms the idle active object.
pub(crate) fn idle_returned(p: &mut Process, ao: u32, r0: u32) -> Result<(), crate::run::Outcome> {
    if r0 != 0 {
        p.arm_idle(ao).map_err(crate::run::Outcome::Fault)?;
    }
    Ok(())
}

/// Called when a `CPeriodic` `TCallBack` returns: Symbian `CPeriodic::RunL` ignores the callback return value
/// and re-arms for `iInterval` unless the timer was cancelled during the callback.
pub(crate) fn periodic_returned(p: &mut Process, ao: u32) -> Result<(), crate::run::Outcome> {
    let rearm = p.nat.periodic_timers.get(&ao).copied().filter(|t| t.armed);
    if let Some(t) = rearm {
        p.arm_periodic(ao, t.interval_us, t.interval_us).map_err(crate::run::Outcome::Fault)?;
    }
    Ok(())
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"));
    let done = |p: &mut Process, r: Result<(), String>| match r {
        Ok(()) => { p.cpu.regs[0] = this; NativeOutcome::Return }
        Err(m) => refuse(m),
    };
    Some(match name {
        // CActive::CActive(TInt aPriority) (e32base.h:1630+): status and links cleared, the priority recorded.
        "_ZN7CActiveC1Ei" | "_ZN7CActiveC2Ei" => {
            let r = (ACTIVE_I_STATUS..ACTIVE_SIZE).step_by(4).try_for_each(|o| p.wr(this + o, 0)).and_then(|_| p.wr(this + ACTIVE_I_PRIORITY, r1));
            done(p, r)
        }
        // static void CActiveScheduler::Add(CActive*)
        "_ZN16CActiveScheduler3AddEP7CActive" => { let r = p.sched_add(this); done(p, r) }
        // void CActive::Deque() / ~CActive(): out of the queue.
        "_ZN7CActive5DequeEv" => { let r = p.sched_remove(this); done(p, r) }
        "_ZN7CActive6CancelEv" => { let r = p.sched_cancel(this); done(p, r) }
        "_ZN7CActiveD1Ev" | "_ZN7CActiveD2Ev" | "_ZN7CActiveD0Ev" => {
            let _ = p.sched_cancel(this);
            let r = p.sched_remove(this);
            done(p, r)
        }
        // protected void CActive::SetActive(): the request has been issued; the scheduler will run `RunL` when it completes.
        "_ZN7CActive9SetActiveEv" => {
            let r = p.rd(this + ACTIVE_I_FLAGS).and_then(|f| p.wr(this + ACTIVE_I_FLAGS, f | FLAG_ACTIVE));
            done(p, r)
        }
        // static CIdle* CIdle::NewL(TInt aPriority) / CIdle::New(TInt aPriority)
        "_ZN5CIdle4NewLEi" | "_ZN5CIdle3NewEi" => {
            let prio = this;
            let r = (|| -> Result<u32, String> {
                let ao = p.alloc_zeroed(IDLE_SIZE).ok_or_else(|| String::from("no heap for CIdle"))?;
                let vptr = p.build_vtable("CIdle", &crate::enginestub::slots_with_gaps(8, IDLE_VTABLE))?;
                p.wr(ao, vptr)?;
                p.wr(ao + ACTIVE_I_PRIORITY, prio)?;
                p.sched_add(ao)?;
                Ok(ao)
            })();
            match r {
                Ok(ao) => { p.cpu.regs[0] = ao; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // static CPeriodic* CPeriodic::NewL(TInt aPriority) / CPeriodic::New(TInt aPriority)
        "_ZN9CPeriodic4NewLEi" | "_ZN9CPeriodic3NewEi" => {
            let prio = this;
            let r = (|| -> Result<u32, String> {
                let ao = p.alloc_zeroed(IDLE_SIZE).ok_or_else(|| String::from("no heap for CPeriodic"))?;
                let vptr = p.build_vtable("CPeriodic", &crate::enginestub::slots_with_gaps(8, PERIODIC_VTABLE))?;
                p.wr(ao, vptr)?;
                p.wr(ao + ACTIVE_I_PRIORITY, prio)?;
                p.sched_add(ao)?;
                Ok(ao)
            })();
            match r {
                Ok(ao) => { p.cpu.regs[0] = ao; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // void CPeriodic::Start(TTimeIntervalMicroSeconds32 aDelay, TTimeIntervalMicroSeconds32 anInterval, TCallBack aCallBack)
        "_ZN9CPeriodic5StartE27TTimeIntervalMicroSeconds32S0_9TCallBack" => {
            let delay_us = (p.cpu.regs[1] as i32).max(0) as u64;
            let interval_us = (p.cpu.regs[2] as i32).max(1) as u64;
            let func = p.cpu.regs[3];
            let ptr = p.rd(p.cpu.regs[13]).unwrap_or(0);
            let r = p.sched_cancel(this)
                .and_then(|_| p.wr(this + IDLE_I_FUNC, func))
                .and_then(|_| p.wr(this + IDLE_I_PTR, ptr))
                .and_then(|_| p.arm_periodic(this, delay_us, interval_us));
            done(p, r)
        }
        // void CIdle::Start(TCallBack aCallBack): AAPCS passes 2-word TCallBack by value in r1 (iFunction), r2 (iPtr).
        "_ZN5CIdle5StartE9TCallBack" => {
            let (func, ptr) = (p.cpu.regs[1], p.cpu.regs[2]);
            let r = p.wr(this + IDLE_I_FUNC, func)
                .and_then(|_| p.wr(this + IDLE_I_PTR, ptr))
                .and_then(|_| p.arm_idle(this));
            done(p, r)
        }
        // CIdle::RunL(): invoke iCallBack.iFunction(iCallBack.iPtr); re-arm if return != 0.
        IDLE_RUN_L => {
            let r = (|| -> Result<(), String> {
                let func = p.rd(this + IDLE_I_FUNC)?;
                let ptr = p.rd(this + IDLE_I_PTR)?;
                if func != 0 {
                    p.call_guest(func, &[ptr], crate::callback::Continuation::IdleCallback { ao: this })?;
                }
                Ok(())
            })();
            match r { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) }
        }
        // CPeriodic::RunL(): invoke iCallBack.iFunction(iCallBack.iPtr); always re-arm for iInterval unless cancelled.
        PERIODIC_RUN_L => {
            let r = (|| -> Result<(), String> {
                let func = p.rd(this + IDLE_I_FUNC)?;
                let ptr = p.rd(this + IDLE_I_PTR)?;
                if func != 0 {
                    p.call_guest(func, &[ptr], crate::callback::Continuation::PeriodicCallback { ao: this })?;
                } else {
                    let _ = periodic_returned(p, this);
                }
                Ok(())
            })();
            match r { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) }
        }
        "_ZN5CIdle8DoCancelEv" | "_ZN9CPeriodic8DoCancelEv" => NativeOutcome::Return,
        "_ZN5CIdleD1Ev" | "_ZN5CIdleD0Ev" | "_ZN9CPeriodicD1Ev" | "_ZN9CPeriodicD0Ev" => {
            let _ = p.sched_cancel(this);
            let _ = p.sched_remove(this);
            p.nat.periodic_timers.remove(&this);
            if name.ends_with("D0Ev") {
                if let Some(heap) = p.heap.as_mut() { let _ = heap.free(this); }
            }
            NativeOutcome::Return
        }
        _ => return None,
    })
}
