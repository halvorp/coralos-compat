//! Classic Mac OS 68k engine (SKELETON): the second personality over the SAME `retro-m68k` core and the SAME
//! `guestui-abi` UI trait the Amiga engine uses.
//!
//! Flow: an application file (MacBinary, AppleDouble header, or a raw resource fork; [`segment`] + `retro-resfork`) is
//! launched into an A5 world with its jump table, or a flat 68k image is placed at [`machine::CODE_BASE`]. Toolbox and
//! OS calls are A-line opcodes (`$Axxx`); `retro-m68k` hands each one to [`machine::Mac`]'s `Host::aline`, which looks
//! the trap number up in a table ([`traps`]) and applies ONE of two calling conventions: the Pascal stack frame
//! ([`pascal`], Toolbox traps) or registers ([`osconv`], OS traps). Handlers live in [`toolbox`] (windows),
//! [`menu`], [`events`], [`resmgr`], [`memtraps`] (over [`memmgr`]), [`segment`] and [`quickdraw`]; the low-memory
//! globals they share are the documented map in [`lowmem`]. Anything not in the table, or any bad guest pointer, is a
//! named refusal; nothing is silently accepted.
//!
//! Structure layouts (Rect, WindowRecord) are written from Inside Macintosh and need re-verification against a
//! licence-checked source before they are relied on (see NOTICE).
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod events;
pub mod fixture;
pub mod lowmem;
pub mod machine;
pub mod memmgr;
pub mod memtraps;
pub mod menu;
pub mod osconv;
pub mod pascal;
pub mod quickdraw;
pub mod rects;
pub mod resmgr;
pub mod run;
pub mod segment;
pub mod toolbox;
pub mod trap_names;
pub mod traps;

pub use machine::Mac;
pub use retro_common::{Refusal, Step};
pub use events::Delivered;
pub use run::RunReport;
