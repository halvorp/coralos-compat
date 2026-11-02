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

pub use heirloom_cpu_abi::ppc::{Prof, Tier};

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
    /// Tier 2: the executable memory the caller gave (`set_code_mem`), turned into the JIT on its first run.
    #[cfg(target_arch = "x86_64")]
    code_mem: Option<Box<dyn crate::jit_mem::CodeMem>>,
    #[cfg(target_arch = "x86_64")]
    jit: Option<Box<crate::jit::Jit>>,
    jit_mutant: u8,
    jit_refused: bool,
    jit_publish_after: Option<u64>,
}

/// Per native address: (calls, time inside), timed by a host-supplied clock.
pub struct NativeProfile { clock: fn() -> u64, pub by_addr: BTreeMap<u32, (u64, u64)> }

/// The default tier is tier 1: it passed the gates (docs/PPC-SPEED.md: the differential oracle on every tier, lockstep
/// on every workload with zero divergence, the planted stale-code mutant RED, the speed floor). Tier 0 stays the
/// reference, one `Exec::new(Tier::Interp)` away.
impl Default for Exec { fn default() -> Self { Exec::new(Tier::Blocks) } }

impl Exec {
    pub fn new(tier: Tier) -> Exec {
        Exec {
            tier, prof: Prof::default(), census: None, natives: None, cache: None, lockstep: LockstepStats::default(), divergence: None,
            #[cfg(target_arch = "x86_64")]
            code_mem: None,
            #[cfg(target_arch = "x86_64")]
            jit: None,
            jit_mutant: 0, jit_refused: false, jit_publish_after: None,
        }
    }
    /// Executable memory for [`Tier::Jit`] / [`Tier::JitLockstep`] (the caller holds the authority for it; this crate makes
    /// no syscall). Replaces any JIT made from earlier memory.
    #[cfg(target_arch = "x86_64")]
    pub fn set_code_mem(&mut self, mem: Box<dyn crate::jit_mem::CodeMem>) { self.code_mem = Some(mem); self.jit = None; }
    /// Tier 2's counters; `refused` = Tier::Jit was asked for but tier 1 ran (no executable memory, it was refused, off
    /// x86-64, or the trace ring is on).
    pub fn jit_stats(&self) -> crate::JitStats {
        #[cfg(target_arch = "x86_64")]
        let s = self.jit.as_ref().map(|j| j.stats).unwrap_or_default();
        #[cfg(not(target_arch = "x86_64"))]
        let s = crate::JitStats::default();
        crate::JitStats { refused: s.refused || self.jit_refused, ..s }
    }
    /// Publish new JIT code after it waited `n` guest instructions (default 2M, or once most of a page is written). Tests
    /// set 0 so that short programs run in JIT code at all.
    #[doc(hidden)]
    pub fn set_jit_publish_after(&mut self, n: u64) {
        self.jit_publish_after = Some(n);
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.publish_after = n; }
    }
    /// Diagnostics: count the instruction forms JIT code hands to tier 1's handler (key = word & 0xFC0007FE: primary and
    /// extended opcode). `None` before the JIT ran or when not enabled.
    #[cfg(target_arch = "x86_64")]
    pub fn jit_helper_census(&mut self, enable: bool) -> Option<&alloc::collections::BTreeMap<u32, u64>> {
        let j = self.jit.as_mut()?;
        let c = j.census();
        if enable && c.is_none() { *c = Some(Default::default()); }
        c.as_ref()
    }
    /// NEGATIVE CONTROL ONLY: plant JIT mutant `m` (1 subfic CA inverted, 2 no XER[SO] in CR, 3 no code-watch test on
    /// stores, 4 one instruction too many retired at a helper exit, 5 single-precision FP arithmetic not rounded to single, 6 sraw's count > 31 path dropped). The JIT gates must go RED with each.
    #[doc(hidden)]
    pub fn plant_jit_mutant(&mut self, m: u8) {
        self.jit_mutant = m;
        #[cfg(target_arch = "x86_64")]
        { self.jit = None; }
    }
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
                Tier::Jit | Tier::JitLockstep => {
                    if self.divergence.is_some() { cpu.halt(); return RunStats { insns: 0, halted: true, blocked: false, trap: None }; }
                    self.run_jit(host, cpu, budget)
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

impl Exec {
    /// Tier 2 (or its lockstep), or tier 1 with `jit_refused` when there is no JIT to run.
    fn run_jit<H: Host>(&mut self, host: &mut H, cpu: &mut Cpu, budget: u64) -> RunStats {
        let lock = self.tier == Tier::JitLockstep;
        #[cfg(target_arch = "x86_64")]
        {
            if self.jit.as_ref().is_some_and(|j| j.logged != lock) { self.jit = None; }
            if self.jit.is_none() {
                if let Some(mem) = self.code_mem.take() {
                    self.jit = crate::jit::Jit::new(mem, self.jit_mutant).map(Box::new);
                    if let Some(j) = self.jit.as_mut() {
                        j.logged = lock;
                        if let Some(n) = self.jit_publish_after { j.publish_after = n; }
                    }
                }
            }
            if let Some(jit) = self.jit.as_mut() {
                let cache = self.cache.get_or_insert_with(|| Box::new(Cache::new()));
                let ls = if lock { Some(&mut self.lockstep) } else { None };
                let (mut st, d) = crate::jit::run(host, cpu, budget, &mut self.prof, cache, jit, ls);
                if d.is_some() { cpu.halt(); st.halted = true; }
                self.divergence = d;
                return st;
            }
        }
        self.jit_refused = true;
        let cache = self.cache.get_or_insert_with(|| Box::new(Cache::new()));
        t1::run(host, cpu, budget, &mut self.prof, cache)
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
