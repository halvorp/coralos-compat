//! Classic Mac OS 68k engine (SKELETON): the second personality over the SAME `retro-m68k` core and the SAME
//! `guestui-abi` UI trait the Amiga engine uses.
//!
//! Flow: a flat 68k code image is placed at a fixed address ([`machine::CODE_BASE`]); there is NO resource fork /
//! `CODE` resource loader yet (the next step, see docs/MAC68K-SKELETON.md). Toolbox and OS calls are A-line opcodes
//! (`$Axxx`); `retro-m68k` hands each one to [`machine::Mac`]'s `Host::aline`, which looks the trap number up in a
//! table ([`traps`]), reads the Pascal-convention arguments off the guest stack ONCE for every trap ([`pascal`]),
//! runs the named handler ([`toolbox`]) and writes the function result back. Anything not in the table, or any bad
//! guest pointer, is a named refusal; nothing is silently accepted.
//!
//! Structure layouts (Rect, WindowRecord) are written from Inside Macintosh and need re-verification against a
//! licence-checked source before they are relied on (see NOTICE).
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod fixture;
pub mod machine;
pub mod pascal;
pub mod run;
pub mod toolbox;
pub mod trap_names;
pub mod traps;

pub use machine::Mac;
pub use retro_common::{Refusal, Step};
pub use run::{Delivered, RunReport};
