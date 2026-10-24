//! Classic Mac OS 68k engine (SKELETON): the second personality over the SAME `retro-m68k` core and the SAME
//! `guestui-abi` UI trait the Amiga engine uses.
//!
//! Flow: an application file (MacBinary, AppleDouble header, or a raw resource fork; [`segment`] + `retro-resfork`) is
//! launched into an A5 world with its jump table, or a flat 68k image is placed at [`machine::CODE_BASE`]. Toolbox and
//! OS calls are A-line opcodes (`$Axxx`); `retro-m68k` hands each one to [`machine::Mac`]'s `Host::aline`, which looks
//! the trap number up in a table ([`traps`]) and applies ONE of two calling conventions: the Pascal stack frame
//! ([`pascal`], Toolbox traps) or registers ([`osconv`], OS traps). Handlers live in [`toolbox`] (windows),
//! [`menu`], [`events`], [`resmgr`], [`memtraps`] (over [`memmgr`]), [`segment`], [`quickdraw`] (globals) and [`qd`] (drawing on
//! `retro-raster`, published by `retro-present` exactly as the Amiga engine does); the low-memory
//! globals they share are the documented map in [`lowmem`]. Anything not in the table, or any bad guest pointer, is a
//! named refusal; nothing is silently accepted.
//!
//! Structure layouts (Rect, WindowRecord) are written from Inside Macintosh and need re-verification against a
//! licence-checked source before they are relied on (see NOTICE).
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod addrmode;
pub mod appleevents;
pub mod clock;
pub mod compressed;
pub mod cursor;
pub mod driver;
pub mod events;
pub mod fixmath;
pub mod fm;
pub mod fontnames;
pub mod gestalt;
pub mod guestcall;
pub mod helpmgr;
pub mod intl;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod lowmem;
pub mod machine;
pub mod memmgr;
pub mod memtraps;
pub mod menu;
pub mod mouse;
pub mod menu_items;
pub mod mixed;
pub mod osconv;
pub mod pack7;
pub mod palette;
pub mod package;
pub mod pascal;
pub mod procmgr;
pub mod profile;
pub mod cqd;
pub mod qd;
pub mod quickdraw;
pub mod pack2;
pub mod rects;
pub mod resmgr;
pub mod run;
pub mod scrap;
pub mod script;
pub mod segment;
pub mod strcmp;
pub mod sound;
pub mod system_res;
pub mod sysenv;
pub mod textedit;
pub mod toolbox;
pub mod toolutil;
pub mod winmgr;
pub mod winregions;
pub mod traptable;
pub mod mixed_mode;
pub mod vbl;
pub mod translate;
pub mod trap_names;
pub mod traps;

pub use machine::Mac;
pub use retro_common::{Refusal, Step};
pub use events::Delivered;
pub use run::RunReport;
