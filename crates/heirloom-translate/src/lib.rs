//! The ONE translation crate shared by amiga68k, mac68k and macppc (ruling 17, pieces 2 and 3). It knows no toolkit and no
//! guest; each engine feeds it a toolkit table and a thin apply function:
//!
//! * [`rows`]: the toolkit-neutral ROW TABLES: [`Role`], [`On`], [`Row`], [`lookup`]. A toolkit's table says, per role and
//!   intent, which guest effect happens first and which guest message follows (the payload types are the toolkit's).
//! * [`project`]: the PROJECTION registry [`Projections`]: which guest object is which host control, diff-push on change, a
//!   new generation on a new shape, and the ONE stale-intent rule ([`check_ref`]) every intent passes before any table.
//!
//! `heirloom-ui-abi` stays the seam types only; this crate is the only generic mapper on top of it.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
mod project;
mod rows;

pub use project::{check_ref, same_shape, state, Projected, Projections};
pub use rows::{lookup, lookup_role, on_of, role_of, row, On, Role, Row};
