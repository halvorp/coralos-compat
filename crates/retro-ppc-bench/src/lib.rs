//! The retro-ppc benchmark: fixed workloads run on a chosen execution tier, measured (instructions retired, elapsed,
//! instructions/s, native callouts) and fingerprinted (a digest of the final architectural state and all of guest
//! RAM), so a faster tier is only ever compared on a run that ended in the same state as tier 0.
pub mod apps;
pub mod synth;

use retro_guest_mem::GuestMem;
use retro_ppc::{Cpu, Exec, Tier};
use std::time::Instant;

/// The scheduler quantum of the synthetic runs (the engine's own loop decides for the applications).
pub const SLICE: u64 = 100_000;

#[derive(Debug, Clone)]
pub struct Measure {
    pub name: String,
    pub tier: Tier,
    pub insns: u64,
    pub natives: u64,
    pub elapsed_us: u64,
    /// FNV-1a of registers, time base and guest RAM at the end.
    pub digest: u64,
    /// How the run ended (and the workload's own check).
    pub outcome: String,
    pub prof: retro_ppc::Prof,
    /// With native timing: (symbol, calls, nanoseconds inside), most time first.
    pub native_report: Vec<(String, u64, u64)>,
    pub cache: retro_ppc::CacheStats,
    pub lockstep: retro_ppc::LockstepStats,
}

impl Measure {
    pub fn ips(&self) -> f64 { if self.elapsed_us == 0 { 0.0 } else { self.insns as f64 * 1e6 / self.elapsed_us as f64 } }
}

fn fnv(h: &mut u64, bytes: &[u8]) { for b in bytes { *h ^= *b as u64; *h = h.wrapping_mul(0x100_0000_01B3); } }

pub fn digest(c: &Cpu, mem: &GuestMem) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for g in c.gpr { fnv(&mut h, &g.to_be_bytes()); }
    for f in c.fpr { fnv(&mut h, &f.to_be_bytes()); }
    for r in [c.cr, c.lr, c.ctr, c.xer, c.fpscr, c.pc] { fnv(&mut h, &r.to_be_bytes()); }
    fnv(&mut h, &c.tb.to_be_bytes());
    fnv(&mut h, mem.bytes(mem.span(0, mem.len() as u32).unwrap()).unwrap());
    h
}

/// What to collect besides the counters.
#[derive(Clone, Copy, Default)]
pub struct Opts { pub census: bool, pub natives: bool }

fn clock_ns() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Configure a fresh runner for `tier` (with a census / native timing when asked).
pub fn exec_for(tier: Tier, o: Opts) -> Exec {
    let mut e = Exec::new(tier);
    if o.census { e.enable_census(); }
    if o.natives { e.profile_natives(clock_ns); }
    e
}

fn native_report(e: &Exec, name: impl Fn(u32) -> String) -> Vec<(String, u64, u64)> {
    let Some(p) = e.native_profile() else { return Vec::new() };
    let mut v: Vec<(String, u64, u64)> = p.by_addr.iter().map(|(a, (n, t))| (name(*a), *n, *t)).collect();
    v.sort_by(|a, b| b.2.cmp(&a.2));
    v
}

pub fn run_synth(s: &synth::Synth, tier: Tier, o: Opts) -> (Measure, Exec) {
    let (mut c, mut v) = synth::load(s);
    let mut exec = exec_for(tier, o);
    let t = Instant::now();
    let st = synth::run(&mut exec, &mut c, &mut v, SLICE);
    let elapsed_us = t.elapsed().as_micros() as u64;
    let mut outcome = match (st.halted, st.trap) { (true, None) => "exit".to_string(), (_, Some(t)) => format!("TRAP {t}"), _ => "stopped".into() };
    if let Err(e) = (s.check)(&c, &v, s.n) { outcome = format!("{outcome}; CHECK FAILED: {e}"); }
    if let Some(d) = exec.divergence() { outcome = format!("LOCKSTEP DIVERGENCE at block {:#x}: {:?}; {outcome}", d.block, d.what); }
    let native_report = native_report(&exec, |a| format!("{a:#x}"));
    let m = Measure { name: s.name.into(), tier, insns: exec.prof.insns, natives: exec.prof.natives, elapsed_us, digest: digest(&c, &v.mem), outcome, prof: exec.prof, native_report, cache: exec.cache_stats(), lockstep: exec.lockstep_stats() };
    (m, exec)
}

pub fn run_app(a: &apps::App, tier: Tier, o: Opts) -> Result<(Measure, Exec), String> {
    let mut m = apps::load(a)?;
    *m.exec() = exec_for(tier, o);
    let t = Instant::now();
    let (n, step) = apps::run(&mut m, a.insns);
    let elapsed_us = t.elapsed().as_micros() as u64;
    let exec = std::mem::take(m.exec());
    let d = m.cpu().map_or(0, |c| digest(c, &m.mac.mem));
    let mut outcome = format!("{step:?}{}", m.fault().map(|f| format!(" ({})", f.chars().take(120).collect::<String>())).unwrap_or_default());
    if let Some(d) = exec.divergence() { outcome = format!("LOCKSTEP DIVERGENCE at block {:#x}: {:?}; {outcome}", d.block, d.what); }
    debug_assert_eq!(n, exec.prof.insns);
    let stubs: std::collections::BTreeMap<u32, String> = (0..m.glue.len()).filter_map(|i| m.glue.entry(i)).map(|e| (e.stub, format!("{}/{}", e.library, e.symbol))).collect();
    let native_report = native_report(&exec, |a| stubs.get(&a).cloned().unwrap_or_else(|| format!("{a:#x}")));
    Ok((Measure { name: a.name.clone(), tier, insns: exec.prof.insns, natives: exec.prof.natives, elapsed_us, digest: d, outcome, prof: exec.prof, native_report, cache: exec.cache_stats(), lockstep: exec.lockstep_stats() }, exec))
}

/// Geometric mean of positive ratios (0 for an empty list).
pub fn geomean(v: &[f64]) -> f64 {
    if v.is_empty() { return 0.0; }
    (v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp()
}
