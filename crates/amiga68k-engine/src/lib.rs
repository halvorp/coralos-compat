//! AmigaOS 68k engine (skeleton).
//!
//! Flow: a Hunk executable is loaded into guest RAM; a fake ExecBase sits at the longword
//! at address 4; every library function slot (LVO) below a library base holds
//! `ILLEGAL ($4AFC); RTS ($4E75)`. Musashi's illegal hook lands in [`Amiga`]'s `Host::illegal`,
//! which maps the slot address to a registered handler. The handler reads guest registers
//! and structs through checked memory, calls [`guestui_abi::GuestUi`], sets D0, and the
//! guest then executes the slot's own RTS. Anything not registered, or any bad guest
//! pointer, is a named refusal that returns NULL; nothing is silently accepted.
//!
//! Struct offsets and LVO numbers are written from public NDK-style documentation and
//! need re-verification against a licence-checked source (see NOTICE).
#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::string::String;
use alloc::format;
pub mod devices;
pub mod dos;
pub mod exec;
pub mod gadtools;
pub mod graphics;
pub mod fixture;
pub mod intuition;
pub mod libtable;
pub mod lvo_names;
pub mod machine;
pub mod memory;
pub mod ports;
pub mod process;
pub mod rdargs;
pub mod screen;
pub mod tags;
pub mod window;
mod menu;

pub use machine::{Amiga, RunReport};

/// A handler refused a call (bad guest pointer, unsupported feature, UI refusal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl Refusal {
    pub fn mem(what: &str) -> impl Fn(retro_guest_mem::MemError) -> Refusal + '_ {
        move |e| Refusal(format!("{what}: {e}"))
    }
    pub fn ui(e: guestui_abi::UiRefusal) -> Refusal { Refusal(format!("guestui: {e}")) }
}
