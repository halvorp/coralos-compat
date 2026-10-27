//! Heirloom Symbian EKA2 (ARM) engine.
#![no_std]
extern crate alloc;
#[cfg(feature = "host")]
extern crate std;

pub mod mem;
pub mod layout;
pub mod loader;
pub mod heap;
pub mod stubs;
pub mod natives;
pub mod run;
pub mod callback;
pub mod appstart;
pub mod drive;
pub mod ui;
pub mod cone;
#[cfg(feature = "host")]
pub mod hostdir;

pub use loader::{Process, LoadError};
pub use run::Outcome;