//! Classic Mac OS PowerPC engine, PLUMBING ONLY: everything around the processor that does not need the processor.
//! The PowerPC interpreter (`heirloom-cpu-ppc`, another lane) will run the code this crate places in guest memory.
//!
//! Flow: a PEF fragment ([`heirloom_fmt_pef`]; found directly, or through a `cfrg` resource in an application's resource fork:
//! [`cfrg`], [`source`]) is *instantiated* ([`cfm`]) into the same guest memory and heap `heirloom-mac68k-engine` uses: sections
//! are placed (pattern data unpacked), imports are bound BY SYMBOL NAME ([`symbols`]) and relocations applied, so
//! every imported function becomes a TVector whose code address is a host *glue* stub ([`glue`]). When the CPU's pc
//! reaches a glue stub it calls [`host::PpcHost::glue_call`]; that adapter ([`natives`]) turns r3..r10 into the
//! Toolbox call's arguments and runs the SAME Rust handler the 68k personality runs, through the Mixed Mode seam
//! (`heirloom_mac68k_engine::mixed`). [`census`] reports which imports are bound and which is NEXT-NEEDED.
//!
//! Register and TVector conventions follow the PowerPC Mac OS runtime (Mac OS Runtime Architectures): a function
//! pointer points at a TVector `{code, toc}`; the caller loads both, sets r2 = toc and branches; arguments are in
//! r3..r10 (floating point in f1..f13, not handled here: such imports are NEXT-NEEDED), the result is in r3, and the
//! return goes through LR.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
#[cfg(feature = "fixtures")]
pub mod asm;
/// The PowerPC CPU this engine runs on, named in ONE place (doctrine ONE-NAME-ONE-CONCERN-LOADED): the static core
/// (`cpu-static`, the default: host tests, the macppc-run CLI) or cpu-ppc.library through heirloom-cpu-ppc-client (`cpu-library`:
/// the CoralOS product, which links no PowerPC core). The two expose the same Cpu / Host / Exec / RunStats / Trap / Tier.
pub mod cpu {
    #[cfg(not(feature = "cpu-library"))]
    pub use heirloom_cpu_ppc::{resume, Cpu, Exec, Host, Reg, RunStats, Tier, Trap};
    #[cfg(feature = "cpu-library")]
    pub use heirloom_cpu_ppc_client::{resume, Cpu, Exec, Host, Reg, RunStats, Tier, Trap};
    /// What a product that opened cpu-ppc.library binds before the first run (Heirloom.app while this engine runs in it).
    #[cfg(feature = "cpu-library")]
    pub use heirloom_cpu_ppc_client::{bind, BindRefusal};
    #[cfg(feature = "cpu-library")]
    pub use heirloom_cpu_ppc_client::{PpcVTable, PPC_ABI_VERSION};
}
pub mod census;
pub mod cfm;
pub mod cfrg;
pub mod driver;
pub mod error;
pub mod files;
pub mod gestalt_ppc;
pub mod fsspec;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod gl_extension;
pub mod glue;
pub mod host;
pub mod machine;
pub mod natives;
pub mod osroutines;
pub mod pb;
pub mod run;
pub mod source;
pub mod mp;
pub mod sched;
pub mod stdclib;
pub mod threads;
pub mod alias_c;
pub mod carbon_ctl;
pub mod cf_more;
pub mod decimal;
pub mod dsp_c;
pub mod qdacc_c;
pub mod script_c;
pub mod stdalert_c;
pub mod sysmisc_c;
pub mod tm_c; pub mod gl_c;
pub mod tsm_c;
pub mod gworld;
pub mod invoke_c;
pub mod isp_c;
pub mod qt_c; // the Movie Toolbox calls with no movie decoder: every movie refused by name
pub mod keys_c;
pub mod menu_c;
pub mod toolbox_c;
pub mod stdio;
pub mod symbols;
pub mod ae_c;
pub mod callback;
pub mod carbon;
pub mod cf;
pub mod callback_c;
pub mod clib;
pub mod clib_fmt;
pub mod clib_math;
pub mod clib_num;

pub use error::CfmError;
pub use host::{GlueOutcome, PpcHost};
pub use machine::MacPpc;
pub use heirloom_common::Step;