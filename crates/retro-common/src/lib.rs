//! What every engine (amiga68k, mac68k, ...) shares beyond the CPU, RAM and UI crates: the named [`Refusal`] and
//! the [`Step`] a resumable machine's `step` returns. Moved out of amiga68k-engine so the second engine did not copy
//! them, and the [`GuestMachine`] surface a host drives any of them through.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod driver;
pub use driver::{Delivery, GuestMachine, RunStats};
use alloc::format;
use alloc::string::String;

/// A handler refused a call (bad guest pointer, unsupported feature, UI refusal). Never silently accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl Refusal {
    pub fn mem(what: &str) -> impl Fn(retro_guest_mem::MemError) -> Refusal + '_ {
        move |e| Refusal(format!("{what}: {e}"))
    }
    pub fn ui(e: guestui_abi::UiRefusal) -> Refusal { Refusal(format!("guestui: {e}")) }
}

/// How a machine's `step` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step { Exited, Blocked, BudgetSpent, Fault(String) }

/// Big-endian byte helpers every container parser/writer shares (PEF, resource forks, cfrg, hunks). The readers are
/// bounds-checked and overflow-safe: `None` on a short buffer or an offset that wraps, never a panic.
pub mod bytes {
    use alloc::vec::Vec;
    pub fn be16(b: &[u8], at: usize) -> Option<u16> { Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?)) }
    pub fn be32(b: &[u8], at: usize) -> Option<u32> { Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?)) }
    pub fn put_be16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }
    pub fn put_be32(v: &mut Vec<u8>, x: u32) { v.extend_from_slice(&x.to_be_bytes()); }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn readers_are_checked_and_writers_round_trip() {
            let mut v = Vec::new();
            put_be32(&mut v, 0x0102_0304); put_be16(&mut v, 0xA0B0);
            assert_eq!((be32(&v, 0), be16(&v, 4), be16(&v, 1)), (Some(0x0102_0304), Some(0xA0B0), Some(0x0203)));
            // negative controls: short buffer, and an offset that would wrap
            assert_eq!((be32(&v, 3), be16(&v, 5), be32(&v, usize::MAX), be16(&v, usize::MAX - 1)), (None, None, None, None));
        }
    }
}
