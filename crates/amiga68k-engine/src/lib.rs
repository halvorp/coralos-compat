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
pub mod boopsi;
pub mod devices;
pub mod disklib;
pub mod dos;
pub mod dos_assign;
pub mod dos_err;
pub mod dos_files;
pub mod dos_fs;
pub mod dos_io;
pub mod dos_misc;
pub mod dos_path;
pub mod exec;
pub mod execlists;
pub mod exec_wait;
pub mod fs;
pub mod gadtools;
pub mod graphics;
pub mod guest_call;
pub mod fixture;
pub mod intuition;
pub mod libtable;
pub mod loadseg;
pub mod lvo_names;
pub mod lists;
pub mod machine;
pub mod makelib;
pub mod memheader;
pub mod memory;
pub mod mui;
pub mod ports;
pub mod process;
pub mod rdargs;
pub mod run;
pub mod screen;
pub mod semaphores;
pub mod tags;
pub mod timer;
pub mod volume;
pub mod window;
mod menu;

pub use machine::{Amiga, RunReport};
pub use retro_common::Step;
pub use run::Delivered;

pub use retro_common::Refusal;
