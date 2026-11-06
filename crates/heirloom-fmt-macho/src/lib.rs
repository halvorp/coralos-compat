//! Bounds-checked Mach-O x86_64 parsing and dyld opcode decoding.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod bind;
pub mod exports;
pub mod header;
pub mod writer;

pub use bind::{Bind, Rebase};
pub use exports::Export;
pub use header::{DyldInfo, Dylib, Entry, MachO, MachoError, Section, Segment};
pub use writer::{BuiltImage, ImageBuilder, Patch};