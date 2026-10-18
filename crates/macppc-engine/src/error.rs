//! Why a fragment could not be instantiated. Every variant names what was wrong.
use alloc::string::String;
use core::fmt;
use retro_pef::PefError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfmError {
    Pef(PefError),
    /// The guest heap could not hold a section, the stack or a glue chunk.
    NoMemory { what: &'static str, bytes: u32 },
    /// A section asks for more alignment than the loader provides (4 KiB).
    AlignmentTooLarge { section: usize, exponent: u8 },
    /// A guest address was outside RAM while placing or relocating.
    Memory(String),
    /// Strict mode: a strong (non-weak) import has no binding.
    UnresolvedImport { library: String, symbol: String, why: String },
    /// The fragment has no main entry point (it is a library, not an application).
    NoMain,
}

impl From<PefError> for CfmError { fn from(e: PefError) -> Self { CfmError::Pef(e) } }

impl fmt::Display for CfmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CfmError::Pef(e) => write!(f, "{e}"),
            CfmError::NoMemory { what, bytes } => write!(f, "no guest memory for {what} ({bytes} bytes)"),
            CfmError::AlignmentTooLarge { section, exponent } => write!(f, "PEF section {section} asks for 2^{exponent}-byte alignment, the loader provides at most 4 KiB"),
            CfmError::Memory(w) => write!(f, "guest memory: {w}"),
            CfmError::UnresolvedImport { library, symbol, why } => write!(f, "unresolved strong import {library}/{symbol}: {why}"),
            CfmError::NoMain => write!(f, "the fragment has no main entry point"),
        }
    }
}
impl core::error::Error for CfmError {}
