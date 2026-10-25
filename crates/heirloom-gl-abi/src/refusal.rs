//! Named refusals. An unimplemented call is REFUSED BY NAME on the ring; it is never skipped, never a silent no-op.
use crate::contract::Entry;
use alloc::format;
use alloc::string::{String, ToString};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No GPU acceleration: `aglChoosePixelFormat` returns no format (ONE-GPU-PATH's explicit degrade).
    NoGpuAccel,
    /// The entry exists in the contract but the worker does not execute it yet.
    Unsupported { name: String },
    /// A call frame named an entry index the table does not have.
    UnknownEntry { index: u16 },
    /// Nothing implemented/attested: no `GL_VERSION` can be claimed.
    NoClaimableVersion,
}

/// Admit a call by table index: only an implemented entry passes; anything else is a [`Refusal`] naming it.
pub fn admit_call(entries: &[Entry], index: u16) -> Result<&Entry, Refusal> {
    let e = entries.get(usize::from(index)).ok_or(Refusal::UnknownEntry { index })?;
    if e.implemented {
        Ok(e)
    } else {
        Err(Refusal::Unsupported { name: e.name.to_string() })
    }
}

/// The ring marker for a refusal, e.g. `[HEIRLOOM-GL REFUSED reason=no-gpu-accel]` or `[HEIRLOOM-GL REFUSED call=glFoo]`.
pub fn refusal_marker(r: &Refusal) -> String {
    match r {
        Refusal::NoGpuAccel => "[HEIRLOOM-GL REFUSED reason=no-gpu-accel]".to_string(),
        Refusal::Unsupported { name } => format!("[HEIRLOOM-GL REFUSED call={name}]"),
        Refusal::UnknownEntry { index } => format!("[HEIRLOOM-GL REFUSED entry-index={index}]"),
        Refusal::NoClaimableVersion => "[HEIRLOOM-GL REFUSED reason=no-claimable-version]".to_string(),
    }
}
