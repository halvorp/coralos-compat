//! [`GuestMachine`]: the one surface a host drives this engine through, like the Amiga and Mac engines.
//!
//! **UI ownership.** `EkaArm` owns the UI (`Box<U>`) at all times a host can see it, so `ui()` / `ui_mut()` are plain
//! borrows. The process holds its UI as `Box<dyn EngineUi>`; `step` lends the box to the process for the slice and takes it
//! back afterwards (`into_any` + a downcast to the `Box<U>` that was lent, so nothing moves or allocates per slice).
//!
//! **Volume.** Before `start_file` the host fills `volume()` (a `Z` volume exists from `new`); `start_file` moves the tree
//! into the process's [`Drive`] and `volume()` then borrows it from there (no copy of the files).
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::{Delivery, GuestMachine, Refusal, RunStats, Step};
use heirloom_fmt_e32::OrdinalTable;
use heirloom_ui_abi::{GuestUi, GuestUiEvent};
use heirloom_volume::Tree;
use crate::drive::Drive;
use crate::loader::{self, Process};
use crate::Outcome;

/// Seconds from 0 AD (the Symbian `TTime` epoch) to 1970-01-01.
const SECS_0AD_TO_1970: i64 = 62_167_219_200;

pub struct EkaArm<U: GuestUi> {
    ui: Option<Box<U>>,
    /// The S60 export names (dll, ordinal) -> mangled name the imports are bound by. A data FILE the host reads (Heirloom.app
    /// ships heirloom-fmt-e32's eka-ordinals.tsv in its bundle) and hands in: never baked into the engine (ASSETS-ARE-FILES).
    /// None until the host hands one in (an engine process receives it as a named input; see [`EkaArm::without_ordinals`]).
    ordinals: Option<OrdinalTable>,
    tree: Tree,
    /// The device ROM (`Z:`) the host hands in before `start_file` (a read-only source of resource files; never copied into the engine).
    rom: Option<Box<dyn crate::drive::RomSource>>,
    /// The layout data file the host extracted for this device (`cdlextract`), handed to the process at `start_file`.
    cdl_data: Option<Vec<u8>>,
    process: Option<Process>,
    /// How the run ended once it has (Exited / Fault are terminal: a later `step` repeats it).
    done: Option<Step>,
    stop_on_unimplemented: bool,
    clock_us: u64,
    /// Local wall time at virtual time 0, microseconds since 0 AD (set by `set_wall_clock`; no guest call reads it yet).
    wall_us_since_0ad: Option<i64>,
    log: Vec<String>,
    refused: Vec<String>,
}

impl<U: GuestUi> EkaArm<U> {
    pub fn new(ui: U, ordinals: OrdinalTable) -> Self {
        let mut m = Self::without_ordinals(ui);
        m.ordinals = Some(ordinals);
        m
    }
    /// A machine whose ordinal table arrives later ([`set_ordinals`](Self::set_ordinals), before `start_file`; until then
    /// `start_file` refuses by name). os-eka.library makes its machines this way: the table is a file the host sends.
    pub fn without_ordinals(ui: U) -> Self {
        let mut tree = Tree::new(255);
        tree.add_volume("Z", false);
        EkaArm { ui: Some(Box::new(ui)), ordinals: None, tree, rom: None, cdl_data: None, process: None, done: None, stop_on_unimplemented: true, clock_us: 0, wall_us_since_0ad: None, log: Vec::new(), refused: Vec::new() }
    }
    /// The S60 export names the imports are bound by (heirloom-fmt-e32's eka-ordinals.tsv), before `start_file`.
    pub fn set_ordinals(&mut self, ordinals: OrdinalTable) {
        self.ordinals = Some(ordinals);
    }
    /// Put the device ROM behind the volume before `start_file`: the environment reads its colour list from the ROM's `eikpriv` resource file.
    pub fn set_rom(&mut self, rom: Box<dyn crate::drive::RomSource>) {
        self.rom = Some(rom);
    }
    /// The device's layout data (the text `cdlextract::extract` makes) before `start_file`: the CDL engine serves the layout queries from it.
    pub fn set_cdl_data(&mut self, bytes: Vec<u8>) {
        self.cdl_data = Some(bytes);
    }
    /// The running process, once started (tests and the runner read its log and registers).
    pub fn process(&self) -> Option<&Process> {
        self.process.as_ref()
    }
    fn apply_census(&mut self) {
        if let Some(p) = self.process.as_mut() {
            p.census = if self.stop_on_unimplemented { None } else { Some(p.census.take().unwrap_or_default()) };
        }
    }
}

impl<U: GuestUi> GuestMachine for EkaArm<U> {
    type Ui = U;

    /// A Symbian E32 executable (an EXE image, as `sys\bin\<app>.exe`). `name` is only logged: the image carries its own UIDs.
    fn start_file(&mut self, file: &[u8], name: &str) -> Result<(), Refusal> {
        if self.process.is_some() {
            return Err(Refusal(String::from("ekaarm: only a fresh machine starts a program")));
        }
        let Some(ordinals) = self.ordinals.as_ref() else {
            return Err(Refusal(format!("ekaarm: {name}: no ordinal table was handed in (the S60 export names every import binds by; eka-ordinals.tsv)")));
        };
        let mut p = loader::load(file, ordinals).map_err(|e| Refusal(format!("ekaarm: {name}: not a loadable E32 image: {e:?}")))?;
        let mut drive = Drive::from_tree(core::mem::replace(&mut self.tree, Tree::new(255)));
        if let Some(rom) = self.rom.take() {
            drive.set_rom(rom);
        }
        p.set_drive(drive);
        if let Some(data) = self.cdl_data.take() {
            p.set_cdl_data(&data).map_err(|e| Refusal(format!("ekaarm: {name}: the layout data is refused: {e}")))?;
        }
        self.log.push(format!("ekaarm: started {name} (UID3 {:#010x})", p.uid3));
        self.process = Some(p);
        self.apply_census();
        Ok(())
    }

    /// A host key press or release reaches the guest through the environment's own path (`wsevent.rs`, `keyroute.rs`): it becomes window
    /// server events, completes the environment's outstanding event request and the scheduler runs it on the next `step`. The key's
    /// `code` is its `TStdScanCode` (`keymap.rs`). Every other event kind is accepted and dropped, by name.
    /// DELETE WHEN pointer, menu-command, close and resize events are routed (the slices after this one).
    fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivery, Refusal> {
        let GuestUiEvent::Key { win, code, text, modifiers, down } = ev else {
            self.log.push(String::from("ekaarm: deliver: only key events are routed so far; this event was ignored"));
            return Ok(Delivery::Ignored);
        };
        let Some(p) = self.process.as_mut() else {
            return Err(Refusal(String::from("ekaarm: deliver: no program started (start_file first)")));
        };
        if p.host.window != Some(win) {
            return Ok(Delivery::NotForGuestWindow);
        }
        p.deliver_key(code, &text, modifiers, down).map_err(|m| Refusal(format!("ekaarm: deliver: {m}")))?;
        Ok(Delivery::Queued)
    }

    fn step(&mut self, budget: u64) -> (Step, RunStats) {
        if let Some(d) = &self.done {
            return (d.clone(), RunStats::default());
        }
        let (Some(p), Some(ui)) = (self.process.as_mut(), self.ui.take()) else {
            return (Step::Fault(String::from("ekaarm: step: no program started (start_file first)")), RunStats::default());
        };
        let before = p.insns;
        p.set_ui_boxed(ui);
        let outcome = p.run(before.saturating_add(budget));
        let insns = p.insns - before;
        self.ui = p.take_ui().and_then(|b| b.into_any().downcast::<U>().ok());
        p.census.as_mut().map(|c| self.refused.append(c));
        p.host.ui = None;
        self.log.append(&mut p.log);
        let step = match outcome {
            Outcome::Exited(_) => Step::Exited,
            Outcome::BudgetSpent => Step::BudgetSpent,
            Outcome::Blocked => Step::Blocked,
            Outcome::Refused(m) => { self.refused.push(m.clone()); Step::Fault(m) }
            Outcome::Fault(m) => { self.log.push(m.clone()); Step::Fault(m) }
        };
        if matches!(step, Step::Exited | Step::Fault(_)) {
            self.done = Some(step.clone());
        }
        (step, RunStats { insns, bus_faults: 0 })
    }

    fn advance_clock_to(&mut self, virtual_us: u64) -> Result<(), Refusal> {
        if virtual_us < self.clock_us {
            return Err(Refusal(format!("ekaarm: the clock does not run backwards ({virtual_us} us < {} us)", self.clock_us)));
        }
        self.clock_us = virtual_us;
        Ok(())
    }

    fn set_wall_clock(&mut self, local_unix_secs: i64) -> Result<(), Refusal> {
        if local_unix_secs < 0 {
            return Err(Refusal(format!("ekaarm: wall clock {local_unix_secs} s is before 1970: this engine takes only dates from the Unix epoch on")));
        }
        let us = local_unix_secs.checked_add(SECS_0AD_TO_1970).and_then(|s| s.checked_mul(1_000_000));
        let us = us.ok_or_else(|| Refusal(format!("ekaarm: wall clock {local_unix_secs} s is past the range of a Symbian TTime")))?;
        self.wall_us_since_0ad = Some(us);
        self.log.push(format!("ekaarm: wall clock set ({us} us since 0 AD); no guest call reads it yet"));
        Ok(())
    }

    /// Nothing in the engine waits on time: no Symbian timer (RTimer, CPeriodic) is implemented, so a `Blocked` step has no deadline.
    /// DELETE WHEN the first timer is (`RTimer::After` issuing a request on `request.rs`): then the nearest deadline is returned here.
    fn next_timer_deadline(&self) -> Option<u64> {
        None
    }

    fn ui(&self) -> &U {
        match &self.ui { Some(u) => u, None => unreachable!("EkaArm lends its UI only inside step, which restores it") }
    }
    fn ui_mut(&mut self) -> &mut U {
        match &mut self.ui { Some(u) => u, None => unreachable!("EkaArm lends its UI only inside step, which restores it") }
    }

    fn volume(&mut self) -> &mut Tree {
        if self.process.as_ref().is_some_and(|p| p.host.drive.is_some()) {
            if let Some(d) = self.process.as_mut().and_then(|p| p.host.drive.as_mut()) {
                return d.tree_mut();
            }
        }
        &mut self.tree
    }

    fn set_stop_on_unimplemented(&mut self, on: bool) {
        self.stop_on_unimplemented = on;
        self.apply_census();
    }
    fn drain_log(&mut self) -> Vec<String> {
        core::mem::take(&mut self.log)
    }
    fn drain_refused(&mut self) -> Vec<String> {
        core::mem::take(&mut self.refused)
    }
}
