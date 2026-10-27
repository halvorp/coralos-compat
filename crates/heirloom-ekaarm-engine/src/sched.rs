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
        // void CActive::Deque() / ~CActive(): out of the queue (a request still outstanding is cancelled first on a device; none can be
        // outstanding here because nothing issues one yet).
        "_ZN7CActive5DequeEv" => { let r = p.sched_remove(this); done(p, r) }
        // protected void CActive::SetActive(): the request has been issued; the scheduler will run `RunL` when it completes.
        "_ZN7CActive9SetActiveEv" => {
            let r = p.rd(this + ACTIVE_I_FLAGS).and_then(|f| p.wr(this + ACTIVE_I_FLAGS, f | FLAG_ACTIVE));
            done(p, r)
        }
        _ => return None,
    })
}
