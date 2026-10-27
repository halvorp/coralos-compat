//! Asynchronous request accounting: the thread's request semaphore and the set of `TRequestStatus` words with a request outstanding
//! (e32std.h `RThread::RequestComplete` / `User::WaitForAnyRequest`). A producer (the window server's event notification, later timers
//! and sockets) takes a status with [`Process::request_issue`] (it holds `KRequestPending`, e32const.h:961) and ends it with
//! [`Process::request_complete`], which writes the result and signals the thread exactly once; the active scheduler consumes one signal
//! per object it runs (`schedrun.rs`). Completing a status that is not pending, or issuing on one that already is, is refused by name:
//! it would signal the thread for nothing (a stray signal, E32USER-CBase 46 on a device) or lose a completion.
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use crate::loader::Process;

/// `KRequestPending` (e32const.h:961 `-KMaxTInt`).
pub const KREQUEST_PENDING: u32 = 0x8000_0001;

#[derive(Debug, Default)]
pub struct RequestState {
    /// Guest addresses of the status words with a request outstanding.
    pending: BTreeSet<u32>,
    /// Completions not yet consumed by the scheduler (the thread's request semaphore).
    pub signals: u32,
}

impl RequestState {
    pub fn is_pending(&self, status: u32) -> bool {
        self.pending.contains(&status)
    }
}

impl Process {
    /// A request was issued on the status at `status`: it holds `KRequestPending` until it completes.
    pub(crate) fn request_issue(&mut self, status: u32) -> Result<(), String> {
        if !self.nat.requests.pending.insert(status) {
            return Err(format!("a request is already outstanding on the status at {status:#x} (E32USER-CBase 46 on a device)"));
        }
        self.wr(status, KREQUEST_PENDING)
    }

    /// The producer completed the request on `status` with `code`: written to the guest's status, and the thread signalled once.
    pub(crate) fn request_complete(&mut self, status: u32, code: i32) -> Result<(), String> {
        if !self.nat.requests.pending.remove(&status) {
            return Err(format!("no request is outstanding on the status at {status:#x}: completing it would signal the thread for nothing"));
        }
        self.wr(status, code as u32)?;
        self.nat.requests.signals += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_pending_until_completed_and_signals_once() {
        let mut s = RequestState::default();
        assert!(!s.is_pending(0x1000));
        assert!(s.pending.insert(0x1000));
        assert!(s.is_pending(0x1000));
        assert!(!s.pending.insert(0x1000), "a second issue on the same status is detected");
    }
}
