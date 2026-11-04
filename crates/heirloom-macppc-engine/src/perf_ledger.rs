//! THE FRAME LEDGER (h86 aot-ledger, 2026-11-03; the WC3 council's step 0): every guest frame's wall time (swap to swap) as ONE
//! EXCLUSIVE account - each TSC tick charged to exactly one bucket - and the hardware counters over the same interval.
//!
//! WHY: the stage clock's shares did not add up (3.05 M guest instructions at 56 MIPS is 54.5 ms, "interp" was 37 ms, natives
//! 12.4, and `outside_step` one opaque 7-8 ms), and its unit was TSC ticks, not core cycles. Nothing could be ranked on it.
//!
//! HOW. Two state machines, each with exactly one phase open at a time, so nothing can be counted twice:
//! * the LIBRARY's ([`Clock`]): `step_other` (inside [`crate::MacPpc::step`], outside the CPU), `jit` (inside the CPU library's
//!   run, outside natives), one phase per native class, `gl_ring` (inside the GL worker's transact), `probe` (inside a counter
//!   sample). Nested phases (a native inside the run, the ring inside a native) suspend their parent; time while no phase is
//!   open is not the library's to name;
//! * the ENGINE's ([`heirloom_os_abi::perf::EnginePhases`], read through `perf_attach`): waiting for the app, serving a batch,
//!   its own lines, inside the library, and the GL carrier's own three (carrier, back-pressure wait, ring push) - which happen
//!   INSIDE the library's `gl_ring` phase and are subtracted from it.
//! At the swap both are read at one `now`; the frame's buckets are the deltas since the last swap: the library's phases, the
//! engine's, `lib_other` = the engine's `lib` minus what the library named in it, and the RESIDUE = wall - every bucket. The
//! residue is the ledger's own check: near zero means the account is complete; a host with no engine phases leaves the engine's
//! whole share there.
//!
//! The counters (`HwSample`, the thread's own: APERF/MPERF, instructions, core and reference cycles, one programmable pair) are
//! read at each swap (the frame) and around each run of the CPU library (the guest's instructions and the natives inside them),
//! inside the `probe` phase: the probe's own cost is a bucket, never hidden in another. The pair rotates per report window
//! through [`PAIRS`] (two programmable counters on the Silvermont family), so one deployment sees every pair.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_os_abi::perf::{self as hw, HwSample, PerfHost, ENGINE_PHASES, PH_GL_CARRIER, PH_GL_PUSH, PH_GL_WAIT, PH_LIB, PH_LOG, PH_SERVE, PH_WAIT_APP};

/// The library's phases: `jit`, then one per native class (crate::perf::Class + 1), the ring, the rest of `step`, the probe.
pub const L_JIT: usize = 0;
pub const L_GL_RING: usize = 1 + crate::perf::CLASSES;
pub const L_STEP_OTHER: usize = L_GL_RING + 1;
pub const L_PROBE: usize = L_STEP_OTHER + 1;
pub const NLIB: usize = L_PROBE + 1;
const OUT: u8 = u8::MAX;
const DEPTH: usize = 8;

/// The library's phase clock (one phase open at a time; nested phases suspend their parent).
#[derive(Debug, Clone)]
pub struct Clock {
    cur: u8,
    since: u64,
    acc: [u64; NLIB],
    stack: [u8; DEPTH],
    depth: usize,
}

impl Default for Clock {
    fn default() -> Self { Clock { cur: OUT, since: 0, acc: [0; NLIB], stack: [OUT; DEPTH], depth: 0 } }
}

impl Clock {
    fn charge(&mut self, now: u64) {
        if let Some(a) = self.acc.get_mut(usize::from(self.cur)) { *a = a.wrapping_add(now.wrapping_sub(self.since)); }
        self.since = now;
    }
    /// Open phase `p` at `now` (the open one is suspended until the matching [`Clock::exit`]).
    pub fn enter(&mut self, p: usize, now: u64) {
        self.charge(now);
        if self.depth < DEPTH { self.stack[self.depth] = self.cur; }
        self.depth += 1;
        self.cur = p.min(NLIB - 1) as u8;
    }
    /// Close the open phase at `now`, resuming its parent.
    pub fn exit(&mut self, now: u64) {
        self.charge(now);
        self.depth = self.depth.saturating_sub(1);
        self.cur = if self.depth < DEPTH { self.stack[self.depth] } else { OUT };
    }
    /// Every phase's ticks as of `now`, the open one included.
    pub fn at(&self, now: u64) -> [u64; NLIB] {
        let mut a = self.acc;
        if let Some(x) = a.get_mut(usize::from(self.cur)) { *x = x.wrapping_add(now.wrapping_sub(self.since)); }
        a
    }
}

/// The frame's exclusive buckets, in print order.
pub const BUCKETS: [&str; 17] = ["jit", "gl", "clock", "thread", "event", "file", "other", "gl_ring", "gl_carrier", "gl_wait", "gl_push",
    "step_other", "probe", "lib_other", "serve", "wait_app", "log"];
pub const NB: usize = BUCKETS.len();
const B_GL_CARRIER: usize = 8;
const B_STEP_OTHER: usize = 11;
const B_LIB_OTHER: usize = 13;
const B_SERVE: usize = 14;

/// Counter deltas: run_tsc, aperf, mperf, inst, core, ref, pmc0, pmc1, switches, migrations.
pub type Hw = [u64; 10];
const HW_RUN: usize = 0;
const HW_APERF: usize = 1;
const HW_MPERF: usize = 2;
const HW_INST: usize = 3;
const HW_CORE: usize = 4;
const HW_PMC: usize = 6;
const HW_SWITCHES: usize = 8;
const HW_MIGR: usize = 9;

fn hw_of(s: &HwSample) -> Hw {
    [s.run_tsc, s.aperf, s.mperf, s.inst, s.core_cycles, s.ref_cycles, s.pmc[0], s.pmc[1], s.switches, s.migrations]
}
fn hw_sub(a: &Hw, b: &Hw) -> Hw { core::array::from_fn(|i| a[i].wrapping_sub(b[i])) }
fn hw_add(a: &mut Hw, d: &Hw) { for i in 0..a.len() { a[i] = a[i].wrapping_add(d[i]); } }

/// One closed frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame {
    pub wall: u64,
    pub insns: u64,
    pub b: [u64; NB],
    /// wall - every bucket (signed: open-interval rounding can make it a few ticks negative).
    pub residue: i64,
    /// The thread's counters over the frame, and over the CPU library's runs inside it.
    pub hw: Hw,
    pub hw_run: Hw,
    /// Ticks inside the CPU library's runs (jit + natives + ring + the engine's GL phases inside them): `hw_run`'s interval.
    pub run_ticks: u64,
    /// Tier 2's events in the frame: blocks compiled, publishes, withdraws (arena reuse), bails.
    pub jit: [u64; 4],
}

/// The programmable pairs, one per report window in turn ([`hw::HW_EVENTS`] indices): the council's suspects, two at a time.
pub const PAIRS: [(usize, usize); 6] = [(1, 2), (3, 4), (5, 6), (8, 9), (10, 11), (7, 12)];

/// The ledger: the library's clock, the engine's host, the last swap's snapshot and the window's frames.
#[derive(Default)]
pub struct Ledger {
    pub clock: Clock,
    pub host: Option<PerfHost>,
    /// The counters are off (no host, or the host has none): no sample is taken again.
    hw_off: bool,
    /// The pair of this window (an index into PAIRS) and whether it still has to be asked for (at the window's first swap).
    pair: usize,
    pair_due: bool,
    pair_refused: bool,
    last: Option<Snap>,
    run_open: Option<(u64, Hw)>,
    run_acc: (u64, Hw),
    pub frames: Vec<Frame>,
    /// Windows closed (the pair rotation's counter).
    pub windows: u64,
    /// The engine's GL phases (carrier + wait + push) charged INSIDE the library's `gl_ring` phase, cumulative, and their
    /// total at the open ring's entry. Only these are nested in a library phase: the engine also settles a batch in flight
    /// OUTSIDE the library (the guest stopped running), and that wait is its own bucket, never subtracted from the library's.
    ring_eng: u64,
    ring_open: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct Snap { now: u64, lib: [u64; NLIB], eng: [u64; ENGINE_PHASES], hw: Hw, tb: u64, jit: [u64; 4], ring_eng: u64 }

impl Ledger {
    /// Attach the engine's half; the first swap after it starts the account.
    pub fn attach(&mut self, host: PerfHost) {
        (self.host, self.hw_off, self.pair_due, self.last) = (Some(host), false, true, None);
    }

    /// One counter sample inside the probe phase (None when there are no counters).
    fn sample(&mut self, now: fn() -> u64, select: u32) -> Option<Hw> {
        if self.hw_off { return None; }
        let h = self.host?;
        self.clock.enter(L_PROBE, now());
        let r = h.sample(select);
        self.clock.exit(now());
        match r {
            Ok(s) => Some(hw_of(&s)),
            Err(hw::HW_PAIR_REFUSED) => { self.pair_refused = true; h.sample(hw::SELECT_KEEP).ok().map(|s| hw_of(&s)) }
            Err(_) => { self.hw_off = true; None }
        }
    }

    fn eng_gl(&self, now: u64) -> u64 {
        self.host.and_then(|h| h.phases_at(now)).map_or(0, |e| e[PH_GL_CARRIER] + e[PH_GL_WAIT] + e[PH_GL_PUSH])
    }
    /// The GL worker's transact begins / ends (gl_c.rs `gl_flush`): the library's `gl_ring` phase, and the engine's GL phases
    /// nested in it (h86 Z83 jit run, violet meadow wave: the slow frames' -7.5 ms residue was the engine's OUTSIDE settle
    /// subtracted from the library's account as if it had been inside it).
    pub fn ring_enter(&mut self, now: u64) {
        self.clock.enter(L_GL_RING, now);
        self.ring_open = Some(self.eng_gl(now));
    }
    pub fn ring_exit(&mut self, now: u64) {
        self.clock.exit(now);
        if let Some(e0) = self.ring_open.take() {
            self.ring_eng = self.ring_eng.wrapping_add(self.eng_gl(now).wrapping_sub(e0));
        }
    }

    /// The CPU library's run begins / ends (around `Exec::resume`): the counters over the guest's instructions.
    pub fn run_begin(&mut self, now: fn() -> u64) {
        let s = self.sample(now, hw::SELECT_KEEP);
        self.run_open = Some((now(), s.unwrap_or_default()));
        self.clock.enter(L_JIT, now());
    }
    pub fn run_end(&mut self, now: fn() -> u64) {
        self.clock.exit(now());
        let Some((t0, h0)) = self.run_open.take() else { return };
        let t1 = now();
        if let Some(h1) = self.sample(now, hw::SELECT_KEEP) {
            hw_add(&mut self.run_acc.1, &hw_sub(&h1, &h0));
        }
        self.run_acc.0 += t1.wrapping_sub(t0);
    }

    /// The guest's swap at `t` (instructions retired `tb`, tier 2's cumulative events `jit`): close the frame.
    pub fn swap(&mut self, now: fn() -> u64, tb: u64, jit: [u64; 4]) {
        // a window's first swap asks for its pair (the kernel restarts the pair's sums: this snapshot is its base)
        let select = if self.pair_due { self.pair_due = false; let (a, b) = PAIRS[self.pair]; hw::select(a, b) } else { hw::SELECT_KEEP };
        let hwv = self.sample(now, select).unwrap_or_default();
        let t = now();
        let lib = self.clock.at(t);
        let eng = self.host.and_then(|h| h.phases_at(t)).unwrap_or([0; ENGINE_PHASES]);
        let ring_eng = self.ring_eng.wrapping_add(self.ring_open.map_or(0, |e0| self.eng_gl(t).wrapping_sub(e0)));
        let snap = Snap { now: t, lib, eng, hw: hwv, tb, jit, ring_eng };
        // a new pair restarted the programmable sums at this sample: a CPU-library run open across it has its base there too
        // (h86 Z83: its delta crossed the restart - new sums minus the old pair's - and wrapped below zero, ~2^64 per 1k insns)
        if select != hw::SELECT_KEEP {
            if let Some((_, h0)) = self.run_open.as_mut() { (h0[HW_PMC], h0[HW_PMC + 1]) = (hwv[HW_PMC], hwv[HW_PMC + 1]); }
        }
        let run = core::mem::take(&mut self.run_acc);
        if let Some(p) = self.last.replace(snap) {
            let mut f = Frame { wall: t.wrapping_sub(p.now), insns: tb.wrapping_sub(p.tb), hw: hw_sub(&hwv, &p.hw), hw_run: run.1, run_ticks: run.0, ..Frame::default() };
            if select != hw::SELECT_KEEP { (f.hw[HW_PMC], f.hw[HW_PMC + 1], f.hw_run[HW_PMC], f.hw_run[HW_PMC + 1]) = (0, 0, 0, 0); } // the pair restarted at this swap: its frame counts none
            f.jit = core::array::from_fn(|i| jit[i].wrapping_sub(p.jit[i]));
            let dl: [u64; NLIB] = core::array::from_fn(|i| lib[i].wrapping_sub(p.lib[i]));
            let de: [u64; ENGINE_PHASES] = core::array::from_fn(|i| eng[i].wrapping_sub(p.eng[i]));
            let eng_gl = ring_eng.wrapping_sub(p.ring_eng); // the engine's GL time INSIDE the library's ring phase only
            f.b[..L_GL_RING].copy_from_slice(&dl[..L_GL_RING]); // jit + the native classes
            f.b[7] = dl[L_GL_RING].saturating_sub(eng_gl); // the library's own side of the ring
            (f.b[B_GL_CARRIER], f.b[9], f.b[10]) = (de[PH_GL_CARRIER], de[PH_GL_WAIT], de[PH_GL_PUSH]);
            (f.b[B_STEP_OTHER], f.b[12]) = (dl[L_STEP_OTHER], dl[L_PROBE]);
            let named: u64 = dl.iter().sum::<u64>().saturating_sub(eng_gl);
            f.b[B_LIB_OTHER] = de[PH_LIB].saturating_sub(named);
            (f.b[B_SERVE], f.b[15], f.b[16]) = (de[PH_SERVE], de[PH_WAIT_APP], de[PH_LOG]);
            f.residue = f.wall as i64 - f.b.iter().sum::<u64>() as i64;
            if self.frames.try_reserve(1).is_ok() { self.frames.push(f); }
        }
    }

    /// Close the window: its frames, and the pair for the next one.
    pub fn take_window(&mut self) -> (Vec<Frame>, (usize, usize), bool) {
        let pair = PAIRS[self.pair];
        let refused = core::mem::take(&mut self.pair_refused);
        self.windows += 1;
        self.pair = (self.pair + 1) % PAIRS.len();
        self.pair_due = true;
        (core::mem::take(&mut self.frames), pair, refused)
    }
}

fn q(sorted: &[u64], pct: usize) -> u64 { sorted.get((sorted.len() * pct / 100).min(sorted.len().saturating_sub(1))).copied().unwrap_or(0) }

/// Mean microseconds of each bucket over `frames` (and the residue), as " name=us" pairs.
fn means(frames: &[&Frame], per_us: u64) -> String {
    let n = frames.len().max(1) as u64;
    let mut s = String::new();
    for (i, name) in BUCKETS.iter().enumerate() {
        let t: u64 = frames.iter().map(|f| f.b[i]).sum();
        s.push_str(&format!(" {name}={}", t / per_us.max(1) / n));
    }
    let r: i64 = frames.iter().map(|f| f.residue).sum();
    s.push_str(&format!(" residue={}", r / per_us.max(1) as i64 / n as i64));
    s
}

/// The window's two `[HEIRLOOM-LEDGER ...]` lines and its `[HEIRLOOM-HW ...]` line, at `per_us` clock units a microsecond.
/// `frames_total` names them (the same number as the window's `[HEIRLOOM-PERF ...]` line).
pub fn lines(frames: &[Frame], pair: (usize, usize), pair_refused: bool, per_us: u64, frames_total: u64) -> Vec<String> {
    let per_us = per_us.max(1);
    let n = frames.len();
    let mut out = Vec::new();
    if n == 0 { return out; }
    let mut walls: Vec<u64> = frames.iter().map(|f| f.wall).collect();
    walls.sort_unstable();
    let mut ins: Vec<u64> = frames.iter().map(|f| f.insns).collect();
    ins.sort_unstable();
    let (deadline33, deadline50) = (33_000 * per_us, 50_000 * per_us);
    let miss = |d: u64| frames.iter().filter(|f| f.wall > d).count();
    let p95 = q(&walls, 95);
    let all: Vec<&Frame> = frames.iter().collect();
    let slow: Vec<&Frame> = frames.iter().filter(|f| f.wall >= p95).collect();
    let ms = |t: u64| format!("{}.{}", t / per_us / 1000, t / per_us / 100 % 10);
    out.push(format!("[HEIRLOOM-LEDGER frames={frames_total} n={n}] wall_ms p50={} p95={} p99={} max={} miss33={miss33} miss50={miss50} | insns/frame p50={} p95={} \
| mean_us:{} | slow_us (>=p95, k={}):{}",
        ms(q(&walls, 50)), ms(p95), ms(q(&walls, 99)), ms(walls[n - 1]), q(&ins, 50), q(&ins, 95), means(&all, per_us), slow.len(), means(&slow, per_us),
        miss33 = miss(deadline33), miss50 = miss(deadline50)));
    // the worst frame alone, and how often the slow frames carry tier 2's events against every frame
    let worst = frames.iter().max_by_key(|f| f.wall).copied().unwrap_or_default();
    let with = |set: &[&Frame], k: usize| set.iter().filter(|f| f.jit[k] > 0).count();
    // time off a CPU needs the thread's run time (the counters); without them it is not known, and says so (`-`)
    let has_hw = frames.iter().any(|f| f.hw[HW_RUN] != 0);
    let offcpu = |f: &Frame| f.wall.saturating_sub(f.hw[HW_RUN]);
    let us = |t: u64| if has_hw { format!("{}", t / per_us) } else { String::from("-") };
    out.push(format!("[HEIRLOOM-LEDGER-TAIL frames={frames_total}] worst ms={} insns={} compiles={} publishes={} withdraws={} bails={} switches={} migrations={} offcpu_us={} |{} \
| frames with compiles: slow {}/{} all {}/{n}; publishes: slow {}/{} all {}/{n}; withdraws: slow {}/{} all {}/{n} | offcpu_us mean all={} slow={}",
        ms(worst.wall), worst.insns, worst.jit[0], worst.jit[1], worst.jit[2], worst.jit[3], worst.hw[HW_SWITCHES], worst.hw[HW_MIGR], us(offcpu(&worst)),
        means(&[&worst], per_us), with(&slow, 0), slow.len(), with(&all, 0), with(&slow, 1), slow.len(), with(&all, 1), with(&slow, 2), slow.len(), with(&all, 2),
        us(all.iter().map(|f| offcpu(f)).sum::<u64>() / n as u64), us(slow.iter().map(|f| offcpu(f)).sum::<u64>() / slow.len().max(1) as u64)));
    // the counters: frequency, on-CPU share, and per guest instruction over the CPU library's runs
    let sum = |sel: &dyn Fn(&Frame) -> u64| frames.iter().map(sel).sum::<u64>();
    let (aperf, mperf) = (sum(&|f| f.hw[HW_APERF]), sum(&|f| f.hw[HW_MPERF]));
    if mperf == 0 {
        out.push(format!("[HEIRLOOM-HW frames={frames_total}] none: no counters (no PerfCounters right, no PMU, or a host without them)"));
        return out;
    }
    let wall: u64 = walls.iter().sum();
    let (r_aperf, r_mperf, r_inst, r_core) = (sum(&|f| f.hw_run[HW_APERF]), sum(&|f| f.hw_run[HW_MPERF]), sum(&|f| f.hw_run[HW_INST]), sum(&|f| f.hw_run[HW_CORE]));
    let g = sum(&|f| f.insns).max(1);
    let jit_ticks = sum(&|f| f.b[0]);
    let x100 = |a: u64, b: u64| if b == 0 { 0 } else { a.saturating_mul(100) / b };
    let fix2 = |v: u64| format!("{}.{:02}", v / 100, v % 100);
    // the JIT's own core cycles: its TSC ticks at the runs' measured frequency (aperf/mperf over the runs)
    let jit_core = if r_mperf == 0 { 0 } else { (jit_ticks as u128 * r_aperf as u128 / r_mperf as u128) as u64 };
    let pmc = |k: usize| sum(&|f| f.hw_run[HW_PMC + k]);
    let ev = |k: usize, idx: usize| {
        let name = hw::HW_EVENTS.get(idx).copied().unwrap_or("?");
        if name.ends_with("CYCLES") { format!("{name}={}% of core cycles", x100(pmc(k), r_core)) } else { format!("{name}={}/1k guest insns", pmc(k).saturating_mul(1000) / g) }
    };
    out.push(format!("[HEIRLOOM-HW frames={frames_total}] freq_mhz={} run_freq_mhz={} oncpu={}% switches/frame={} migrations/frame={} \
| runs: guest_insns={g} ticks/ginsn={} core_cycles/ginsn={} host_insns/ginsn={} ipc={} jit_core_cycles/ginsn={} \
| pair: {} {}{}",
        aperf.saturating_mul(per_us) / mperf, if r_mperf == 0 { 0 } else { r_aperf.saturating_mul(per_us) / r_mperf }, x100(sum(&|f| f.hw[HW_RUN]), wall),
        sum(&|f| f.hw[HW_SWITCHES]) / n as u64, sum(&|f| f.hw[HW_MIGR]) / n as u64,
        fix2(x100(sum(&|f| f.run_ticks), g)), fix2(x100(r_core, g)), fix2(x100(r_inst, g)), fix2(x100(r_inst, r_core)), fix2(x100(jit_core, g)),
        ev(0, pair.0), ev(1, pair.1), if pair_refused { " (pair REFUSED: zeros)" } else { "" }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, Ordering};

    static T: AtomicU64 = AtomicU64::new(1000);
    fn now() -> u64 { T.load(Ordering::Relaxed) }
    fn at(t: u64) { T.store(t, Ordering::Relaxed) }

    /// Phases nest and suspend their parent: a native inside the run, the ring inside the native; each tick once.
    #[test]
    fn the_clock_charges_every_tick_to_the_innermost_phase() {
        let mut c = Clock::default();
        c.enter(L_STEP_OTHER, 100);
        c.enter(L_JIT, 110);
        c.enter(1, 150); // the GL class
        c.enter(L_GL_RING, 160);
        c.exit(190);
        c.exit(200);
        c.exit(230);
        c.exit(240);
        let a = c.at(1000);
        assert_eq!((a[L_STEP_OTHER], a[L_JIT], a[1], a[L_GL_RING]), (20, 70, 20, 30));
        assert_eq!(a.iter().sum::<u64>(), 140, "outside every phase nothing is charged");
    }

    /// A frame with engine phases: the GL carrier's phases come out of the library's ring, the rest of `lib` is lib_other, and
    /// the residue is zero when every tick of the frame was in some phase.
    #[test]
    fn a_frame_is_one_exclusive_account_of_its_wall_time() {
        let mut eng = heirloom_os_abi::perf::EnginePhases::default();
        extern "C" fn none(_: *mut core::ffi::c_void, _: u32, _: *mut HwSample) -> i32 { hw::HW_NONE }
        let mut l = Ledger::default();
        l.attach(PerfHost { size: 0, flags: 0, ctx: core::ptr::null_mut(), phases: &eng, sample: none });
        eng.enter(PH_LIB, 1000);
        at(1000);
        l.swap(now, 0, [0; 4]); // the first swap: the base
        // the frame: 100 lib-other, 50 step_other, then a run of 400 with a 100 GL native holding a 60 ring (engine: 10 carrier,
        // 30 wait, 15 push), then the engine serves 70 and waits 200 for the app
        let e = |eng: &mut heirloom_os_abi::perf::EnginePhases, p, t| { eng.enter(p, t); };
        l.clock.enter(L_STEP_OTHER, 1100);
        at(1150);
        l.run_begin(now);
        l.clock.enter(1, 1300);
        l.ring_enter(1320);
        e(&mut eng, PH_GL_CARRIER, 1325);
        e(&mut eng, PH_GL_WAIT, 1335);
        e(&mut eng, PH_GL_PUSH, 1365);
        e(&mut eng, PH_LIB, 1380);
        l.ring_exit(1380);
        l.clock.exit(1400);
        at(1550);
        l.run_end(now);
        l.clock.exit(1550);
        // the engine serves 70, of which it SETTLES the batch in flight for 30 OUTSIDE the library (the guest stopped
        // running): its own gl_wait, never the library's (the Z83's slow frames counted it twice), then waits 200 for the app
        e(&mut eng, PH_SERVE, 1550);
        e(&mut eng, PH_GL_WAIT, 1580);
        e(&mut eng, PH_SERVE, 1610);
        e(&mut eng, PH_WAIT_APP, 1650);
        e(&mut eng, PH_LIB, 1850);
        at(1850);
        l.swap(now, 3_000, [2, 1, 0, 0]);
        let f = l.frames[0];
        assert_eq!(f.wall, 850);
        assert_eq!(f.insns, 3_000);
        let b = |name: &str| f.b[BUCKETS.iter().position(|n| *n == name).unwrap()];
        assert_eq!((b("jit"), b("gl"), b("gl_ring"), b("gl_carrier"), b("gl_wait"), b("gl_push")), (300, 40, 5, 10, 30 + 30, 15));
        assert_eq!((b("step_other"), b("probe"), b("serve"), b("wait_app")), (50, 0, 70, 200));
        assert_eq!(b("lib_other"), 100, "the engine's lib time the library did not name");
        assert_eq!(f.residue, 0, "every tick is in exactly one bucket");
        assert_eq!(f.jit, [2, 1, 0, 0]);
        let lines = lines(&l.frames, PAIRS[0], false, 1, 2);
        assert!(lines[0].starts_with("[HEIRLOOM-LEDGER frames=2 n=1] wall_ms p50=0.8 "), "{}", lines[0]);
        assert!(lines[0].contains(" residue=0 "), "{}", lines[0]);
        assert!(lines[0].contains(" jit=300 gl=40 "), "{}", lines[0]);
        assert!(lines[2].starts_with("[HEIRLOOM-HW frames=2] none"), "{}", lines[2]);
    }

    /// Counters: the run brackets give per-guest-instruction rates, the pair rotates per window and its first swap restarts it.
    #[test]
    fn the_counters_give_core_cycles_per_guest_instruction_and_the_pair_rotates() {
        static N: AtomicU64 = AtomicU64::new(0);
        static SEL: AtomicU64 = AtomicU64::new(0);
        // after a pair change the kernel's pair sums restart from the sample that asked for it (k0)
        static K0: AtomicU64 = AtomicU64::new(0);
        extern "C" fn counts(_: *mut core::ffi::c_void, select: u32, out: *mut HwSample) -> i32 {
            let k = N.fetch_add(1, Ordering::Relaxed) + 1;
            if select != hw::SELECT_KEEP { SEL.store(u64::from(select), Ordering::Relaxed); if N.load(Ordering::Relaxed) > 4 { K0.store(k, Ordering::Relaxed); } }
            let pk = if K0.load(Ordering::Relaxed) != 0 { k - K0.load(Ordering::Relaxed) } else { k };
            // every sample: 1000 TSC-rate ticks on, 1333 core cycles (a 1.333x burst), 2000 instructions retired
            // SAFETY: the test passes a valid record.
            unsafe { *out = HwSample { valid: 7, run_tsc: k * 1000, aperf: k * 1333, mperf: k * 1000, inst: k * 2000, core_cycles: k * 1333, pmc: [pk * 10, pk * 3], ..HwSample::default() } };
            hw::HW_OK
        }
        let mut l = Ledger::default();
        l.attach(PerfHost { size: 0, flags: 0, ctx: core::ptr::null_mut(), phases: core::ptr::null(), sample: counts });
        at(0);
        l.swap(now, 0, [0; 4]);
        assert_eq!(SEL.load(Ordering::Relaxed), u64::from(hw::select(1, 2)), "the window's first swap asks for its pair");
        at(100);
        l.run_begin(now);
        at(1100);
        l.run_end(now);
        at(1200);
        l.swap(now, 500, [0; 4]);
        let lines = lines(&l.frames, PAIRS[0], false, 1, 2);
        let hwl = &lines[2];
        assert!(hwl.contains("core_cycles/ginsn=2.66 host_insns/ginsn=4.00 ipc=1.50"), "{hwl}");
        assert!(hwl.contains("BR_MISP_RETIRED.ALL_BRANCHES=20/1k guest insns ICACHE.MISSES=6/1k guest insns"), "{hwl}");
        assert!(hwl.contains("run_freq_mhz=1"), "aperf/mperf x per_us: {hwl}");
        let (_, pair, _) = l.take_window();
        assert_eq!(pair, PAIRS[0]);
        // the next window's first swap lands INSIDE a run (as on the Z83: the swap is a native inside the CPU library's run) and
        // restarts the pair's sums: the run's delta must not cross the restart (it wrapped below zero there)
        at(1300);
        l.run_begin(now);
        l.swap(now, 600, [0; 4]);
        assert_eq!(SEL.load(Ordering::Relaxed), u64::from(hw::select(PAIRS[1].0, PAIRS[1].1)), "the next window counts the next pair");
        at(1400);
        l.run_end(now);
        at(1500);
        l.swap(now, 700, [0; 4]);
        let f = l.frames.last().unwrap();
        assert!(f.hw_run[HW_PMC] < 1_000 && f.hw_run[HW_PMC + 1] < 1_000, "no wrapped pmc delta: {:?}", f.hw_run);
    }
}
