//! The ONE surface a host drives a heirloom engine through (Amiga 68k, Mac 68k, Mac PowerPC), so a host app has one code path
//! and no per-platform branch: start a program from a file, deliver native window events, step, move the clock, read what the
//! machine logged and refused.
//!
//! **The clock is ONE unit: virtual microseconds** since the machine was created. A host never counts ticks. An engine whose
//! OS counts in another unit converts inside [`GuestMachine::advance_clock_to`] and [`GuestMachine::next_timer_deadline`]
//! and documents the boundary rule there (the Mac engines: 60 Hz ticks, see `heirloom_mac68k_engine::clock`). The contract between
//! the two: advancing to the deadline `next_timer_deadline` returned wakes the wait it names.
//!
//! The pump a host runs per slice: `advance_clock_to(now)`, `deliver` every queued event, `step(budget)`, then on
//! [`Step::Blocked`] arm a timer for `next_timer_deadline()` (none: wait for a window event), on [`Step::BudgetSpent`]
//! continue at once. The trait is object-safe (`dyn GuestMachine<Ui = U>`): it has no generic methods and the associated
//! type is named at the use; a slice costs one virtual call, so nothing is boxed on the hot path. A host that knows its
//! engine statically can use the generic form and pay nothing.
//!
//! The volume is [`heirloom_volume::Tree`], the tree all three engines share, so the host stages files the same way for every
//! platform (`add_volume`, `mkdirs`, `put_file`). What is one platform's own stays on the concrete type (AmigaDOS assigns and
//! `Dev:a/b` path syntax: `Amiga::fs.vol`).
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestUi, GuestUiEvent};
use heirloom_volume::Tree;
use crate::{Refusal, Step};

/// What a slice executed. `bus_faults` counts guest accesses outside RAM on the 68k engines (a bridge that validates its
/// pointers keeps it 0); the PowerPC interpreter reports a bad access as `Step::Fault` instead, so it is always 0 there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunStats { pub insns: u64, pub bus_faults: u32 }

/// What [`GuestMachine::deliver`] did with a native event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Became an event the guest will receive (an IntuiMessage, an EventRecord).
    Queued,
    /// Consumed by an engine-side toolkit or control without a guest message (a MUI object, a scrolled list).
    Handled,
    /// Accepted and deliberately not turned into anything (the guest did not ask for it, or the platform polls for it).
    Ignored,
    /// The window is not one of this guest's.
    NotForGuestWindow,
}

pub trait GuestMachine {
    /// The window system the machine draws on.
    type Ui: GuestUi;

    /// Start the program in `file` (the bytes of a file from the volume) under `name` (its file name). The container is the
    /// platform's own: an AmigaOS Hunk executable; a Mac 68k application as MacBinary, AppleSingle or AppleDouble (its resource
    /// fork holds the code; `name` names a program whose container carries no name of its own); a Mac PowerPC application
    /// as MacBinary/AppleSingle with a `cfrg`, or a bare PEF. Only a fresh machine starts a program. A file the platform cannot
    /// run (wrong container, no code, a data fork the single file does not hold) is a named refusal.
    fn start_file(&mut self, file: &[u8], name: &str) -> Result<(), Refusal>;

    /// The program's own folder on the staged volumes, volume first (`["Warcraft III"]`), set BEFORE [`start_file`](Self::start_file):
    /// the program is launched from there (a Mac application's file is installed in it and it is the default directory, so the
    /// program finds the files beside it). A family with no application folder, or a folder that is not on the volumes, is a
    /// named refusal; the default is the former.
    fn set_home(&mut self, path: &[&str]) -> Result<(), Refusal> {
        Err(Refusal(alloc::format!("this engine has no application folder to set (asked for {})", path.join(":"))))
    }

    /// A native event for the guest. It never runs guest code: the next [`step`](Self::step) does.
    fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivery, Refusal>;

    /// Run at most `budget` instructions.
    fn step(&mut self, budget: u64) -> (Step, RunStats);

    /// Move the virtual clock to `virtual_us` (never backwards) and complete every wait that is due by then.
    fn advance_clock_to(&mut self, virtual_us: u64) -> Result<(), Refusal>;
    /// The guest's LOCAL wall time, in seconds since 1970-01-01 as read on the host's clock with its time zone applied (the
    /// Amiga and the classic Mac keep local time and know no zone), at virtual time 0. Call before `start_file`; a host that
    /// never calls it leaves the engine's documented default date. A time the platform cannot hold (before its epoch, or past
    /// its range) is a named refusal.
    fn set_wall_clock(&mut self, local_unix_secs: i64) -> Result<(), Refusal>;

    /// The earliest virtual microsecond at which a blocked guest wakes by itself, if one is waiting on time. `None`: it
    /// waits for a window event (or is not blocked).
    fn next_timer_deadline(&self) -> Option<u64>;

    fn ui(&self) -> &Self::Ui;
    fn ui_mut(&mut self) -> &mut Self::Ui;

    /// The guest's volumes, shared by every platform. Fill it before [`start_file`](Self::start_file).
    fn volume(&mut self) -> &mut Tree;

    /// Census mode: the first call to a function nobody implemented stops the guest by name instead of returning NULL into it.
    fn set_stop_on_unimplemented(&mut self, on: bool);

    /// Every bridged call since the last drain, one line each. A long-lived host drains (or discards) every pump.
    fn drain_log(&mut self) -> Vec<String>;

    /// Every refusal since the last drain, in order: a bridge call the engine refused by name.
    fn drain_refused(&mut self) -> Vec<String>;
}
