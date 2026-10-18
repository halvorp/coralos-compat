//! Classic Mac OS PowerPC engine, PLUMBING ONLY: everything around the processor that does not need the processor.
//! The PowerPC interpreter (`retro-ppc`, another lane) will run the code this crate places in guest memory.
//!
//! Flow: a PEF fragment ([`retro_pef`]; found directly, or through a `cfrg` resource in an application's resource fork:
//! [`cfrg`], [`source`]) is *instantiated* ([`cfm`]) into the same guest memory and heap `mac68k-engine` uses: sections
//! are placed (pattern data unpacked), imports are bound BY SYMBOL NAME ([`symbols`]) and relocations applied, so
//! every imported function becomes a TVector whose code address is a host *glue* stub ([`glue`]). When the CPU's pc
//! reaches a glue stub it calls [`host::PpcHost::glue_call`]; that adapter ([`natives`]) turns r3..r10 into the
//! Toolbox call's arguments and runs the SAME Rust handler the 68k personality runs, through the Mixed Mode seam
//! (`mac68k_engine::mixed`). [`census`] reports which imports are bound and which is NEXT-NEEDED.
//!
//! Register and TVector conventions follow the PowerPC Mac OS runtime (Mac OS Runtime Architectures): a function
//! pointer points at a TVector `{code, toc}`; the caller loads both, sets r2 = toc and branches; arguments are in
//! r3..r10 (floating point in f1..f13, not handled here: such imports are NEXT-NEEDED), the result is in r3, and the
//! return goes through LR.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod asm;
pub mod census;
pub mod cfm;
pub mod cfrg;
pub mod error;
pub mod fixture;
pub mod glue;
pub mod host;
pub mod machine;
pub mod natives;
pub mod run;
pub mod source;
pub mod stdclib;
pub mod symbols;

pub use error::CfmError;
pub use host::{GlueOutcome, PpcHost};
pub use machine::MacPpc;
pub use retro_common::Step;
