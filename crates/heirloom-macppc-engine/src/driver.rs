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
        // A machine made before its program (os-mac.library's, in macppc.engine) takes the program's SIZE partition now; one made
        // for it (MacPpc::for_program) already has it.
        self.size_for_program(file).map_err(|e| Refusal(format!("{name}: {e}")))?;
        // The private libraries the application carries in its own data fork (its cfrg import-library members, e.g.
        // Warcraft III's StormCarbon), as the CFM finds them and as macppc-run already provides them.
        for (lib, lib_bytes) in crate::source::library_fragments(file, None) {
            self.provide_library(&lib, &lib_bytes);
        }
        // and the shared libraries in the application's own folder (the CFM searches it first, before Extensions): Warcraft III's
        // "Miles Carbon Library" sits beside it
        self.provide_folder_libraries();
        if !Pef::has_magic(file) {
            if let Err(why) = self.attach_resources(file) { self.mac.log.push(format!("[cfm] no resource fork attached: {why}")); }
        }
        // The application's own file on the default volume (the folder set_home named, else the boot volume), as the 68k launch
        // does: GetProcessInformation's processAppSpec names it and the program finds the files beside it.
        self.install_app_file(file, &frag_name).map_err(|e| Refusal(format!("{name}: {e}")))?;
        // CurApName / CurApRefNum: the program finds itself the way a real one does (LMGetCurApName, PBGetFCBInfo(CurApRefNum))
        if let Some(app) = self.mac.fm.app {
            let n: Vec<u8> = self.mac.fm.tree.name(app).chars().take(31).map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
            use heirloom_mac68k_engine::lowmem::{CUR_AP_NAME, CUR_AP_REF_NUM};
            let w = self.mac.mem.write_u8(CUR_AP_NAME, n.len() as u8).and_then(|_| self.mac.mem.write_bytes(CUR_AP_NAME + 1, &n))
                .and_then(|_| self.mac.mem.write_u16(CUR_AP_REF_NUM, heirloom_mac68k_engine::resmgr::APP_REF_NUM as u16));
            w.map_err(|e| Refusal(format!("{name}: CurApName: {e}")))?;
        }
        // its init routine's CFragInitBlock names this file and the fragment's span in it (a fat loader reads it to load its real code)
        self.app_span = crate::source::fragment_span(file, None).ok();
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
    fn set_home(&mut self, path: &[&str]) -> Result<(), Refusal> {
        self.mac.fm.set_home(path).map_err(|e| Refusal(format!("the application folder {} is not on the staged volumes (OSErr {e})", path.join(":"))))
    }
    fn set_stop_on_unimplemented(&mut self, on: bool) { self.mac.stop_on_unimplemented = on; }
    fn drain_log(&mut self) -> Vec<String> { core::mem::take(&mut self.mac.log) }
    fn drain_refused(&mut self) -> Vec<String> { core::mem::take(&mut self.mac.refused) }
    fn perf_take_line(&mut self, per_us: u64) -> Option<String> { MacPpc::perf_take_line(self, per_us) }
}
