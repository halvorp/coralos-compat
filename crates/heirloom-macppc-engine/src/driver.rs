//! [`GuestMachine`]: the one driver surface the host app shares with the Amiga and 68k Mac engines. A PowerPC application
//! starts from its file (MacBinary / AppleSingle with a `cfrg`, or a bare PEF); the clock is the host's virtual microseconds
//! over the 60 Hz tick, by the same rule as the 68k engine (`heirloom_mac68k_engine::clock`).
use crate::machine::MacPpc;
use crate::source::fragment_bytes;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestUi, GuestUiEvent};
use heirloom_mac68k_engine::clock::us_at_tick;
use heirloom_common::{Delivery, GuestMachine, Refusal, RunStats, Step};
use heirloom_fmt_pef::Pef;
use heirloom_volume::Tree;

impl<U: GuestUi> MacPpc<U> {
    /// The virtual microsecond at which a parked `WaitNextEvent` times out; `None` when nothing waits on time.
    fn wait_deadline_us(&self) -> Option<u64> {
        self.rt.wait_mask?;
        let ahead = self.rt.wait_deadline?.wrapping_sub(self.mac.ev.ticks);
        let left = if ahead < 0x8000_0000 { u64::from(ahead) } else { 0 };
        Some(us_at_tick(self.mac.ev.ticks_total + left))
    }
}

impl<U: GuestUi> GuestMachine for MacPpc<U> {
    type Ui = U;

    /// A PowerPC application: its data fork holds the PEF, so an AppleDouble header file (whose data fork is a separate file)
    /// is refused by name (`NeedDataFork`): one file cannot carry both. A file without a resource fork runs without resources
    /// (logged); a fat application runs its PowerPC half.
    fn start_file(&mut self, file: &[u8], name: &str) -> Result<(), Refusal> {
        let (frag_name, bytes) = fragment_bytes(file, None).map_err(|e| Refusal(format!("{name}: {e}")))?;
        let pef = Pef::parse(&bytes).map_err(|e| Refusal(format!("{name}: {e}")))?;
        // The private libraries the application carries in its own data fork (its cfrg import-library members, e.g.
        // Warcraft III's StormCarbon), as the CFM finds them and as macppc-run already provides them.
        for (lib, lib_bytes) in crate::source::library_fragments(file, None) {
            self.provide_library(&lib, &lib_bytes);
        }
        if !Pef::has_magic(file) {
            if let Err(why) = self.attach_resources(file) { self.mac.log.push(format!("[cfm] no resource fork attached: {why}")); }
        }
        let frag = self.instantiate(&frag_name, &pef).map_err(|e| Refusal(format!("{name}: {e}")))?;
        MacPpc::start(self, &frag).map_err(|e| Refusal(format!("{name}: {e}")))
    }
    fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivery, Refusal> { self.mac.deliver(ev).map(Delivery::from) }
    fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let (step, s) = MacPpc::step(self, budget);
        (step, RunStats { insns: s.insns, bus_faults: 0 })
    }
    fn advance_clock_to(&mut self, virtual_us: u64) -> Result<(), Refusal> { self.mac.advance_clock_to_us(virtual_us); Ok(()) }
    fn set_wall_clock(&mut self, local_unix_secs: i64) -> Result<(), Refusal> { self.mac.set_wall_clock(local_unix_secs) }
    fn next_timer_deadline(&self) -> Option<u64> { self.wait_deadline_us() }
    fn ui(&self) -> &U { &self.mac.ui }
    fn ui_mut(&mut self) -> &mut U { &mut self.mac.ui }
    fn volume(&mut self) -> &mut Tree { &mut self.mac.fm.tree }
    fn set_stop_on_unimplemented(&mut self, on: bool) { self.mac.stop_on_unimplemented = on; }
    fn drain_log(&mut self) -> Vec<String> { core::mem::take(&mut self.mac.log) }
    fn drain_refused(&mut self) -> Vec<String> { core::mem::take(&mut self.mac.refused) }
}
