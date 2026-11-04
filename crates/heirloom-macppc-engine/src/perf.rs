//! Where a guest frame's time goes (doc 22 phase 5, WC3 row I10): an OPT-IN stage clock over the run loop.
//!
//! The engine owns no time source, so the host hands one in ([`MacPpc::perf_enable`]: any monotonic clock - nanoseconds on a
//! desktop host, TSC cycles on CoralOS) and says its units per microsecond when it takes a line (a TSC's rate is measured by
//! the host, coral-tsc-rate, so it is known only after the first second). Off (the default) it costs one `Option` test per native
//! call and per step. On, it costs two clock reads per native call and per step, nothing per instruction:
//!
//! * `step` time = everything inside [`MacPpc::step`]; `native` time = inside a glue call, split by what the call is
//!   ([`Class`]); the interpreter's own time is `step - native`;
//! * `gl-ring` = inside the GL worker's `transact` (the batch handed to heirloom-gl.engine and its replies), a part of `gl`;
//! * a FRAME is one guest swap (`aglSwapBuffers`): its wall time is swap-to-swap on the host clock, so the time the host spent
//!   OUTSIDE `step` (its event loop, timer waits, the GL worker running elsewhere) is `frame - step`;
//! * `clock-poll` = instructions run between two reads of the Mac clock (`Microseconds`, `UpTime`, `TickCount`) that saw the
//!   SAME tick and came within [`POLL_GAP_INSNS`] of each other: a guest polling a clock that has not moved (a busy-wait).
//!
//! Every [`REPORT_FRAMES`] frames the window is closed and queued for the host ([`MacPpc::perf_take_line`] formats it as one
//! `[HEIRLOOM-PERF ...]` line); the counters then restart, so each line covers its own frames.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;

/// Frames per report line: rate-limited so a long game cannot flood a log ring (one line per 256 swaps).
pub const REPORT_FRAMES: usize = 256;
/// Two clock reads on the same tick within this many instructions are a poll, not work that happened to read the clock twice.
pub const POLL_GAP_INSNS: u64 = 20_000;

/// What a native call is, for the breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class { Gl = 0, Clock = 1, Thread = 2, Event = 3, File = 4, Other = 5 }
pub const CLASSES: usize = 6;
pub const CLASS_NAMES: [&str; CLASSES] = ["gl", "clock", "thread", "event", "file", "other"];

/// One report window's counters, in host clock units.
#[derive(Debug, Clone, Default)]
pub struct Window {
    pub step_time: u64,
    pub steps: u64,
    pub insns: u64,
    pub native_time: [u64; CLASSES],
    pub native_calls: [u64; CLASSES],
    pub gl_ring_time: u64,
    /// The GL draw calls among the GL calls (`glDrawElements`, `glDrawArrays`, `glDrawRangeElements`: their client arrays are
    /// snapshotted) and their time, a part of the GL class's.
    pub draw_time: u64,
    pub draws: u64,
    /// Inside the draws: the capture alone (guest bytes copied into the batch and byte-swapped) and the bytes it queued.
    pub capture_time: u64,
    pub capture_bytes: u64,
    /// The capture's cost per KiB, draw by draw, as a histogram: bucket `b` counts the draws whose capture took `[2^b, 2^(b+1))`
    /// clock units a KiB, and sums their time - a uniformly slow capture (every byte costs: memory) and a few huge ones (a page
    /// fault, an interrupt, a preemption inside one) give the same mean and different shapes. Per KiB, so a big draw is not a
    /// stall; and the slowest single capture.
    pub capture_hist: [u32; HIST],
    pub capture_hist_time: [u64; HIST],
    pub capture_max: u64,
    /// The capture probe ([`CaptureProbe`]): sampled draws and the guest-RAM / heap read passes, this window.
    pub probe: CaptureProbe,
    pub clock_reads: u64,
    pub poll_insns: u64,
    /// Swap-to-swap durations of the frames in this window.
    pub frames: Vec<u64>,
    /// The ledger's frames of this window, the programmable pair they counted, and whether that pair was refused.
    pub ledger: (Vec<crate::perf_ledger::Frame>, (usize, usize), bool),
    /// The tier-2 ablation setting this window ran under (cpu-tier "jitablate": [`ABLATIONS`]); empty = none.
    pub cfg: &'static str,
}

/// cpu-tier "jitablate" (h86 aot-ledger): tier 2 with its guards switched off in turn, each for [`ABLATE_WINDOWS`] perf windows
/// (the first after a switch recompiles everything; the second is the warm one), so ONE Z83 deployment measures what each guard
/// costs. Each knob drops a check that keeps the JIT exact for self-modifying code - a measurement setting, never a default.
pub const ABLATIONS: [(&str, u32); 11] = {
    use heirloom_os_abi::ppc::{KNOB_COLD_TAIL as T, KNOB_FOLLOW_CALLS as C, KNOB_IBTC_BITS_SHIFT as I, KNOB_NO_DISPLACE as D, KNOB_NO_ENTRY_GEN as G, KNOB_NO_LAZY_CA as L, KNOB_NO_SMC_STORE as S, KNOB_NO_XLATE as X};
    [("base", 0), ("noxlate", X), ("nodisplace", D), ("nolazyca", L), ("followcalls", C), ("coldtail", T), ("nosmc", S), ("nogen", G), ("nosmc+nogen", S | G), ("ibtc12", 12 << I), ("ibtc14", 14 << I)]
};
pub const ABLATE_WINDOWS: u64 = 2;

/// Buckets of [`Window::capture_hist`] (log2 of the clock units: 2^31 TSC cycles is ~1.5 s on the Z83).
pub const HIST: usize = 32;
/// One draw in this many is a SAMPLED draw ([`CaptureProbe`]): its capture is taken apart by three extra timed passes.
pub const CAPTURE_SAMPLE: u64 = 32;

/// What the sampled draws and the once-a-window read passes measured (clock units and bytes, summed over the window). On a
/// sampled draw the batch's next [`crate::gl_c::capture_probe::DST_TOUCH`] bytes are written once first (`dst`: the destination's own cost,
/// write-allocating lines last written a frame ago), then the capture runs as always (`cold`: its source as the game left it,
/// its destination now in L1), then the same capture again into a pre-touched scratch buffer (`warm`: source and destination
/// both in L1 - the capture's instructions alone). So per KiB: cold - warm = reading the guest's bytes; warm = the code; dst =
/// writing the batch. Once a window the guest RAM around the first array and a heap block of the same size are each read
/// twice (`ram_*`, `heap_*`): a first pass far over the second is DRAM (or TLB) latency; a SECOND pass as slow as the first
/// means the memory is not being cached at all (an uncached or write-combining mapping).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureProbe {
    pub draws: u64,
    pub bytes: u64,
    pub dst_time: u64,
    pub dst_bytes: u64,
    pub cold_time: u64,
    pub warm_time: u64,
    pub read_bytes: u64,
    pub ram_first: u64,
    pub ram_second: u64,
    pub heap_first: u64,
    pub heap_second: u64,
}

pub struct Perf {
    clock: fn() -> u64,
    pub win: Window,
    last_swap: Option<u64>,
    /// (tick, cpu timebase) at the last clock read.
    last_read: Option<(u64, u64)>,
    /// Class by glue id, filled on first call.
    pub(crate) class_of: Vec<u8>,
    /// Closed windows (frames reported through it, the window), oldest first.
    done: Vec<(u64, Window)>,
    /// Per-frame instruction counts are `insns / frames`; the total frames reported so far names the line.
    pub frames_total: u64,
    /// Frames per line ([`REPORT_FRAMES`] unless the host asks otherwise).
    pub every: usize,
    /// Opt-in diagnostic: poll instructions by the clock read's call chain (LR and the two saved LRs above it).
    pub sites: Option<alloc::collections::BTreeMap<[u32; 3], u64>>,
    /// Draws captured so far (the sampling's counter).
    pub(crate) draw_seq: u64,
    /// The frame ledger (perf_ledger.rs): the exclusive per-frame account, the engine's half once attached.
    pub(crate) ledger: crate::perf_ledger::Ledger,
    /// Lines of a closed window not yet taken (the `[HEIRLOOM-PERF ...]` line first, then the ledger's).
    pending: alloc::collections::VecDeque<String>,
    /// The CPU's build line was said (once, after the first run that reported).
    lib_said: bool,
    /// What a swap (inside a run) left for the run's end (MacPpc::after_run): the next knobs, the closed windows' CPU-PPC-TIER
    /// lines (ABLATIONS index, window), and tier 2's event counts as of the last run's end.
    knobs_due: Option<u32>,
    tier_line_due: Vec<(usize, u64)>,
    jit_now: [u64; 4],
    /// Guest instructions retired at the last native's entry (the CPU's timebase): a swap is inside a native.
    pub(crate) tb: u64,
    /// cpu-tier "jitablate": rotate tier 2's knobs through [`ABLATIONS`] (the index of the running one).
    pub(crate) ablate: Option<usize>,
    /// cpu-tier "jitprofile": the guest-PC profile (profile.rs), and the closed ones waiting for their lines.
    pub(crate) profile: Option<alloc::boxed::Box<crate::profile::Profile>>,
    profile_done: Vec<(u64, crate::profile::Profile)>,
}

impl Perf {
    pub fn now(&self) -> u64 { (self.clock)() }
    pub(crate) fn clock_fn(&self) -> fn() -> u64 { self.clock }
    pub(crate) fn add_native(&mut self, class: Class, dt: u64) {
        self.win.native_time[class as usize] += dt;
        self.win.native_calls[class as usize] += 1;
    }
    pub(crate) fn add_step(&mut self, dt: u64, insns: u64) {
        self.win.step_time += dt;
        self.win.steps += 1;
        self.win.insns += insns;
    }
    pub(crate) fn add_gl_ring(&mut self, dt: u64) { self.win.gl_ring_time += dt; }
    pub(crate) fn add_capture(&mut self, dt: u64, bytes: u64) {
        (self.win.capture_time, self.win.capture_bytes) = (self.win.capture_time + dt, self.win.capture_bytes + bytes);
        let b = (63 - per_kib(dt, bytes.max(1)).max(1).leading_zeros() as usize).min(HIST - 1);
        self.win.capture_max = self.win.capture_max.max(dt);
        self.win.capture_hist[b] += 1;
        self.win.capture_hist_time[b] += dt;
    }
    /// Count a draw; whether it is a sampled one ([`CAPTURE_SAMPLE`]).
    pub(crate) fn capture_sample(&mut self) -> bool {
        self.draw_seq += 1;
        self.draw_seq % CAPTURE_SAMPLE == 0
    }

    /// A read of the Mac clock at `tick` (the engine's tick count) when the CPU's timebase (instructions retired) is `tb`.
    pub(crate) fn clock_read(&mut self, tick: u64, tb: u64, site: [u32; 3]) {
        self.win.clock_reads += 1;
        if let Some((t, b)) = self.last_read {
            let gap = tb.wrapping_sub(b);
            if t == tick && gap <= POLL_GAP_INSNS {
                self.win.poll_insns += gap;
                if let Some(m) = self.sites.as_mut() { *m.entry(site).or_insert(0) += gap; }
            }
        }
        self.last_read = Some((tick, tb));
    }

    /// A guest swap: closes a frame; every [`REPORT_FRAMES`] frames queues a line and restarts the window.
    /// `jit` = tier 2's cumulative events (blocks compiled, publishes, withdraws, bails) for the ledger's frame.
    pub(crate) fn swap(&mut self, jit: [u64; 4]) {
        let now = self.now();
        self.ledger.swap(self.clock, self.tb, jit);
        if let Some(prev) = self.last_swap { self.win.frames.push(now.wrapping_sub(prev)); }
        if let Some(pr) = self.profile.as_mut() { pr.frames += 1; }
        self.last_swap = Some(now);
        if self.win.frames.len() >= self.every.max(1) {
            self.frames_total += self.win.frames.len() as u64;
            self.win.ledger = self.ledger.take_window();
            self.win.cfg = self.ablate.map_or("", |k| ABLATIONS[k].0);
            if let Some(pr) = self.profile.as_mut().filter(|_| self.ledger.windows % crate::profile::PROFILE_WINDOWS == 0) {
                self.profile_done.push((self.frames_total, core::mem::take(&mut **pr)));
            }
            let w = core::mem::take(&mut self.win);
            self.done.push((self.frames_total, w));
        }
    }

    /// The next line of the closed windows, at `per_us` clock units a microsecond: each window gives its `[HEIRLOOM-PERF ...]`
    /// line, its `[HEIRLOOM-CAPTURE ...]` line (in clock units) and the frame ledger's lines (perf_ledger.rs), one per call. The
    /// ONE output path: on CoralOS only what the engine takes here reaches the ring (the machine's log does not).
    pub fn take_line(&mut self, per_us: u64) -> Option<String> {
        if per_us == 0 { return None; }
        if self.pending.is_empty() && !self.done.is_empty() {
            let (n, w) = self.done.remove(0);
            self.pending.push_back(report(&w, per_us, n));
            self.pending.push_back(capture_line(&w, n));
            let (frames, pair, refused) = &w.ledger;
            self.pending.extend(crate::perf_ledger::lines(frames, *pair, *refused, per_us, n).into_iter().map(|l| if w.cfg.is_empty() { l } else { format!("{l} | ablate={}", w.cfg) }));
        }
        self.pending.pop_front()
    }
}

/// Clock units per KiB of `bytes`.
fn per_kib(t: u64, bytes: u64) -> u64 { if bytes == 0 { 0 } else { t.saturating_mul(1024) / bytes } }

/// The `[HEIRLOOM-CAPTURE ...]` line of one window, in CLOCK UNITS (TSC cycles on CoralOS - the Z83's ~1,439 a microsecond; ns on
/// a desktop host): the per-draw capture cost per KiB as percentiles over the draws (each the upper edge of its power-of-two
/// bucket), the share of the capture time spent in draws 8x or more over the median's bucket and the slowest capture; the
/// sampled draws' split per KiB; the read passes per KiB.
pub fn capture_line(w: &Window, frames_total: u64) -> String {
    let draws: u64 = w.capture_hist.iter().map(|&n| u64::from(n)).sum();
    let edge = |q: u64| {
        let want = (draws * q).div_ceil(100).max(1);
        let mut seen = 0u64;
        w.capture_hist.iter().position(|&n| { seen += u64::from(n); seen >= want }).map_or(0, |b| 1u64 << (b + 1))
    };
    let median_b = edge(50).max(2).trailing_zeros() as usize - 1;
    let tail: u64 = w.capture_hist_time.iter().skip(median_b + 3).sum();
    let p = &w.probe;
    format!("[HEIRLOOM-CAPTURE frames={frames_total}] per draw clk/KiB: draws={draws} p50<={} p90<={} p99<={} tail8x={}% max_clk={} \
| sampled 1/{CAPTURE_SAMPLE}: n={} kib={} clk/KiB dst={} cold={} warm={} | read kib={} clk/KiB guest_ram first={} second={} heap first={} second={}",
        edge(50), edge(90), edge(99), pct(tail, w.capture_time), w.capture_max,
        p.draws, p.bytes / 1024, per_kib(p.dst_time, p.dst_bytes), per_kib(p.cold_time, p.bytes), per_kib(p.warm_time, p.bytes),
        p.read_bytes / 1024, per_kib(p.ram_first, p.read_bytes), per_kib(p.ram_second, p.read_bytes), per_kib(p.heap_first, p.read_bytes), per_kib(p.heap_second, p.read_bytes))
}

fn pct(part: u64, whole: u64) -> u64 { if whole == 0 { 0 } else { part.saturating_mul(100) / whole } }

/// The `[HEIRLOOM-PERF ...]` line for one window. Frame times in ms (p50/p95 of swap-to-swap), MIPS over the time inside `step`
/// AND over the wall time, instructions per frame, and the share of the WALL time (sum of the frames) each stage took; then the
/// absolute means that tell the two host-side suspects apart: per frame the GL round trip (`gl_ring_us`, the guest blocked on
/// heirloom-gl.engine) and the time outside `step` (`outside_us`), per slice the time inside it (`step_us`) and between two
/// (`gap_us`, the host's scheduling of the next slice); and the engine's own GL time without the round trip, per frame (`gl_us`)
/// and per GL call (`per gl call: ns`): what the guest's core spends encoding its GL.
pub fn report(w: &Window, per_us: u64, frames_total: u64) -> String {
    let mut f = w.frames.clone();
    f.sort_unstable();
    let n = f.len().max(1);
    let at = |q: usize| f.get((n * q / 100).min(n - 1)).copied().unwrap_or(0);
    let per_ms = per_us.max(1) * 1000;
    let wall: u64 = f.iter().sum();
    let native: u64 = w.native_time.iter().sum();
    let interp = w.step_time.saturating_sub(native);
    let outside = wall.saturating_sub(w.step_time);
    // the engine's OWN GL cost (encoding, validation, snapshots), the worker round trip excluded: the callouts' share of the guest's core
    let gl_own = w.native_time[Class::Gl as usize].saturating_sub(w.gl_ring_time);
    let mips = |t: u64| if t == 0 { 0 } else { w.insns.saturating_mul(per_us) / t };
    let classes: String = (0..CLASSES).map(|i| format!(" {}={}%/{}", CLASS_NAMES[i], pct(w.native_time[i], wall), w.native_calls[i] / n as u64)).collect();
    format!("[HEIRLOOM-PERF frames={frames_total} n={n} frame_ms p50={} p95={} fps={}] insns/frame={} mips_step={} mips_wall={} steps/frame={} \
interp={}% native={}% outside_step={}% gl_ring={}%{classes} clock_reads/frame={} clock_poll={}% of insns \
| per frame: gl_ring_us={} outside_us={} gl_us={} draw_us={} draws={} capture_us={} draw_kib={} | per gl call: ns={} | per slice: step_us={} gap_us={}",
        at(50) / per_ms, at(95) / per_ms, if wall == 0 { 0 } else { (n as u64) * per_ms * 1000 / wall },
        w.insns / n as u64, mips(w.step_time), mips(wall), w.steps / n as u64,
        pct(interp, wall), pct(native, wall), pct(outside, wall), pct(w.gl_ring_time, wall),
        w.clock_reads / n as u64, pct(w.poll_insns, w.insns),
        w.gl_ring_time / per_us.max(1) / n as u64, outside / per_us.max(1) / n as u64, gl_own / per_us.max(1) / n as u64,
        w.draw_time / per_us.max(1) / n as u64, w.draws / n as u64, w.capture_time / per_us.max(1) / n as u64, w.capture_bytes / 1024 / n as u64,
        gl_own.saturating_mul(1000) / per_us.max(1) / w.native_calls[Class::Gl as usize].max(1),
        w.step_time / per_us.max(1) / w.steps.max(1), outside / per_us.max(1) / w.steps.max(1))
}

impl<U: GuestUi> MacPpc<U> {
    /// Turn the stage clock on: `clock` is any monotonic host clock.
    pub fn perf_enable(&mut self, clock: fn() -> u64) {
        self.perf = Some(alloc::boxed::Box::new(Perf {
            clock, win: Window::default(), last_swap: None, last_read: None, class_of: Vec::new(), done: Vec::new(), frames_total: 0, every: REPORT_FRAMES, sites: None, draw_seq: 0,
            ledger: Default::default(), pending: Default::default(), tb: 0, ablate: None, profile: None, profile_done: Vec::new(), lib_said: false, knobs_due: None, tier_line_due: Vec::new(), jit_now: [0; 4],
        }));
    }
    /// Report every `n` frames instead of [`REPORT_FRAMES`] (a host harness's finer view).
    pub fn perf_report_every(&mut self, n: usize) { if let Some(p) = self.perf.as_mut() { p.every = n.max(1); } }
    /// Diagnostic: keep poll instructions by call chain ([`Perf::sites`]).
    pub fn perf_census(&mut self) { if let Some(p) = self.perf.as_mut() { p.sites = Some(Default::default()); } }
    /// The stage clock, when on.
    pub fn perf(&self) -> Option<&Perf> { self.perf.as_deref() }
    /// The next closed window as its `[HEIRLOOM-PERF ...]` line (one per [`REPORT_FRAMES`] frames), with the clock's `per_us`
    /// units per microsecond; None when no window is closed, or `per_us` is 0 (not measured yet: the window waits).
    /// Each closed window gives its `[HEIRLOOM-PERF ...]` line, then its capture and ledger lines ([`Perf::take_line`]).
    pub fn perf_take_line(&mut self, per_us: u64) -> Option<String> {
        if per_us == 0 { return None; }
        let p = self.perf.as_mut()?;
        // the map's lines first (one per section, at instantiation), then the windows, then a closed profile's lines
        if !self.code_map.lines.is_empty() { return Some(self.code_map.lines.remove(0)); }
        if let Some(l) = p.take_line(per_us) { return Some(l); }
        if p.profile_done.is_empty() { return None; }
        let (n, pr) = p.profile_done.remove(0);
        let glue = &self.glue;
        let name_of = |id: usize| glue.key_of(id).map_or_else(|| format!("glue#{id}"), |k| String::from(&*k));
        p.pending.extend(crate::profile::lines(&pr, &self.code_map, &name_of, per_us, n));
        p.pending.pop_front()
    }
    /// Turn the guest-PC profile on (cpu-tier "jitprofile"): the stage clock goes on; the profile prints every
    /// [`crate::profile::PROFILE_WINDOWS`] perf windows.
    pub fn perf_profile(&mut self, clock: fn() -> u64) {
        if self.perf.is_none() { self.perf_enable(clock); }
        if let Some(p) = self.perf.as_mut() { p.profile = Some(Default::default()); }
    }
    /// The engine's half of the frame ledger ([`heirloom_os_abi::perf::PerfHost`]): turns the stage clock on (on `clock`).
    pub fn perf_attach(&mut self, clock: fn() -> u64, host: heirloom_os_abi::perf::PerfHost) {
        if self.perf.is_none() { self.perf_enable(clock); }
        if let Some(p) = self.perf.as_mut() { p.ledger.attach(host); }
    }

    /// cpu-tier "jitablate": after a swap that closed a perf window, the next [`ABLATIONS`] setting every [`ABLATE_WINDOWS`].
    pub(crate) fn ablate_after_swap(&mut self, windows_before: u64) {
        let Some(p) = self.perf.as_ref() else { return };
        let Some(k) = p.ablate else { return };
        let w = p.ledger.windows;
        if w == windows_before { return; }
        // the window that just closed ran under ABLATIONS[k]: what the CPU itself says it ran under (h86 fnreg - the A/B proves
        // itself or names why not), then the next setting every ABLATE_WINDOWS
        // a swap is inside a native, inside the run: the run's Exec is moved out of self.rt meanwhile (run.rs), so the window's
        // line and the next knobs wait for the run's end (after_run) - h86 fnreg: set here, they went to the placeholder Exec and
        // were lost, which is why no jitablate window ever applied its knobs
        let Some(p) = self.perf.as_mut() else { return };
        p.tier_line_due.push((k, w));
        if w % ABLATE_WINDOWS != 0 { return; }
        let next = (k + 1) % ABLATIONS.len();
        p.ablate = Some(next);
        p.knobs_due = Some(ABLATIONS[next].1);
    }
    /// After every run, with the run's Exec back in self.rt: the knobs a swap chose, the windows' CPU-PPC-TIER lines, the
    /// CPU-PPC-LIB line once, and tier 2's event counts for the ledger's next frames.
    pub(crate) fn after_run(&mut self) {
        if self.perf.is_none() { return; }
        let jit = self.jit_events_live();
        let Some(p) = self.perf.as_mut() else { return };
        p.jit_now = jit;
        // the closed windows' lines first (what ran in them), then the next setting
        let due = core::mem::take(&mut p.tier_line_due);
        for (k, w) in due {
            let lines = self.cpu_tier_lines(k, w);
            if let Some(p) = self.perf.as_mut() { p.pending.extend(lines); }
        }
        if let Some(k) = self.perf.as_mut().and_then(|p| p.knobs_due.take()) { self.rt.exec.set_jit_knobs(k); }
        self.cpu_lib_once();
    }
    /// `[CPU-PPC-TIER window=<n> label=<setting> asked=<knobs> knobs=<in the CPU> applied=0|1 ...]` for the window that closed
    /// under ABLATIONS[k] - the knobs, compiles, regions, translations and code drops as the CPU (cpu-ppc.library's read_prof, or
    /// the static core) reports them, and a refused knob word by name; once, first, `[CPU-PPC-LIB build=<hash> prof=<bytes>]`.
    fn cpu_tier_lines(&mut self, k: usize, w: u64) -> Vec<String> {
        let (label, asked) = ABLATIONS[k];
        let mut out = Vec::new();
        #[cfg(feature = "cpu-library")]
        let (knobs, tier, compiles, regions, xlate, drops, refusals, refused, build, size) = {
            let t = &self.rt.exec.tiers;
            let refused = self.rt.exec.knob_refused().map(|w| w as u64).or((t.tier_refusals != 0).then_some(t.tier_refused_word));
            (t.knobs as u32, t.tier, t.jit_compiled, t.jit_regions, t.jit_translated, t.jit_reconfigures, t.tier_refusals, refused, t.build, t.size)
        };
        #[cfg(not(feature = "cpu-library"))]
        let (knobs, tier, compiles, regions, xlate, drops, refusals, refused, build, size) = {
            let j = self.rt.exec.jit_stats();
            (self.rt.exec.jit_knobs(), self.rt.exec.tier.number(), j.compiled, j.regions, j.translated, j.reconfigures, 0u64, None::<u64>, heirloom_cpu_ppc::BUILD, 0u32)
        };
        let _ = (build, size);
        let why = match refused { Some(w) => format!(" applied=0 why=refused-word-{w:#x}"), None if knobs != asked => " applied=0 why=knobs-differ".into(), None => " applied=1".into() };
        out.push(format!("[CPU-PPC-TIER window={w} label={label} asked={asked:#x} knobs={knobs:#x}{why} tier={tier} compiles={compiles} regions={regions} xlate={xlate} reconfigures={drops} refusals={refusals}]"));
        out
    }
    /// Once, at the first swap after the CPU reported: `[CPU-PPC-LIB build=<hash> prof=<bytes>]` - which core this engine runs
    /// (cpu-ppc.library's PpcProf::build: a hash of heirloom-cpu-ppc's sources; 0 = a library older than the field).
    pub(crate) fn cpu_lib_once(&mut self) {
        #[cfg(feature = "cpu-library")]
        let (build, size) = (self.rt.exec.tiers.build, self.rt.exec.tiers.size);
        #[cfg(not(feature = "cpu-library"))]
        let (build, size) = (heirloom_cpu_ppc::BUILD, 0u32);
        let Some(p) = self.perf.as_mut().filter(|p| !p.lib_said) else { return };
        if cfg!(feature = "cpu-library") && size == 0 { return; } // no read_prof yet
        p.lib_said = true;
        p.pending.push_back(format!("[CPU-PPC-LIB build={build:#018x} prof={size}]{}", if build == 0 { " - an OLDER cpu-ppc.library: it reports no build id (it predates h86 fnreg-3)" } else { "" }));
    }
    /// Turn the rotation on (cpu-tier "jitablate"): the stage clock goes on, the first setting runs now.
    pub fn perf_ablate(&mut self, clock: fn() -> u64) {
        if self.perf.is_none() { self.perf_enable(clock); }
        if let Some(p) = self.perf.as_mut() { p.ablate = Some(0); }
        self.rt.exec.set_jit_knobs(ABLATIONS[0].1);
    }

    /// Tier 2's cumulative events for the frame ledger: blocks compiled, publishes, withdraws (arena reuse), bails - from the CPU
    /// library's last `read_prof` (cpu-library) or the static core's own counters.
    pub(crate) fn jit_events(&self) -> [u64; 4] {
        // at a swap (inside the run: the Exec is moved out) the counts as of the last run's end (after_run)
        self.perf.as_ref().map_or([0; 4], |p| p.jit_now)
    }
    fn jit_events_live(&self) -> [u64; 4] {
        #[cfg(feature = "cpu-library")]
        { let t = &self.rt.exec.tiers; [t.jit_compiled, t.publishes, t.withdraws, t.jit_bails] }
        #[cfg(not(feature = "cpu-library"))]
        { let t = self.rt.exec.jit_stats(); [t.compiled, t.publishes, t.withdraws, t.bails] }
    }

    /// The class of the glue call whose stub is at `addr` (cached by its glue id).
    pub(crate) fn perf_class(&mut self, addr: u32) -> Class { self.perf_class_draw(addr).0 }

    /// [`MacPpc::perf_class`], and whether the call is a GL draw (cached with it: bit 7 of the class byte).
    pub(crate) fn perf_class_draw(&mut self, addr: u32) -> (Class, bool) {
        let Some(id) = self.glue.id_at_stub(addr) else { return (Class::Other, false) };
        if let Some(c) = self.perf.as_ref().and_then(|p| p.class_of.get(id).copied()).filter(|&c| c != u8::MAX) { return (from_u8(c & 0x7F), c & 0x80 != 0); }
        let symbol = self.glue.entry(id).map(|e| e.key()).unwrap_or_default();
        let b = self.bindings.get(&symbol);
        let (c, draw) = (classify(b, &symbol), is_draw(b));
        if let Some(p) = self.perf.as_mut() {
            if p.class_of.len() <= id { p.class_of.resize(id + 1, u8::MAX); }
            p.class_of[id] = c as u8 | if draw { 0x80 } else { 0 };
        }
        (c, draw)
    }
}

fn from_u8(c: u8) -> Class {
    match c { 0 => Class::Gl, 1 => Class::Clock, 2 => Class::Thread, 3 => Class::Event, 4 => Class::File, _ => Class::Other }
}

/// Whether a binding is a GL draw (its client arrays are snapshotted into the batch).
pub fn is_draw(b: Option<&crate::symbols::Binding>) -> bool {
    use crate::symbols::{Binding, Native};
    matches!(b, Some(Binding::Native(Native::Gl(f))) if matches!(heirloom_gl_abi::ENTRIES.get(usize::from(f.0)).map(|e| e.name), Some("glDrawElements" | "glDrawArrays" | "glDrawRangeElements")))
}

/// A call's class from its binding (the clocks and the event calls by name: they are bound several ways).
pub fn classify(b: Option<&crate::symbols::Binding>, symbol: &str) -> Class {
    use crate::symbols::{Binding, Native};
    if is_clock(symbol) { return Class::Clock; }
    match b {
        Some(Binding::Native(Native::Gl(_))) => Class::Gl,
        Some(Binding::Native(Native::Thread(_) | Native::Mp(_))) => Class::Thread,
        Some(Binding::Native(Native::Carbon(_) | Native::WaitNextEvent | Native::GetNextEvent)) => Class::Event,
        Some(Binding::Native(Native::File(_) | Native::Pb(_) | Native::Stdio(_))) => Class::File,
        _ => Class::Other,
    }
}

/// The Mac clocks a program polls.
pub fn is_clock(symbol: &str) -> bool { matches!(symbol, "Microseconds" | "UpTime" | "TickCount" | "GetCurrentEventTime") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_on_the_same_tick_close_together_are_a_poll_and_a_far_or_new_tick_read_is_not() {
        fn zero() -> u64 { 0 }
        let mut p = Perf { clock: zero, win: Window::default(), last_swap: None, last_read: None, class_of: Vec::new(), done: Vec::new(), frames_total: 0, every: REPORT_FRAMES, sites: None, draw_seq: 0, ledger: Default::default(), pending: Default::default(), tb: 0, ablate: None, profile: None, profile_done: Vec::new(), lib_said: false, knobs_due: None, tier_line_due: Vec::new(), jit_now: [0; 4] };
        p.clock_read(5, 100, [0; 3]);
        p.clock_read(5, 400, [0; 3]); // 300 insns later, same tick: a poll
        p.clock_read(6, 500, [0; 3]); // the tick moved: not a poll
        p.clock_read(6, 500 + POLL_GAP_INSNS + 1, [0; 3]); // same tick but far apart: work that read the clock twice
        assert_eq!(p.win.poll_insns, 300);
        assert_eq!(p.win.clock_reads, 4);
    }

    #[test]
    fn a_window_reports_its_frame_percentiles_and_shares_of_the_wall_time() {
        let mut w = Window::default();
        w.frames = (1..=100).map(|i| i * 1000).collect(); // 1..100 ms at 1000 units/ms (per_us = 1)
        w.insns = 100 * 2_000_000;
        w.step_time = 2_525_000; // half the wall time (sum = 5_050_000)
        w.native_time[Class::Gl as usize] = 505_000; // 10%
        w.steps = 1000;
        let line = report(&w, 1, 100);
        assert!(line.starts_with("[HEIRLOOM-PERF frames=100 n=100 frame_ms p50=51 p95=96 fps=19]"), "{line}");
        assert!(line.contains("insns/frame=2000000 mips_step=79 mips_wall=39 steps/frame=10"), "{line}");
        assert!(line.contains("interp=40% native=10% outside_step=50%"), "{line}");
        assert!(line.contains(" gl=10%/0"), "{line}");
        // the absolute times that tell a slow GL round trip from slow slicing on the Z83: 2_525_000 units outside step over 100
        // frames and 1000 slices; nothing in gl_ring
        assert!(line.ends_with("| per frame: gl_ring_us=0 outside_us=25250 gl_us=5050 draw_us=0 draws=0 capture_us=0 draw_kib=0 | per gl call: ns=505000000 | per slice: step_us=2525 gap_us=2525"), "{line}");
    }

    #[test]
    fn the_engines_own_gl_time_excludes_the_worker_round_trip_per_frame_and_per_call() {
        let mut w = Window::default();
        w.frames = vec![10_000; 4]; // 4 frames of 10 ms at per_us = 1
        w.steps = 4;
        w.step_time = 40_000;
        w.native_time[Class::Gl as usize] = 9_000; // 9 ms of GL glue in all, 1 ms of it waiting on the worker
        w.native_calls[Class::Gl as usize] = 4_000;
        w.gl_ring_time = 1_000;
        w.draw_time = 4_000; // 1 ms of draws a frame, 200 draws a frame
        w.draws = 800;
        (w.capture_time, w.capture_bytes) = (2_000, 4 * 470 * 1024); // half the draws' time capturing, 470 KiB a frame
        let line = report(&w, 1, 4);
        assert!(line.contains("| per frame: gl_ring_us=250 outside_us=0 gl_us=2000 draw_us=1000 draws=200 capture_us=500 draw_kib=470 | per gl call: ns=2000 |"), "{line}");
    }

    /// The capture line: percentiles are the upper edges of their power-of-two buckets, the tail is the capture time in draws 8x
    /// or more over the median's bucket, the sampled split and the read passes are per KiB; a window is closed with its line.
    #[test]
    fn the_capture_line_shows_the_draws_shape_the_sampled_split_and_the_read_passes() {
        fn zero() -> u64 { 0 }
        let mut p = Perf { clock: zero, win: Window::default(), last_swap: None, last_read: None, class_of: Vec::new(), done: Vec::new(), frames_total: 0, every: 2, sites: None, draw_seq: 0, ledger: Default::default(), pending: Default::default(), tb: 0, ablate: None, profile: None, profile_done: Vec::new(), lib_said: false, knobs_due: None, tier_line_due: Vec::new(), jit_now: [0; 4] };
        for _ in 0..98 { p.add_capture(2000, 2048); } // 1000 a KiB: bucket 9 = [512, 1024), the edge 1024
        p.add_capture(100_000, 1024); // two stalls: 100,000 a KiB, bucket 16 [65536, 131072)
        p.add_capture(50_000, 512);
        assert_eq!((p.win.capture_hist[9], p.win.capture_hist[16]), (98, 2));
        assert_eq!((0..CAPTURE_SAMPLE * 2).filter(|_| p.capture_sample()).count(), 2, "one draw in CAPTURE_SAMPLE is sampled");
        p.win.probe = CaptureProbe { draws: 2, bytes: 2048, dst_time: 300, dst_bytes: 32768, cold_time: 9000, warm_time: 1000, read_bytes: 262_144,
            ram_first: 256_000, ram_second: 25_600, heap_first: 128_000, heap_second: 25_600 };
        let line = capture_line(&p.win, 256);
        assert!(line.starts_with("[HEIRLOOM-CAPTURE frames=256] per draw clk/KiB: draws=100 p50<=1024 p90<=1024 p99<=131072 tail8x=43% max_clk=100000 "), "{line}");
        assert!(line.ends_with("| sampled 1/32: n=2 kib=2 clk/KiB dst=9 cold=4500 warm=500 | read kib=256 clk/KiB guest_ram first=1000 second=100 heap first=500 second=100"), "{line}");
        // the window closes at the swap with its line, taken once
        p.swap([0; 4]);
        p.swap([0; 4]);
        p.swap([0; 4]);
        let lines: Vec<String> = core::iter::from_fn(|| p.take_line(1)).collect();
        assert!(lines[0].starts_with("[HEIRLOOM-PERF frames=2 "), "{lines:?}");
        assert!(lines[1].starts_with("[HEIRLOOM-CAPTURE frames=2] per draw clk/KiB: draws=100 "), "the capture line takes the one output path: {lines:?}");
        assert!(lines.iter().filter(|l| l.starts_with("[HEIRLOOM-CAPTURE")).count() == 1, "{lines:?}");
        assert!(p.take_line(1).is_none());
        assert_eq!(p.win.probe, CaptureProbe::default(), "the next window starts empty");
    }
}
