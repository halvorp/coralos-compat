//! Preferred Executable Format (PEF), the Code Fragment Manager's container for classic Mac OS PowerPC code
//! (no_std + alloc). Facts are from Apple's "Mac OS Runtime Architectures" (the PEF chapter) and cross-checked against
//! Apple's CFMLateImport sample source and the MIT-licensed Executor loader (`src/cfm.cpp`, whose loader
//! implements RelocSmRepeat (rewinding by 16-bit chunks, as here) but asserts on RelocLgRepeat/RelocLgSetOrBySection,
//! which are implemented here from the spec). No code is
//! copied from either.
//!
//! * [`header`]: the 40-byte container header, the 28-byte section headers, section names and section images.
//! * [`pidata`]: the pattern-initialised data opcodes (zero / block / repeat / repeat-block / repeat-zero).
//! * [`loader`]: the loader section: imported libraries and symbols, relocation headers, the export hash table and the
//!   main / init / term entry points.
//! * [`reloc`]: every relocation opcode decoded into [`reloc::RelocOp`] (repeats expanded) and an interpreter that
//!   reports each word to add to; the caller owns memory.
//! * [`hash`]: the export hash function.
//! * [`writer`]: the inverse, [`writer::PefBuilder`], for tests and tools.
//!
//! Every malformed input is a named [`PefError`]; nothing here panics on file contents.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod hash;
pub mod header;
pub mod loader;
pub mod pidata;
pub mod reloc;
pub mod writer;

pub use header::{Header, Pef, SectionHeader, SectionKind};
pub use loader::{EntryPoint, ExportedSymbol, ImportedLibrary, ImportedSymbol, Loader, SymbolClass};
use core::fmt;

/// Why a PEF container (or something in it) was refused. The text names the part of the format that is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PefError {
    /// The file or a table runs past its end.
    Truncated(&'static str),
    /// The first eight bytes are not `Joy!peff`.
    BadMagic,
    /// A container for an architecture other than `pwpc` (68k CFM fragments are `m68k`).
    UnsupportedArchitecture([u8; 4]),
    /// A format version other than 1.
    UnsupportedVersion(u32),
    /// A section header, its extent or its name is wrong.
    BadSection { index: usize, why: &'static str },
    /// The loader section is missing, duplicated or inconsistent.
    BadLoader(&'static str),
    /// The pattern-initialised data stream is malformed.
    BadPidata(&'static str),
    /// A relocation opcode the format does not define.
    UnknownRelocOpcode { opcode: u16, index: u32 },
    /// A relocation stream that is structurally wrong (truncated two-word opcode, repeat of nothing ...).
    BadReloc(&'static str),
    /// A repeat instruction would expand past the safety bound.
    RelocTooLarge,
    /// A symbol class byte the format does not define.
    BadSymbolClass(u8),
}

impl fmt::Display for PefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PefError::Truncated(w) => write!(f, "PEF truncated: {w}"),
            PefError::BadMagic => write!(f, "not a PEF container (no 'Joy!peff' magic)"),
            PefError::UnsupportedArchitecture(a) => write!(f, "PEF architecture '{}' is not 'pwpc'", a.iter().map(|&c| char::from(c)).collect::<alloc::string::String>()),
            PefError::UnsupportedVersion(v) => write!(f, "PEF format version {v} is not 1"),
            PefError::BadSection { index, why } => write!(f, "PEF section {index}: {why}"),
            PefError::BadLoader(w) => write!(f, "PEF loader section: {w}"),
            PefError::BadPidata(w) => write!(f, "PEF pattern data: {w}"),
            PefError::UnknownRelocOpcode { opcode, index } => write!(f, "PEF relocation opcode {opcode:#06x} at instruction {index} is not defined"),
            PefError::BadReloc(w) => write!(f, "PEF relocation: {w}"),
            PefError::RelocTooLarge => write!(f, "PEF relocation: a repeat expands past the safety bound"),
            PefError::BadSymbolClass(c) => write!(f, "PEF symbol class {c} is not defined"),
        }
    }
}
impl core::error::Error for PefError {}

pub(crate) use heirloom_common::bytes::{be16, be32};
