//! What every engine (amiga68k, mac68k, ...) shares beyond the CPU, RAM and UI crates: the named [`Refusal`] and
//! the [`Step`] a resumable machine's `step` returns. Moved out of amiga68k-engine so the second engine did not copy
//! them.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
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
