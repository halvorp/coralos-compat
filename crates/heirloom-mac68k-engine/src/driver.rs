//! [`GuestMachine`]: the one driver surface the host app shares with the Amiga and PowerPC engines. The program starts from a
//! file (segment.rs), the clock is the host's virtual microseconds over the 60 Hz tick (clock.rs has the boundary rule).
use crate::events::Delivered;
use crate::machine::Mac;
use crate::{Refusal, Step};
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestUi, GuestUiEvent};
use heirloom_common::{Delivery, GuestMachine, RunStats};
use heirloom_volume::Tree;

impl From<Delivered> for Delivery {
    fn from(d: Delivered) -> Delivery {
        match d {
            Delivered::Queued => Delivery::Queued,
            Delivered::Ignored => Delivery::Ignored,
            Delivered::NotForAGuestWindow => Delivery::NotForGuestWindow,
        }
    }
}

impl<U: GuestUi> GuestMachine for Mac<U> {
    type Ui = U;

    /// A 68k application as MacBinary, AppleSingle or AppleDouble (or a raw resource fork); the machine is the profile it
    /// was built with ([`Mac::with_profile`], mac-ii by default).
    fn start_file(&mut self, file: &[u8], name: &str) -> Result<(), Refusal> { self.launch_as(file, name).map_err(Refusal) }
    fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivery, Refusal> { Mac::deliver(self, ev).map(Delivery::from) }
    fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let (step, s) = Mac::step(self, budget);
        (step, RunStats { insns: s.insns, bus_faults: s.bus_faults })
    }
    fn advance_clock_to(&mut self, virtual_us: u64) -> Result<(), Refusal> { self.advance_clock_to_us(virtual_us); Ok(()) }
    /// Mac time is u32 seconds since 1904-01-01: 1904 through 2040-02-06 06:28:15.
    fn set_wall_clock(&mut self, local_unix_secs: i64) -> Result<(), Refusal> {
        let secs = u32::try_from(local_unix_secs + crate::clock::UNIX_TO_MAC_SECS)
            .map_err(|_| Refusal(alloc::format!("wall clock {local_unix_secs} (unix) is outside the Mac's 1904-2040 range")))?;
        self.set_clock(secs)
    }
    fn next_timer_deadline(&self) -> Option<u64> { self.next_deadline_us() }
    fn ui(&self) -> &U { &self.ui }
    fn ui_mut(&mut self) -> &mut U { &mut self.ui }
    fn volume(&mut self) -> &mut Tree { &mut self.fm.tree }
    fn set_stop_on_unimplemented(&mut self, on: bool) { self.stop_on_unimplemented = on; }
    fn drain_log(&mut self) -> Vec<String> { core::mem::take(&mut self.log) }
    fn drain_refused(&mut self) -> Vec<String> { core::mem::take(&mut self.refused) }
}
