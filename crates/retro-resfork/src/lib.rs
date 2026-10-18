//! Classic Mac resource forks (no_std + alloc), shared by the 68k and PowerPC Mac engines.
//!
//! * [`fork`]: the Resource Manager file format (Inside Macintosh: More Macintosh Toolbox, "Resource Manager"): a
//!   16-byte header, the resource data, then the resource map (type list, reference lists, name list). [`ResFork`]
//!   validates every offset once and indexes the resources.
//! * [`container`]: where a fork comes from: MacBinary II (CRC-checked), an AppleDouble header file, or a raw fork.
//! * [`writer`]: the inverse (build a fork, wrap it in MacBinary II / AppleDouble), used by tests and tools.
//!
//! Every malformed input is a named [`ForkError`]; nothing here panics on file contents.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod container;
pub mod fork;
pub mod writer;

pub use container::{open, Kind, Opened};
pub use fork::{ForkError, ResEntry, ResFork};

/// A four-character resource type or file type, e.g. `*b"CODE"`.
pub type OsType = [u8; 4];

pub(crate) fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}
pub(crate) fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
