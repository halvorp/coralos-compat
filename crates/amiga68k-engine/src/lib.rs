//! AmigaOS 68k engine (skeleton).
//!
//! Flow: a Hunk executable is loaded into guest RAM; a fake ExecBase sits at the longword
//! at address 4; every library function slot (LVO) below a library base holds
//! `ILLEGAL ($4AFC); RTS ($4E75)`. Musashi's illegal hook lands in [`Amiga`]'s `Host::illegal`,
//! which maps the slot address to a registered handler. The handler reads guest registers
//! and structs through checked memory, calls [`guestui_abi::GuestUi`], sets D0, and the
//! guest then executes the slot's own RTS. Anything not registered, or any bad guest
//! pointer, is a named refusal that returns NULL; nothing is silently accepted.
//!
//! Struct offsets and LVO numbers are written from public NDK-style documentation and
//! need re-verification against a licence-checked source (see NOTICE).
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod asl;
pub mod boopsi;
pub mod boopsi_gadget;
pub mod boopsi_draw;
pub mod boopsi_frame;
pub mod boopsi_prop;
pub mod boopsi_glist;
pub mod boopsi_ids;
pub mod boopsi_input;
pub mod boopsi_native;
pub mod boopsi_send;
pub mod boopsi_strg;
pub mod native_gadgets;
pub mod native_gadgets_input;
pub mod translate;
pub mod iclass;
pub mod utility;
pub mod census;
pub mod devices;
pub mod devproc;
pub mod disklib;
pub mod dos;
pub mod dos2;
pub mod doslist;
pub mod dos_assign;
pub mod dos_date;
pub mod dos_err;
pub mod dos_fault;
pub mod dos_files;
pub mod dos_fs;
pub mod dos_io;
pub mod dos_match;
pub mod dos_misc;
pub mod dos_notify;
pub mod dos_path;
pub mod dos_pkt;
pub mod dos_proc;
pub mod dos_text;
pub mod exall;
pub mod exec;
pub mod execlists;
pub mod exec_wait;
pub mod fs;
pub mod gadtools;
pub mod gt_kinds;
pub mod gt_menus;
pub mod graphics;
pub mod idcmp;
pub mod guest_call;
pub mod fixture;
pub mod intuition;
pub mod libtable;
pub mod loadseg;
pub mod lvo_names;
pub mod guest_str;
pub mod icon;
pub mod icon_format;
pub mod icon_host;
pub mod libfactory;
pub mod lists;
pub mod lowlevel;
pub mod machine;
pub mod menu_nav;
pub mod makelib;
pub mod mathieee;
pub mod memheader;
pub mod memory;
pub mod mui;
pub mod pattern;
pub mod ports;
pub mod process;
pub mod rdargs;
pub mod run;
pub mod screen;
pub mod cache;
pub mod commodities;
pub mod commodities_parse;
pub mod cx_tables;
pub mod exec_misc;
pub mod exec_patch;
pub mod fmt;
pub mod rawdofmt;
pub mod locale;
pub mod locale_fmt;
pub mod locale_tables;
pub mod sched;
pub mod semaphores;
pub mod exec_task;
pub mod dos_newproc;
pub mod tags;
pub mod timer;
pub mod vars;
pub mod volume;
pub mod window;
pub mod workbench;
mod menu;

pub use machine::{Amiga, RunReport};
pub use retro_common::Step;
pub use run::Delivered;

pub use retro_common::Refusal;
