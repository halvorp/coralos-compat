//! The one generic binding interpreter (ruling 17: bindings are DATA; coralos doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING).
//!
//! Turns a program's own version-bound binding data and drawn chrome into
//! named seam ops (`DrawOp::Face`). Contains zero application-specific names
//! or knowledge; it is the only crate that may make a `DrawOp::Face`
//! (compat tools/check-translation-seam.sh clause (d)).
//!
//! # The binding file (s-expression, coralos's one grammar: decimal integers, `true`/`false`, strings)
//!
//! This crate parses NO text: the host reads a binding file with its s-expression reader and builds the [`BindingSpec`] value.
//! The shape the converter and the data agree on (field offsets are bytes from the start of the chunk's data; integers big-endian):
//!
//! ```text
//! (binding (id "<program>-<version>")
//!   (file (iff-form "FORM-TYPE"))   ; plus (legacy-form "OLD-TYPE") only when that form holds nothing but this schema's old version
//!   (group (chunk "GRUP") (size 132) (title (cstr 0 32)) (window-box (box 32)) (inset 1) (no-inset-if (flag u32 128 16)))
//!   (item (chunk "ITEM") (min-size 14) (skip-if (flag u32 2 4))
//!         (label (strings 14) (count (field i16 6) (unless (flag u32 2 16) 3)) (per-record 2) (pick 0)))
//!   (placement (chunk "PLCE") (min-size 29) (window-box (box 20)) (path (cstr 28 256)))
//!   (anchor (label-in-fill (prefix-cut true) (strip-lines-inside true) (accent-from-swatch true))))
//! ```
//! `(flag <width> <at> <mask>)` is a [`FlagTest`]; `(unless <flag> <n>)` means "n records when the flag is CLEAR".

#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod anchor;
pub mod parse;
pub mod spec;
pub mod state;

pub use parse::{parse, recognise, Group, Label, Parsed, Placement, Recognised, Refusal};
pub use spec::{
    AnchorRule, BindingSpec, BoxField, CStrField, CountRule, FileForm, FlagTest, GroupLayout,
    IntWidth, ItemLayout, PlacementLayout, StrListPick,
};
pub use state::Bindings;
