//! APPLE_fence bookkeeping: a fence is a real fence. It completes only when the worker has RETIRED the batch it was
//! set in; it is never complete because time passed or because the engine asked.
use alloc::collections::BTreeMap;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FenceError {
    Unknown,
    /// Fence ids are exhausted (named refusal).
    Exhausted,
}

/// Engine-side view of fences; `batch_seq` numbers the batches the engine submits.
#[derive(Default)]
pub struct FenceBook {
    next: u32,
    retired_through: u64,
    set_in: BTreeMap<u32, u64>,
}

impl FenceBook {
    /// `glSetFenceAPPLE`: the fence is pending until `batch_seq` is retired.
    pub fn set(&mut self, batch_seq: u64) -> Result<u32, FenceError> {
        let id = self.next.checked_add(1).ok_or(FenceError::Exhausted)?;
        self.next = id;
        self.set_in.insert(id, batch_seq);
        Ok(id)
    }

    /// The worker reports every batch up to and including `seq` retired. Never moves backwards.
    pub fn retire_through(&mut self, seq: u64) {
        self.retired_through = self.retired_through.max(seq);
    }

    /// `glTestFenceAPPLE`.
    pub fn is_complete(&self, id: u32) -> Result<bool, FenceError> {
        let s = self.set_in.get(&id).ok_or(FenceError::Unknown)?;
        Ok(*s <= self.retired_through)
    }

    pub fn delete(&mut self, id: u32) -> Result<(), FenceError> {
        self.set_in.remove(&id).map(|_| ()).ok_or(FenceError::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fence_is_incomplete_until_the_worker_retires_its_batch() {
        let mut b = FenceBook::default();
        let f = b.set(7).unwrap();
        assert_eq!(b.is_complete(f), Ok(false));
        b.retire_through(6);
        assert_eq!(b.is_complete(f), Ok(false));
        b.retire_through(7);
        assert_eq!(b.is_complete(f), Ok(true));
        b.retire_through(3);
        assert_eq!(b.is_complete(f), Ok(true), "retirement never moves backwards");
        assert_eq!(b.delete(f), Ok(()));
        assert_eq!(b.is_complete(f), Err(FenceError::Unknown));
    }
}
