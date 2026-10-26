//! Heirloom Symbian EKA2 (ARM) engine.
#![no_std]
extern crate alloc;

pub mod mem;
pub mod layout;
pub mod loader;
pub mod heap;
pub mod stubs;
pub mod natives;
pub mod run;

pub use loader::{Process, LoadError};
pub use run::Outcome;