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
pub mod appui;
pub mod objects;
pub mod control;
pub mod timer;
pub mod tz;
pub mod mixins;
pub mod sched;
pub mod dictstore;
pub mod cleanup;
pub mod fs;
pub mod avkon;
pub mod screen;
pub mod enginestub;
pub mod trap;
pub mod desc;
pub mod viewappui;
pub mod resource;
pub mod view;
pub mod remcon;
pub mod machine;
#[cfg(feature = "host")]
pub mod hostdir;

pub use loader::{Process, LoadError};
pub use run::Outcome;
pub use machine::EkaArm;
