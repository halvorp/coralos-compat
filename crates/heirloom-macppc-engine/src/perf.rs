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
use crate::perf_hist::{FrameRec, RECENT};
use crate::perf_pc::{PcProf, Weight};
use alloc::collections::{BTreeMap, VecDeque};
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
    /// Tier 2's translation time (compiling, publishing, flushing: `JitStats::translate_time`), a part of `step`; 0 on tiers 0/1.
    pub translate_time: u64,
    pub clock_reads: u64,
    pub poll_insns: u64,
    /// Swap-to-swap durations of the frames in this window.
    pub frames: Vec<u64>,
    /// The same frames with their stage split ([`crate::perf_hist`]).
    pub recs: Vec<FrameRec>,
    /// Guest-PC samples of this window ([`crate::perf_pc`]): empty unless sampling is on.
    pub pcs: BTreeMap<u32, Weight>,
}

/// The cumulative counters at the last swap, to difference into a frame's stage split.
#[derive(Debug, Clone, Copy, Default)]
struct Mark { step: u64, native: u64, gl: u64, ring: u64 }

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
    /// Where the current `step` began (None between slices): a swap inside a slice charges the slice only up to the swap.
    step_t0: Option<u64>,
    /// The part of the slice in flight already charged to a closed window.
    step_carry: u64,
    /// `JitStats::translate_time` as of the last slice (it is cumulative).
    translate_seen: u64,
    mark: Mark,
    /// The last [`RECENT`] frames, for an on-demand dump.
    pub recent: VecDeque<FrameRec>,
    /// Lines a window's report adds after its aggregate line (`HIST`, `PC`), handed out by `perf_take_line` first.
    extra: VecDeque<String>,
    /// The guest-PC sampler, when on ([`crate::perf_pc`]).
    pub pc: Option<alloc::boxed::Box<PcProf>>,
}

impl Perf {
    pub fn new(clock: fn() -> u64) -> Perf {
        Perf { clock, win: Window::default(), last_swap: None, last_read: None, class_of: Vec::new(), done: Vec::new(), frames_total: 0, every: REPORT_FRAMES, sites: None,
            step_t0: None, step_carry: 0, translate_seen: 0, mark: Mark::default(), recent: VecDeque::new(), extra: VecDeque::new(), pc: None }
    }
    pub fn now(&self) -> u64 { (self.clock)() }
    /// The host clock itself (handed to the JIT to time its translation).
    pub fn clock_fn(&self) -> fn() -> u64 { self.clock }
    /// A slice begins at `t0` on the host clock.
    pub(crate) fn step_begin(&mut self, t0: u64) { self.step_t0 = Some(t0); }
    pub(crate) fn add_native(&mut self, class: Class, dt: u64) {
        self.win.native_time[class as usize] += dt;
        self.win.native_calls[class as usize] += 1;
    }
    pub(crate) fn add_step(&mut self, dt: u64, insns: u64) {
        self.step_t0 = None;
        self.win.step_time += dt.saturating_sub(core::mem::take(&mut self.step_carry));
        self.win.steps += 1;
        self.win.insns += insns;
    }
    pub(crate) fn add_gl_ring(&mut self, dt: u64) { self.win.gl_ring_time += dt; }
    /// The JIT's cumulative translation time after a slice: its growth is this window's.
    pub(crate) fn add_translate(&mut self, total: u64) {
        self.win.translate_time += total.saturating_sub(self.translate_seen);
        self.translate_seen = total;
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
    pub(crate) fn swap(&mut self) {
        let now = self.now();
        // the cumulative counters as of this swap: the slice it happens inside is charged up to here (add_step adds the rest later)
        let inflight = self.step_t0.map_or(0, |t| now.wrapping_sub(t));
        let cum = Mark { step: self.win.step_time + inflight, native: self.win.native_time.iter().sum(), gl: self.win.native_time[Class::Gl as usize], ring: self.win.gl_ring_time };
        if let Some(prev) = self.last_swap {
            let dur = now.wrapping_sub(prev);
            self.win.frames.push(dur);
            let m = self.mark;
            let rec = FrameRec { dur, step: cum.step.saturating_sub(m.step), native: cum.native.saturating_sub(m.native), gl: cum.gl.saturating_sub(m.gl), ring: cum.ring.saturating_sub(m.ring) };
            self.win.recs.push(rec);
            if self.recent.len() >= RECENT { self.recent.pop_front(); }
            self.recent.push_back(rec);
        }
        self.mark = cum;
        self.last_swap = Some(now);
        if self.win.frames.len() >= self.every.max(1) {
            self.frames_total += self.win.frames.len() as u64;
            let mut w = core::mem::take(&mut self.win);
            // the next window's counters restart at zero; the slice in flight is charged to this window up to now and to the next
            // one from now on (`step_carry` is what add_step must not charge twice)
            self.mark = Mark::default();
            if self.step_t0.is_some() { w.step_time += inflight; self.step_carry += inflight; self.step_t0 = Some(now); }
            self.done.push((self.frames_total, w));
        }
    }
}

fn pct(part: u64, whole: u64) -> u64 { if whole == 0 { 0 } else { part.saturating_mul(100) / whole } }

/// The `[HEIRLOOM-PERF ...]` line for one window. Frame times in ms (p50/p95 of swap-to-swap, and the MEAN: wall / frames), MIPS over
/// the time inside `step` (`mips_step`: the slices, glue included), over the interpreter's own time (`mips_interp` = `step` minus
/// the glue calls: the speed of the CPU core itself) and over the wall time, instructions per frame, and the share of the WALL time
/// (sum of the frames) each stage took; then the absolute means that tell the two host-side suspects apart: per frame the GL round
/// trip (`gl_ring_us`, the guest blocked on heirloom-gl.engine) and the time outside `step` (`outside_us`), per slice the time inside
/// it (`step_us`) and between two (`gap_us`, the host's scheduling of the next slice).
///
/// Reconciling the three clocks (h86 perf0): `insns/frame / mips_step` is the mean STEP time per frame, which is `interp + native`
/// of the mean frame, so it is the mean frame times `(interp% + native%)`; the p50 frame is shorter than the mean (a right-skewed
/// window), and `interp%` is of the WALL time and excludes the glue, so "4.3M insns at mips_step 20 = 215 ms" and "interp 80% of a
/// 210-215 ms (p50) frame" do not contradict: with a 225 ms mean frame, interp 80% = 180 ms (mips_interp 24) and native 15% = 34 ms
/// make 214 ms of step.
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
    let mips = |t: u64| if t == 0 { 0 } else { w.insns.saturating_mul(per_us) / t };
    let classes: String = (0..CLASSES).map(|i| format!(" {}={}%/{}", CLASS_NAMES[i], pct(w.native_time[i], wall), w.native_calls[i] / n as u64)).collect();
    let mean_x10 = wall.saturating_mul(10) / (n as u64 * per_ms);
    format!("[HEIRLOOM-PERF frames={frames_total} n={n} frame_ms p50={} p95={} mean={}.{} fps={}] insns/frame={} mips_step={} mips_interp={} mips_wall={} steps/frame={} \
interp={}% native={}% outside_step={}% gl_ring={}%{classes} clock_reads/frame={} clock_poll={}% of insns \
| per frame: gl_ring_us={} outside_us={} | per slice: step_us={} gap_us={}",
        at(50) / per_ms, at(95) / per_ms, mean_x10 / 10, mean_x10 % 10, if wall == 0 { 0 } else { (n as u64) * per_ms * 1000 / wall },
        w.insns / n as u64, mips(w.step_time), mips(interp), mips(wall), w.steps / n as u64,
        pct(interp, wall), pct(native, wall), pct(outside, wall), pct(w.gl_ring_time, wall),
        w.clock_reads / n as u64, pct(w.poll_insns, w.insns),
        w.gl_ring_time / per_us.max(1) / n as u64, outside / per_us.max(1) / n as u64,
        w.step_time / per_us.max(1) / w.steps.max(1), outside / per_us.max(1) / w.steps.max(1))
        + &if w.translate_time == 0 { String::new() } else { format!(" | jit: translate={}% translate_us/frame={}", pct(w.translate_time, wall), w.translate_time / per_us.max(1) / n as u64) }
}

impl<U: GuestUi> MacPpc<U> {
    /// Turn the stage clock on: `clock` is any monotonic host clock.
    pub fn perf_enable(&mut self, clock: fn() -> u64) {
        self.perf = Some(alloc::boxed::Box::new(Perf::new(clock)));
    }
    /// Turn the guest-PC sampler on (the stage clock must be on): every [`crate::perf_pc::EVERY`] instructions the slice is cut and the
    /// PC recorded ([`crate::perf_pc`]); each report window then also queues a `[HEIRLOOM-PERF-PC ...]` line.
    pub fn perf_pc_enable(&mut self, every: u64) {
        if let Some(p) = self.perf.as_mut() { p.pc = Some(alloc::boxed::Box::new(PcProf::new(every))); }
    }
    /// Forget the samples so far (the host harness: profile only the gameplay, not the menus and the loading before it).
    pub fn perf_pc_reset(&mut self) {
        if let Some(p) = self.perf.as_mut() {
            p.win.pcs.clear();
            if let Some(pc) = p.pc.as_mut() { pc.cum.clear(); pc.callers.clear(); pc.dropped = 0; }
        }
    }
    /// The cumulative sample table as text for an offline join with a disassembler's function list: `S <pc> <insns> <time units>` per
    /// sampled PC, `C <class> <caller> <calls>` per glue-call caller of the clock / thread classes, `R <lo> <hi>` the code range.
    pub fn perf_pc_dump(&self) -> String {
        let Some(pc) = self.perf.as_ref().and_then(|p| p.pc.as_deref()) else { return String::new() };
        let mut o = format!("# every={} pcs={} dropped={}\n", pc.every, pc.cum.len(), pc.dropped);
        if let Some((lo, hi)) = self.code_range { o.push_str(&format!("R {lo:#x} {hi:#x}\n")); }
        for (a, w) in &pc.cum { o.push_str(&format!("S {a:#x} {} {}\n", w.insns, w.time)); }
        for ((c, lr), n) in &pc.callers { o.push_str(&format!("C {c} {lr:#x} {n}\n")); }
        o
    }
    /// The guest's code section as the sampler sees it: its range and the bytes (big-endian words), for an offline disassembly.
    pub fn perf_pc_code(&self) -> Option<(u32, Vec<u8>)> {
        let (lo, hi) = self.code_range?;
        let bytes = (lo..hi).step_by(4).flat_map(|a| self.mac.mem.read_u32(a).unwrap_or(0).to_be_bytes()).collect();
        Some((lo, bytes))
    }
    /// The last frames (up to [`RECENT`]) as one `[HEIRLOOM-PERF-FRAME ...]` line each, oldest first: the on-demand dump.
    pub fn perf_frames(&self, per_us: u64) -> Vec<String> {
        let Some(p) = self.perf.as_ref() else { return Vec::new() };
        crate::perf_hist::frame_lines(&p.recent.iter().copied().collect::<Vec<_>>(), per_us)
    }
    /// Report every `n` frames instead of [`REPORT_FRAMES`] (a host harness's finer view).
    pub fn perf_report_every(&mut self, n: usize) { if let Some(p) = self.perf.as_mut() { p.every = n.max(1); } }
    /// Diagnostic: keep poll instructions by call chain ([`Perf::sites`]).
    pub fn perf_census(&mut self) { if let Some(p) = self.perf.as_mut() { p.sites = Some(Default::default()); } }
    /// The stage clock, when on.
    pub fn perf(&self) -> Option<&Perf> { self.perf.as_deref() }
    /// The next closed window as its `[HEIRLOOM-PERF ...]` line (one per [`REPORT_FRAMES`] frames), with the clock's `per_us`
    /// units per microsecond; None when no window is closed, or `per_us` is 0 (not measured yet: the window waits).
    pub fn perf_take_line(&mut self, per_us: u64) -> Option<String> {
        let p = self.perf.as_mut()?;
        if per_us == 0 { return None; }
        if let Some(l) = p.extra.pop_front() { return Some(l); }
        if p.done.is_empty() { return None; }
        let (n, w) = p.done.remove(0);
        let main = report(&w, per_us, n);
        p.extra.push_back(crate::perf_hist::hist_line(&w.recs, per_us, n));
        if p.pc.is_some() { let l = self.perf_pc_report(&w, n); if let Some(p) = self.perf.as_mut() { p.extra.push_back(l); } }
        Some(main)
    }

    /// The window's `[HEIRLOOM-PERF-PC ...]` line; builds the function table on first use (a scan of the code section).
    fn perf_pc_report(&mut self, w: &Window, frames_total: u64) -> String {
        self.perf_pc_starts();
        let Some(pc) = self.perf.as_ref().and_then(|p| p.pc.as_deref()) else { return String::new() };
        crate::perf_pc::pc_line(&pc.starts, self.code_range, &w.pcs, &pc.callers, frames_total, pc.every)
    }

    /// The function entries of the code range ([`PcProf::code`], else the extent of the PCs sampled so far), scanned once.
    pub fn perf_pc_starts(&mut self) {
        let Some(pc) = self.perf.as_ref().and_then(|p| p.pc.as_deref()) else { return };
        if !pc.starts.is_empty() { return; }
        let range = self.code_range.or_else(|| {
            let lo = pc.cum.keys().next().copied()?;
            let hi = pc.cum.keys().next_back().copied()?;
            Some((lo & !0xFFFF, hi.saturating_add(0x10000)))
        });
        let Some((lo, hi)) = range else { return };
        let mem = &self.mac.mem;
        let words = (lo..hi).step_by(4).filter_map(|a| mem.read_u32(a).ok().map(|w| (a, w)));
        let starts = crate::perf_pc::function_starts(words, lo, hi);
        if let Some(pc) = self.perf.as_mut().and_then(|p| p.pc.as_deref_mut()) { pc.starts = starts; }
    }

    /// The class of the glue call whose stub is at `addr` (cached by its glue id).
    pub(crate) fn perf_class(&mut self, addr: u32) -> Class {
        let Some(id) = self.glue.id_at_stub(addr) else { return Class::Other };
        if let Some(c) = self.perf.as_ref().and_then(|p| p.class_of.get(id).copied()).filter(|&c| c != u8::MAX) { return from_u8(c); }
        let symbol = self.glue.entry(id).map(|e| e.key()).unwrap_or_default();
        let c = classify(self.bindings.get(&symbol), &symbol);
        if let Some(p) = self.perf.as_mut() {
            if p.class_of.len() <= id { p.class_of.resize(id + 1, u8::MAX); }
            p.class_of[id] = c as u8;
        }
        c
    }
}

fn from_u8(c: u8) -> Class {
    match c { 0 => Class::Gl, 1 => Class::Clock, 2 => Class::Thread, 3 => Class::Event, 4 => Class::File, _ => Class::Other }
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
        let mut p = Perf::new(zero);
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
        assert!(line.starts_with("[HEIRLOOM-PERF frames=100 n=100 frame_ms p50=51 p95=96 mean=50.5 fps=19]"), "{line}");
        assert!(line.contains("insns/frame=2000000 mips_step=79 mips_interp=99 mips_wall=39 steps/frame=10"), "{line}");
        assert!(line.contains("interp=40% native=10% outside_step=50%"), "{line}");
        assert!(line.contains(" gl=10%/0"), "{line}");
        // the absolute times that tell a slow GL round trip from slow slicing on the Z83: 2_525_000 units outside step over 100
        // frames and 1000 slices; nothing in gl_ring
        assert!(line.ends_with("| per frame: gl_ring_us=0 outside_us=25250 | per slice: step_us=2525 gap_us=2525"), "{line}");
        // tier 2's translation time, when there is any, is a part of the step and is said beside the split
        w.translate_time = 505_000; // 10% of the wall time
        assert!(report(&w, 1, 100).ends_with(" | jit: translate=10% translate_us/frame=5050"));
    }

    /// The Z83's STEP B numbers (4.3M instructions a frame, mips_step 20, interp 80% of the frame, p50 210 ms, p95 260 ms) are ONE
    /// consistent window, not an arithmetic error: the mean frame is longer than the p50 (right-skewed), the step time is
    /// interp + native, and the interpreter's own speed (mips_interp) is higher than mips_step because the glue is not interpretation.
    #[test]
    fn the_z83_step_b_window_is_consistent_and_says_the_interpreters_own_speed() {
        // 100 frames at 1000 units/ms (per_us = 1): 80 frames of 210 ms and 20 of 260 ms (p50 210, p95 260, mean 220 ms)
        let mut w = Window::default();
        w.frames = (0..100).map(|i| if i < 80 { 210_000 } else { 260_000 }).collect();
        let wall: u64 = w.frames.iter().sum(); // 22,000,000 = 220 ms a frame
        w.insns = 100 * 4_300_000;
        w.steps = 100 * 23;
        w.native_time[Class::Gl as usize] = wall * 11 / 100;
        w.native_time[Class::Other as usize] = wall * 4 / 100; // native 15% of the wall
        w.step_time = wall * 95 / 100; // interp 80% + native 15%; outside 5%
        let line = report(&w, 1, 100);
        assert!(line.contains("p50=210 p95=260 mean=220.0"), "{line}");
        // 4.3M / (0.95 * 220 ms = 209 ms) = 20 MIPS over the step; over the interpreter's own 176 ms it is 24
        assert!(line.contains("insns/frame=4300000 mips_step=20 mips_interp=24 mips_wall=19"), "{line}");
        assert!(line.contains("interp=80% native=15% outside_step=5%"), "{line}");
        // the identity the line obeys: insns/frame / mips_step ~ mean frame * (interp% + native%)
        let step_ms = 4_300_000i64 / 20 / 1000;
        assert!((step_ms - 220 * 95 / 100).abs() < 10, "{step_ms}");
    }

    /// A swap inside a slice charges the slice up to the swap, and the rest of that slice to the next frame (nothing twice).
    #[test]
    fn a_frame_is_charged_only_for_the_part_of_a_slice_that_ran_during_it() {
        use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
        static NOW: AtomicU64 = AtomicU64::new(0);
        fn clock() -> u64 { NOW.load(Relaxed) }
        let mut p = Perf::new(clock);
        p.every = 1000;
        // frame 0 ends at t=100 (the first swap only starts the clock); one slice runs 100..400 with a swap at 250 and its end at 400
        NOW.store(100, Relaxed);
        p.swap();
        p.step_begin(100);
        NOW.store(250, Relaxed);
        p.swap(); // frame 1: 150 units, all of it inside the slice
        NOW.store(400, Relaxed);
        p.add_step(300, 1);
        NOW.store(430, Relaxed);
        p.swap(); // frame 2: 180 units, 150 of the slice's remainder (250..400) and 30 outside
        let r = &p.win.recs;
        assert_eq!((r[0].dur, r[0].step), (150, 150));
        assert_eq!((r[1].dur, r[1].step), (180, 150), "{r:?}");
        assert_eq!(r[0].step + r[1].step, 300, "the slice's 300 units are charged once");
        assert_eq!(p.recent.len(), 2);
    }
}
