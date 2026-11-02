//! Per-frame frame-time capture (h86 perf0, WC3 30 fps step 0): the stage clock's 256-frame AGGREGATE cannot say which frames are
//! the p95 outliers or why, so every frame (swap to swap) is also kept as a [`FrameRec`] with its stage split, in
//!
//! * the window the aggregate line covers ([`crate::perf::Window::recs`]), summarised once per window as one rate-limited
//!   `[HEIRLOOM-PERF-HIST ...]` line (percentiles, a bucket histogram, and the TAIL - the frames at or above p95 - against the
//!   median frames, stage by stage, so an outlier is attributed to the interpreter, the glue, the GL ring or the host's own
//!   scheduling instead of guessed at);
//! * a bounded ring of the last [`RECENT`] frames ([`crate::perf::Perf::recent`]), dumped on demand with
//!   [`crate::machine::MacPpc::perf_frames`] (the host harness; a silicon run reads the HIST lines).
//!
//! Times are in the host clock's units; `per_us` converts. A frame's stage split is the DELTA of the window's cumulative counters
//! between its two swaps, with the slice the swap happens inside counted up to the swap (not to the end of the slice), so a frame is
//! charged for what ran during it and not for the slice's remainder.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Frames kept for an on-demand dump (about two report windows).
pub const RECENT: usize = 512;
/// The histogram's upper bounds in milliseconds; the last bucket is "at or above the final bound".
pub const BUCKETS_MS: [u64; 6] = [8, 16, 33, 50, 100, 200];

/// One frame (swap to swap) and what its time was spent on, in host clock units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameRec {
    pub dur: u64,
    /// Time inside the engine's slices (interpreter plus glue).
    pub step: u64,
    /// Time inside glue calls, all classes (a part of `step`).
    pub native: u64,
    /// Glue calls that are GL (a part of `native`).
    pub gl: u64,
    /// The guest blocked on the GL worker's round trip (a part of `gl`).
    pub ring: u64,
}

impl FrameRec {
    /// The interpreter's own time: the slices minus the glue.
    pub fn interp(&self) -> u64 { self.step.saturating_sub(self.native) }
    /// The time outside the engine's slices: the host's event loop, timer waits, the worker running elsewhere.
    pub fn outside(&self) -> u64 { self.dur.saturating_sub(self.step) }
}

/// `x` units as tenths of a millisecond, printed `12.3`.
fn ms1(x: u64, per_us: u64) -> String {
    let t = x.saturating_mul(10) / (per_us.max(1) * 1000);
    format!("{}.{}", t / 10, t % 10)
}

fn mean(recs: &[&FrameRec], f: impl Fn(&FrameRec) -> u64) -> u64 {
    if recs.is_empty() { 0 } else { recs.iter().map(|r| f(r)).sum::<u64>() / recs.len() as u64 }
}

fn split(recs: &[&FrameRec], per_us: u64) -> String {
    format!("dur={} interp={} native={} gl={} ring={} outside={}", ms1(mean(recs, |r| r.dur), per_us), ms1(mean(recs, FrameRec::interp), per_us),
        ms1(mean(recs, |r| r.native.saturating_sub(r.gl)), per_us), ms1(mean(recs, |r| r.gl.saturating_sub(r.ring)), per_us),
        ms1(mean(recs, |r| r.ring), per_us), ms1(mean(recs, FrameRec::outside), per_us))
}

/// The `[HEIRLOOM-PERF-HIST ...]` line for one window's frames: percentiles in ms, the bucket counts, the share over 33 ms (the
/// 30 fps budget), and the tail (frames at or above p95) against the median frames (at or below p50), each a mean per frame in ms
/// split `dur = interp + native(non-GL glue) + gl(excluding the ring wait) + ring + outside` - the stage the tail has more of than
/// the median is the stage that makes it a tail.
pub fn hist_line(recs: &[FrameRec], per_us: u64, frames_total: u64) -> String {
    let mut order: Vec<&FrameRec> = recs.iter().collect();
    order.sort_by_key(|r| r.dur);
    let n = order.len().max(1);
    let at = |q: usize| order.get((n * q / 100).min(n - 1)).map_or(0, |r| r.dur);
    let max = order.last().map_or(0, |r| r.dur);
    let ms = |units: u64| units / (per_us.max(1) * 1000);
    let mut counts = [0u64; BUCKETS_MS.len() + 1];
    for r in &order {
        let b = BUCKETS_MS.iter().position(|&hi| ms(r.dur) < hi).unwrap_or(BUCKETS_MS.len());
        counts[b] += 1;
    }
    let buckets: String = (0..counts.len()).map(|i| match BUCKETS_MS.get(i) { Some(hi) => format!(" <{hi}={}", counts[i]), None => format!(" >={}={}", BUCKETS_MS[BUCKETS_MS.len() - 1], counts[i]) }).collect();
    let over = order.iter().filter(|r| ms(r.dur) >= 33).count();
    let (p50, p95) = (at(50), at(95));
    let tail: Vec<&FrameRec> = order.iter().copied().filter(|r| r.dur >= p95).collect();
    let med: Vec<&FrameRec> = order.iter().copied().filter(|r| r.dur <= p50).collect();
    format!("[HEIRLOOM-PERF-HIST frames={frames_total} n={} ms p50={} p90={} p95={} p99={} max={} |{buckets} | over33ms={}/{} | tail(>=p95,n={}) {} | median(<=p50,n={}) {}]",
        order.len(), ms1(p50, per_us), ms1(at(90), per_us), ms1(p95, per_us), ms1(at(99), per_us), ms1(max, per_us), over, order.len(),
        tail.len(), split(&tail, per_us), med.len(), split(&med, per_us))
}

/// The last frames, one line each, oldest first: `[HEIRLOOM-PERF-FRAME i dur=.. interp=.. native=.. gl=.. ring=.. outside=..]` in ms.
pub fn frame_lines(recent: &[FrameRec], per_us: u64) -> Vec<String> {
    recent.iter().enumerate().map(|(i, r)| format!("[HEIRLOOM-PERF-FRAME {i} {}]", split(&[r], per_us))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(dur_ms: u64, ring_ms: u64) -> FrameRec { FrameRec { dur: dur_ms * 1000, step: dur_ms * 900, native: dur_ms * 300, gl: dur_ms * 200 + ring_ms * 1000, ring: ring_ms * 1000 } }

    #[test]
    fn the_histogram_names_the_stage_that_makes_the_tail() {
        // 95 steady 10 ms frames and 5 outliers of 50 ms that wait 30 ms on the GL ring (1000 units per ms: per_us = 1)
        let mut recs: Vec<FrameRec> = (0..95).map(|_| rec(10, 0)).collect();
        recs.extend((0..5).map(|_| rec(50, 30)));
        let line = hist_line(&recs, 1, 100);
        assert!(line.contains("p50=10.0 p90=10.0 p95=50.0 p99=50.0 max=50.0"), "{line}");
        assert!(line.contains(" <8=0 <16=95 <33=0 <50=0 <100=5 <200=0 >=200=0"), "{line}");
        assert!(line.contains("over33ms=5/100"), "{line}");
        // the tail's ring wait is 30.0 ms against the median's 0.0: the outliers ARE the ring
        assert!(line.contains("tail(>=p95,n=5) dur=50.0") && line.contains("ring=30.0"), "{line}");
        assert!(line.contains("median(<=p50,n=95) dur=10.0 interp=6.0 native=1.0 gl=2.0 ring=0.0 outside=1.0]"), "{line}");
    }

    #[test]
    fn an_empty_window_and_a_dump_are_well_formed() {
        assert!(hist_line(&[], 1, 0).contains("n=0 ms p50=0.0"));
        let lines = frame_lines(&[rec(20, 0), rec(40, 10)], 1);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with("[HEIRLOOM-PERF-FRAME 1 dur=40.0 interp="), "{}", lines[1]);
    }
}
