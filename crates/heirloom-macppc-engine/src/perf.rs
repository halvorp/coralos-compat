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
    pub clock_reads: u64,
    pub poll_insns: u64,
    /// Swap-to-swap durations of the frames in this window.
    pub frames: Vec<u64>,
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
}

impl Perf {
    pub fn now(&self) -> u64 { (self.clock)() }
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
    pub(crate) fn add_capture(&mut self, dt: u64, bytes: u64) { (self.win.capture_time, self.win.capture_bytes) = (self.win.capture_time + dt, self.win.capture_bytes + bytes); }

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
        if let Some(prev) = self.last_swap { self.win.frames.push(now.wrapping_sub(prev)); }
        self.last_swap = Some(now);
        if self.win.frames.len() >= self.every.max(1) {
            self.frames_total += self.win.frames.len() as u64;
            let w = core::mem::take(&mut self.win);
            self.done.push((self.frames_total, w));
        }
    }
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
            clock, win: Window::default(), last_swap: None, last_read: None, class_of: Vec::new(), done: Vec::new(), frames_total: 0, every: REPORT_FRAMES, sites: None,
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
    pub fn perf_take_line(&mut self, per_us: u64) -> Option<String> {
        let p = self.perf.as_mut()?;
        if p.done.is_empty() || per_us == 0 { return None; }
        let (n, w) = p.done.remove(0);
        Some(report(&w, per_us, n))
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
        let mut p = Perf { clock: zero, win: Window::default(), last_swap: None, last_read: None, class_of: Vec::new(), done: Vec::new(), frames_total: 0, every: REPORT_FRAMES, sites: None };
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
}
