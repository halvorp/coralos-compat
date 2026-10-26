//! [`GuestMachine`]: the one driver surface the host app shares with the Mac engines. Each method is the engine's own
//! (run.rs, timer.rs, sched.rs); this only adapts the run types.
use crate::machine::Amiga;
use crate::run::Delivered;
use crate::{Refusal, Step};
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestUi, GuestUiEvent};
use heirloom_common::{Delivery, GuestMachine, RunStats};
use heirloom_volume::Tree;

impl From<Delivered> for Delivery {
    fn from(d: Delivered) -> Delivery {
        match d {
            Delivered::Queued(_) => Delivery::Queued,
            Delivered::Handled => Delivery::Handled,
            Delivered::IgnoredUnsubscribed => Delivery::Ignored,
            Delivered::NotForAGuestWindow => Delivery::NotForGuestWindow,
        }
    }
}

impl<U: GuestUi> GuestMachine for Amiga<U> {
    type Ui = U;

    /// A Hunk executable; `name` is the guest task's and CLI's command name and the title of its untitled windows.
    fn start_file(&mut self, file: &[u8], name: &str) -> Result<(), Refusal> {
        self.start_hunk_as(file, name).map(|_| ()).map_err(Refusal)
    }
    fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivery, Refusal> { Amiga::deliver(self, ev).map(Delivery::from) }
    fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let (step, s) = Amiga::step(self, budget);
        (step, RunStats { insns: s.insns, bus_faults: s.bus_faults })
    }
    /// The Amiga's timer.device counts in microseconds already: the virtual clock is the guest's.
    fn advance_clock_to(&mut self, virtual_us: u64) -> Result<(), Refusal> { Amiga::advance_clock_to(self, virtual_us) }
    fn next_timer_deadline(&self) -> Option<u64> { Amiga::next_timer_deadline(self) }
    fn ui(&self) -> &U { &self.ui }
    fn ui_mut(&mut self) -> &mut U { &mut self.ui }
    fn volume(&mut self) -> &mut Tree { &mut self.fs.vol }
    fn set_stop_on_unimplemented(&mut self, on: bool) { self.stop_on_unimplemented = on; }
    fn drain_log(&mut self) -> Vec<String> { core::mem::take(&mut self.log) }
    fn drain_refused(&mut self) -> Vec<String> { core::mem::take(&mut self.refused) }
}
