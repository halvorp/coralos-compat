//! heirloom-gl-abi: the guest OpenGL contract as DATA, and the command-ring ABI between a guest engine (big-endian guest
//! memory) and `heirloom-gl.engine` (Mesa's own desktop OpenGL, compatibility profile). Ruling: `docs/heirloom/22-GUEST-GL.md`,
//! doctrine `CORAL-GL-IS-ONE-MESA-WORKER`.
//!
//! Guarantees, each pinned by a test in `tests/negative_controls.rs` that `mutants.sh` proves goes RED when the
//! guarantee is broken:
//! * `GL_VERSION` / `GL_EXTENSIONS` are DERIVED from the per-entry `implemented` flags ([`caps`]); no string is written
//!   by hand, and no version/extension is ever claimed that some listed entry point does not back.
//! * An unimplemented entry point is REFUSED BY NAME ([`refusal`]); it is never skipped.
//! * A draw carries its own vertex/index bytes, snapshotted at draw time and already converted to the wire byte order
//!   ([`snapshot`]); a later write to guest memory cannot change a submitted draw.
//! * Every length and offset is computed with checked arithmetic; overflow is an error, never a wrap ([`snapshot`], [`wire`]).
//! * A context/drawable token carries an incarnation; a token from an earlier life of its slot is refused ([`token`]).
//! * Every frame starts with its ABI version and is little-endian on the wire ([`wire`], [`command`]).
//! * Nothing here has a fixed capacity: tables grow, and the decoder sizes allocations from bytes actually present.
//! * The regular call families (`glVertex*`, `glColor*`, `glLight*`, the Get* parameter queries, ...) are ONE mechanism ([`family`]): the
//!   table [`family_table`] is generated from the Khronos registry by `tools/gen-gl-sigs.py` (which also writes the worker's typed Mesa
//!   entry table), and the hand-written bulk-data calls are [`bulk_table`]. What the stack under the worker cannot do is refused by name with
//!   the value or the situation named ([`gaps`]), the engine's own argument checks are [`checks`], and [`coverage`] says which entries
//!   the engine's tests (rather than the worker's GPU tests) grade.
//!
//! Pure `no_std + alloc`: no host, GPU, UI or Mesa reach, so a guest engine may depend on it (seam check).
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod caps;
pub mod checks;
pub mod command;
pub mod coverage;
pub mod contract;
pub mod family;
pub mod bulk_table;
pub mod family_table;
pub mod fence;
pub mod gaps;
pub mod pixel;
pub mod refusal;
pub mod ring;
pub mod sig;
pub mod snapshot;
pub mod token;
pub mod version;
pub mod wire;

pub use caps::{derive, extensions_string, version_string, Capabilities};
pub use command::{Command, DrawCommand, Frame};
pub use contract::{find, Entry, Owner, ENTRIES};
pub use refusal::{admit_call, refusal_marker, Refusal};
pub use ring::{OwnedRing, Ring, RingError};
pub use token::{Token, TokenError, TokenTable};
pub use version::{Ext, Home, Version};
pub use wire::{WireError, ABI_VERSION};
