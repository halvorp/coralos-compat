//! The execution tiers behind one entry point, [`Exec::resume`], and the counters a profile line is made of.
//!
//! Tier 0 is the reference interpreter ([`crate::resume`]: fetch, decode, execute one instruction at a time). Every
//! other tier must leave the SAME architectural state at every exit: same registers, memory, time base, trap and pc.
use crate::census::Census;
use crate::cpu::{Cpu, RunStats};
use crate::t1::{self, Divergence, LockstepStats};
use crate::t1cache::{Cache, CacheStats};
use crate::{exec, Host};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;

/// Which interpreter [`Exec::resume`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Tier 0: the reference, one instruction at a time.
    Interp,
    /// Tier 1: pre-decoded blocks, invalidated by code-page write generations ([`crate::t1cache`]).
    Blocks,
    /// Tier 1 checked against tier 0 on every block (slow; for gates and triage). Stops at the first divergence,
    /// which [`Exec::divergence`] then names.
    Lockstep,
}

impl Tier {
    /// The number a `[PPC-PROF ... tier=]` line prints.
    pub fn number(self) -> u32 { match self { Tier::Interp => 0, Tier::Blocks | Tier::Lockstep => 1 } }
    /// Parse `0` / `interp`, `1` / `blocks`, `lockstep`; None for anything else.
    pub fn parse(s: &str) -> Option<Tier> {
        match s { "0" | "interp" => Some(Tier::Interp), "1" | "blocks" => Some(Tier::Blocks), "lockstep" => Some(Tier::Lockstep), _ => None }
    }
}

/// Cheap counters, kept by every tier (none costs anything per instruction).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Prof {
    /// Guest instructions retired (a native hook counts as one, as in [`RunStats::insns`]).
    pub insns: u64,
    /// Native hooks entered (Toolbox / library callouts).
    pub natives: u64,
    /// Time spent inside native hooks, in the clock's units (only with [`Exec::profile_natives`]).
    pub native_time: u64,
    /// Tier 1: blocks run.
    pub blocks: u64,
}

impl Prof {
    /// The profile line: `[PPC-PROF insns=<n> native_calls=<n> elapsed_us=<n> mips=<n.nn> tier=<t>]`. Elapsed time is
    /// the caller's (this crate has no clock).
    pub fn line(&self, elapsed_us: u64, tier: Tier) -> alloc::string::String {
        let mips = if elapsed_us == 0 { 0.0 } else { self.insns as f64 / elapsed_us as f64 };
        alloc::format!("[PPC-PROF insns={} native_calls={} elapsed_us={elapsed_us} mips={mips:.2} tier={}]", self.insns, self.natives, tier.number())
    }
}

/// A runner: the selected tier, its counters, and (opt-in) a census. One per guest machine; it is not part of the
/// architectural state, so swapping [`Cpu`] contexts under it (threads, Mixed Mode) is fine.
pub struct Exec {
    pub tier: Tier,
    pub prof: Prof,
    census: Option<Box<Census>>,
    natives: Option<Box<NativeProfile>>,
    cache: Option<Box<Cache>>,
    lockstep: LockstepStats,
    divergence: Option<Divergence>,
}

/// Per native address: (calls, time inside), timed by a host-supplied clock.
pub struct NativeProfile { clock: fn() -> u64, pub by_addr: BTreeMap<u32, (u64, u64)> }

/// The default tier is tier 1: it passed the gates (docs/PPC-SPEED.md: the differential oracle on every tier, lockstep
/// on every workload with zero divergence, the planted stale-code mutant RED, the speed floor). Tier 0 stays the
/// reference, one `Exec::new(Tier::Interp)` away.
impl Default for Exec { fn default() -> Self { Exec::new(Tier::Blocks) } }

impl Exec {
    pub fn new(tier: Tier) -> Exec { Exec { tier, prof: Prof::default(), census: None, natives: None, cache: None, lockstep: LockstepStats::default(), divergence: None } }
    /// Collect an opcode/block census from now on (tier 0 runs it; it costs a table update per instruction).
    pub fn enable_census(&mut self) { self.census.get_or_insert_with(Box::default); }
    pub fn census(&self) -> Option<&Census> { self.census.as_deref() }
    /// Time every native hook with `clock` (any monotonic unit) and keep a per-address histogram. Costs two clock
    /// reads per native call, nothing per instruction.
    pub fn profile_natives(&mut self, clock: fn() -> u64) { self.natives = Some(Box::new(NativeProfile { clock, by_addr: BTreeMap::new() })); }
    pub fn native_profile(&self) -> Option<&NativeProfile> { self.natives.as_deref() }
    /// Tier 1's cache counters (zero before tier 1 ran).
    pub fn cache_stats(&self) -> CacheStats { self.cache.as_ref().map(|c| c.stats).unwrap_or_default() }
    /// What lockstep compared so far.
    pub fn lockstep_stats(&self) -> LockstepStats { self.lockstep }
    /// The divergence that stopped (halted) a lockstep run, if one did.
    pub fn divergence(&self) -> Option<&Divergence> { self.divergence.as_ref() }
    /// NEGATIVE CONTROL ONLY: plant the stale-code mutant (tier 1 never invalidates a block when its code page is
    /// written). The planted-mutant tests and gates must go RED with it.
    #[doc(hidden)]
    pub fn plant_stale_code_mutant(&mut self) { self.cache_mut().ignore_generations = true; }
    fn cache_mut(&mut self) -> &mut Cache { self.cache.get_or_insert_with(|| Box::new(Cache::new())) }

    /// Execute `cpu` until [`Cpu::halt`], [`Cpu::block`], a trap, or `budget` instructions: [`crate::resume`] on the
    /// selected tier. A census or native profile runs on tier 0 whatever the tier. A lockstep divergence HALTS the CPU
    /// (the run is over; every further call returns at once) and [`Exec::divergence`] says why.
    pub fn resume<H: Host>(&mut self, host: &mut H, cpu: &mut Cpu, budget: u64) -> RunStats {
        cpu.clear_blocked();
        let st = if self.census.is_some() || self.natives.is_some() {
            interp_census(host, cpu, budget, &mut self.prof, self.census.as_deref_mut(), self.natives.as_deref_mut())
        } else {
            match self.tier {
                Tier::Interp => interp(host, cpu, budget, &mut self.prof),
                Tier::Blocks => {
                    let cache = self.cache.get_or_insert_with(|| Box::new(Cache::new()));
                    t1::run(host, cpu, budget, &mut self.prof, cache)
                }
                Tier::Lockstep => {
                    if self.divergence.is_some() { cpu.halt(); return RunStats { insns: 0, halted: true, blocked: false, trap: None }; }
                    let cache = self.cache.get_or_insert_with(|| Box::new(Cache::new()));
                    let (mut st, d) = t1::lockstep(host, cpu, budget, &mut self.prof, cache, &mut self.lockstep);
                    if d.is_some() { cpu.halt(); st.halted = true; }
                    self.divergence = d;
                    st
                }
            }
        };
        self.prof.insns += st.insns;
        st
    }
}

fn interp<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64, prof: &mut Prof) -> RunStats {
    let mut n = 0u64;
    let mut trap = None;
    while n < budget && !cpu.halted() && !cpu.blocked() {
        match exec::step(host, cpu) {
            Ok(native) => prof.natives += native as u64,
            Err(t) => { trap = Some(t); break; }
        }
        n += 1;
    }
    RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }
}

/// Tier 0 with a census and/or native timing (both opt-in; the plain loop above is the product path).
fn interp_census<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64, prof: &mut Prof, mut c: Option<&mut Census>, mut np: Option<&mut NativeProfile>) -> RunStats {
    let mut n = 0u64;
    let mut trap = None;
    while n < budget && !cpu.halted() && !cpu.blocked() {
        let pc = cpu.pc;
        let is_native = cpu.is_native(pc);
        let word = if is_native || c.is_none() { None } else { host.mem().read_u32(pc).ok() };
        let t0 = match (&np, is_native) { (Some(p), true) => (p.clock)(), _ => 0 };
        let r = exec::step(host, cpu);
        if let (Some(p), true) = (np.as_deref_mut(), is_native) {
            let dt = (p.clock)().wrapping_sub(t0);
            let e = p.by_addr.entry(pc).or_default();
            e.0 += 1;
            e.1 += dt;
            prof.native_time += dt;
        }
        match r {
            Ok(native) => {
                prof.natives += native as u64;
                if let Some(c) = c.as_deref_mut() {
                    c.natives += native as u64;
                    match word { Some(w) if !native => { c.insn(pc, w); c.after(pc, Some(w), cpu.pc) } _ => c.close() }
                }
            }
            Err(t) => { trap = Some(t); if let Some(c) = c.as_deref_mut() { c.close(); } break; }
        }
        n += 1;
    }
    RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }
}
