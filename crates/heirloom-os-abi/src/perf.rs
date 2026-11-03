//! The ENGINE'S half of a machine's frame ledger ([`crate::OsVTable::perf_attach`], appended to OS ABI v3 on 2026-11-03, h86
//! aot-ledger): what the engine process's own thread does between and around the library's calls, and the calling thread's
//! hardware counters, handed to the library once so it can close every guest frame with ONE exclusive account of its wall time.
//!
//! WHY (the WC3 council, 2026-11-03): the stage clock's `interp + native + outside_step` did not add up (3.05 M guest
//! instructions at 56 MIPS is 54.5 ms, but "interp" was 37 ms and natives 12.4), and `outside_step` was one opaque 7-8 ms. The
//! library cannot see the engine's time (waiting for the app's next batch, serving the batch, the GL worker's back-pressure),
//! and the engine cannot see the guest's frames (a frame closes at the guest's swap, inside a library call). So the engine
//! keeps an [`EnginePhases`] state machine - exactly one phase open at a time, every TSC tick of its thread charged to one -
//! in its own memory, and the library reads it at each swap. Same process, same thread: no lock, no copy, no race.
//!
//! The counters ([`HwSample`]) are the engine's syscall (on CoralOS PERF_SAMPLE: the thread's own APERF/MPERF, fixed
//! counters and one programmable pair, accumulated by the kernel over its run intervals only). The library names a pair from
//! [`HW_EVENTS`] by index; the engine maps the names to its kernel's events.
use core::ffi::c_void;

/// What the engine thread is doing (an [`EnginePhases`] index).
pub const PH_WAIT_APP: usize = 0;
/// Serving a batch outside the library's `apply` (decoding, the engine's own frames, building the reply, sending it).
pub const PH_SERVE: usize = 1;
/// The engine's own lines and bookkeeping between batches (log writes, rate, stats).
pub const PH_LOG: usize = 2;
/// Inside the library (`apply`): the library's own ledger splits it.
pub const PH_LIB: usize = 3;
/// Inside a GL transact (the guest's GL batch to the worker), outside the two below: scanning, reports, copies.
pub const PH_GL_CARRIER: usize = 4;
/// Collecting the worker's reply to the batch in flight (back-pressure: the guest waits on the GL worker's previous frame).
pub const PH_GL_WAIT: usize = 5;
/// Handing a batch to the worker's ring (submit or exchange).
pub const PH_GL_PUSH: usize = 6;
pub const ENGINE_PHASES: usize = 7;
pub const PHASE_NAMES: [&str; ENGINE_PHASES] = ["wait_app", "serve", "log", "lib", "gl_carrier", "gl_wait", "gl_push"];

/// The engine thread's phase clock: `acc[p]` = ticks charged to phase `p`, `cur` the open phase since `since`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnginePhases {
    pub cur: u32,
    pub _pad: u32,
    pub since: u64,
    pub acc: [u64; ENGINE_PHASES],
}

impl EnginePhases {
    /// Switch to phase `p` at `now`, charging the open one; returns the phase that was open (to return to it).
    pub fn enter(&mut self, p: usize, now: u64) -> usize {
        let prev = self.cur as usize;
        if self.since != 0 && prev < ENGINE_PHASES {
            self.acc[prev] = self.acc[prev].wrapping_add(now.wrapping_sub(self.since));
        }
        (self.cur, self.since) = (p.min(ENGINE_PHASES - 1) as u32, now.max(1));
        prev
    }
    /// Every phase's ticks as of `now`, the open one included.
    pub fn at(&self, now: u64) -> [u64; ENGINE_PHASES] {
        let mut a = self.acc;
        let c = self.cur as usize;
        if self.since != 0 && c < ENGINE_PHASES {
            a[c] = a[c].wrapping_add(now.wrapping_sub(self.since));
        }
        a
    }
}

/// The programmable events a sample may count (index = the wire number in [`PerfHost::sample`]'s `select`): Intel's names, so
/// the engine maps each to its kernel's event by name. 0 = none.
pub const HW_EVENTS: [&str; 13] = [
    "none", "BR_MISP_RETIRED.ALL_BRANCHES", "ICACHE.MISSES", "FETCH_STALL.ICACHE_FILL_PENDING_CYCLES", "NO_ALLOC_CYCLES.NOT_DELIVERED",
    "REHABQ.LD_BLOCK_ST_FORWARD", "MACHINE_CLEARS.ALL", "MACHINE_CLEARS.SMC", "BACLEARS.ALL", "LONGEST_LAT_CACHE.MISS",
    "PAGE_WALKS.I_SIDE_CYCLES", "PAGE_WALKS.D_SIDE_CYCLES", "REHABQ.ANY_LD",
];
/// `select`: keep the pair the thread counts now.
pub const SELECT_KEEP: u32 = u32::MAX;
pub const fn select(a: usize, b: usize) -> u32 {
    (a as u32 & 0xFF) | (b as u32 & 0xFF) << 8
}

pub const HW_APERF: u32 = 1;
pub const HW_FIXED: u32 = 2;
pub const HW_PMC: u32 = 4;

/// The calling thread's counters, cumulative since its metering began (each field valid per `valid`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HwSample {
    pub valid: u32,
    pub cpu: u32,
    /// The pair counted ([`HW_EVENTS`] indices).
    pub events: [u8; 2],
    pub _pad: [u8; 6],
    pub switches: u64,
    pub migrations: u64,
    /// TSC ticks the thread ran (wall time minus this = time off a CPU).
    pub run_tsc: u64,
    pub aperf: u64,
    pub mperf: u64,
    pub inst: u64,
    pub core_cycles: u64,
    pub ref_cycles: u64,
    pub pmc: [u64; 2],
}

/// `sample`'s answers.
pub const HW_OK: i32 = 0;
/// The engine has no counters (no right, no PMU, a host without them): the ledger runs without hardware lines.
pub const HW_NONE: i32 = 1;
/// The pair was refused (no programmable counters, no event table for this CPU, an event the engine cannot map): the sample
/// is written with the previous pair.
pub const HW_PAIR_REFUSED: i32 = 2;

/// What the engine lends the library ([`crate::OsVTable::perf_attach`], copied; every pointer must outlive the instance).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PerfHost {
    /// `size_of::<PerfHost>()` as the engine built it.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// The engine thread's phase clock, written by the engine, read by the library at a swap. Null = none.
    pub phases: *const EnginePhases,
    /// The calling thread's counters into `out`, picking the pair `select` first ([`select`] / [`SELECT_KEEP`]). See HW_*.
    pub sample: extern "C" fn(ctx: *mut c_void, select: u32, out: *mut HwSample) -> i32,
}

impl PerfHost {
    /// The engine's phases as of `now` (None without a clock).
    pub fn phases_at(&self, now: u64) -> Option<[u64; ENGINE_PHASES]> {
        // SAFETY: the engine keeps its EnginePhases alive and unmoved for the instance's life (the attach contract), and writes it
        // only on this same thread, never during a library call.
        unsafe { self.phases.as_ref() }.map(|p| p.at(now))
    }
    /// One counter sample; Err = the HW_* answer.
    pub fn sample(&self, select: u32) -> Result<HwSample, i32> {
        let mut s = HwSample::default();
        match (self.sample)(self.ctx, select, &mut s) {
            HW_OK => Ok(s),
            HW_PAIR_REFUSED => Err(HW_PAIR_REFUSED),
            other => Err(other),
        }
    }
}

/// `perf_attach`: the ledger reads the engine's phases and counters from now on.
pub const PERF_ATTACH_OK: i32 = 0;
/// `perf_attach`: this machine's family keeps no frame ledger, or a null instance / record.
pub const PERF_ATTACH_REFUSED: i32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_phase_is_open_at_a_time_and_every_tick_is_charged_once() {
        let mut p = EnginePhases::default();
        assert_eq!(p.at(50), [0; ENGINE_PHASES], "nothing is charged before the first phase opens");
        p.enter(PH_WAIT_APP, 100);
        let prev = p.enter(PH_LIB, 130);
        assert_eq!(prev, PH_WAIT_APP);
        let inner = p.enter(PH_GL_WAIT, 150);
        p.enter(inner, 190);
        let a = p.at(200);
        assert_eq!((a[PH_WAIT_APP], a[PH_LIB], a[PH_GL_WAIT]), (30, 20 + 10, 40));
        assert_eq!(a.iter().sum::<u64>(), 100, "the sum is the wall time since the first phase opened");
    }

    #[test]
    fn the_wire_shapes_are_fixed() {
        assert_eq!(core::mem::size_of::<HwSample>(), 96);
        assert_eq!(core::mem::size_of::<EnginePhases>(), 16 + 8 * ENGINE_PHASES);
        assert_eq!(select(1, 2), 0x0201);
        assert_eq!(HW_EVENTS[0], "none");
    }
}
