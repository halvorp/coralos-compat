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
    /// Tier 2's ablation knobs (heirloom_cpu_abi::ppc::KNOB_*): measurement settings only.
    jit_knobs: u32,
    jit_refused: bool,
    jit_publish_after: Option<u64>,
    jit_movbe: Option<bool>,
    jit_clock: Option<fn() -> u64>,
    jit_arena: Option<usize>,
    jit_region_after: Option<u32>,
    jit_cold: Option<(u64, u32)>,
    jit_region_shape: Option<crate::RegionShape>,
    jit_region_census: bool,
    jit_tags: bool,
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
            jit_mutant: 0, jit_knobs: 0, jit_refused: false, jit_publish_after: None, jit_movbe: None, jit_clock: None, jit_arena: None, jit_region_after: None, jit_cold: None, jit_region_shape: None, jit_region_census: false, jit_tags: false,
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
    /// Block entries after which a compiled block's head forms a REGION (default jit_region::REGION_AFTER = 64; 0 = no regions).
    /// Takes effect for blocks compiled after the call (a JIT made later gets it at creation).
    #[doc(hidden)]
    /// The cold-tail rule's window and minimum (jit_cold; tuning and tests - the product sets it with KNOB_COLD_TAIL).
    #[doc(hidden)]
    pub fn set_jit_cold(&mut self, window: u64, min: u32) {
        self.jit_cold = Some((window, min));
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.cold = self.jit_cold; }
    }
    pub fn set_jit_region_after(&mut self, n: u32) {
        self.jit_region_after = Some(n);
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.set_region_after(n); }
    }
    /// The region tier's shape (jit_region::RegionShape; default = its Default). Tuning only.
    #[doc(hidden)]
    pub fn set_jit_region_shape(&mut self, s: crate::RegionShape) {
        self.jit_region_shape = Some(s);
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.region_shape = s; }
    }
    /// Diagnostics: regions compiled from now on count their entries and internal edges (JitStats::region_entries / _edges).
    #[doc(hidden)]
    pub fn set_jit_region_census(&mut self, on: bool) {
        self.jit_region_census = on;
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.region_census = on; }
    }
    /// Diagnostics (jit_churn): classify every block compiled from now on by why it is compiled (`on`), or stop; the counts
    /// by cause and the (pc, cause) log so far. Only once tier 2 exists.
    #[doc(hidden)]
    pub fn set_jit_churn(&mut self, on: bool) {
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.churn.on = on; }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = on;
    }
    /// Diagnostics (jit_churn): zero the counts and the log, keep the history (where a measured window starts).
    #[doc(hidden)]
    pub fn jit_churn_reset(&mut self) {
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.churn.reset(); }
    }
    #[doc(hidden)]
    pub fn jit_churn(&self) -> ([u64; 6], &[(u32, u8)], &[(u8, u64)], &[(u32, bool)]) {
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_ref() { return (j.churn.counts, &j.churn.log, &j.churn.drops, &j.churn.regions); }
        ([0; 6], &[], &[], &[])
    }
    /// Per region compiled while a clock was set (`set_jit_clock`), its translation time in that clock's units.
    #[doc(hidden)]
    pub fn jit_region_times(&self) -> &[u64] {
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_ref() { return &j.region_times; }
        &[]
    }
    /// Diagnostics: tier 2's code arena as it stands (its base address and every byte, published or not), for a host tool
    /// that maps profiler addresses to the code there. None before the JIT ran.
    #[cfg(target_arch = "x86_64")]
    #[doc(hidden)]
    pub fn jit_arena_bytes(&self) -> Option<(u64, &[u8])> {
        let j = self.jit.as_ref()?;
        // SAFETY: the reservation is readable in every state (RW or R+X) for the arena's lifetime, which `&self` spans.
        Some((j.arena.base as u64, unsafe { core::slice::from_raw_parts(j.arena.base, j.arena.len) }))
    }
    /// Diagnostics: keep a tag per tier-2 code byte saying what it is for (jit_census: the guest form's K << 8 | a category),
    /// for a host tool that joins it with a profiler's per-address counts. Code compiled before the call stays untagged (0).
    #[cfg(target_arch = "x86_64")]
    #[doc(hidden)]
    pub fn set_jit_tag_census(&mut self, on: bool) {
        self.jit_tags = on;
        if let Some(j) = self.jit.as_mut() { if on { j.start_tagmap() } else { j.tagmap = None } }
    }
    /// Diagnostics: the tag map (one u16 per byte of `jit_arena_bytes`), when `set_jit_tag_census(true)`.
    #[cfg(target_arch = "x86_64")]
    #[doc(hidden)]
    pub fn jit_arena_tags(&self) -> Option<&[u16]> { self.jit.as_ref()?.tagmap.as_deref() }
    /// Diagnostics: with the tag map, (arena offset, guest word, guest pc) where each placed guest instruction's code starts.
    #[cfg(target_arch = "x86_64")]
    #[doc(hidden)]
    pub fn jit_arena_words(&self) -> &[(u64, u32, u32)] { self.jit.as_ref().map_or(&[], |j| &j.tag_words) }
    /// With the tag census: every unit placed (arena offset, bytes, head pc, kind: 0 block, 1 superblock, 2 region, 3 shape).
    #[cfg(target_arch = "x86_64")]
    #[doc(hidden)]
    pub fn jit_arena_units(&self) -> &[(u64, u32, u32, u8)] { self.jit.as_ref().map_or(&[], |j| &j.units) }
    /// Tests: the code arena's size in bytes (default 32 MiB: the stub page and 16 segments), so a test can make segments
    /// fill and be reused. Takes effect for a JIT made after the call.
    #[doc(hidden)]
    pub fn set_jit_arena(&mut self, bytes: usize) { self.jit_arena = Some(bytes); }
    /// Diagnostics: count the instruction forms JIT code hands to tier 1's handler (key = word & 0xFC0007FE: primary and
    /// extended opcode). `None` before the JIT ran or when not enabled.
    #[cfg(target_arch = "x86_64")]
    pub fn jit_helper_census(&mut self, enable: bool) -> Option<&alloc::collections::BTreeMap<u32, u64>> {
        let j = self.jit.as_mut()?;
        let c = j.census();
        if enable && c.is_none() { *c = Some(Default::default()); }
        c.as_ref()
    }
    /// Time tier 2's translation (compile, publish, flush) with `clock` (any monotonic unit) into
    /// `JitStats::translate_time`: two clock reads per compiled block and per published batch, nothing per instruction.
    pub fn set_jit_clock(&mut self, clock: fn() -> u64) {
        self.jit_clock = Some(clock);
        #[cfg(target_arch = "x86_64")]
        if let Some(j) = self.jit.as_mut() { j.timer = Some(clock); }
    }
    /// Tests: false forces the BSWAP forms even on a CPU with MOVBE (both encodings must agree with tier 0). Takes effect
    /// for a JIT made after the call.
    #[doc(hidden)]
    pub fn set_jit_movbe(&mut self, on: bool) {
        self.jit_movbe = Some(on);
    }
    /// NEGATIVE CONTROL ONLY: plant JIT mutant `m` - one of the `MUT_*` constants of jit_emit.rs (1-25: e.g. 3 no code-watch
    /// test on stores, 8 / 9 the global pins not loaded after a helper / not stored at the exit, 19 a store map that stops
    /// mirroring the watch), each documented there. The JIT gates must go RED with each.
    #[doc(hidden)]
    /// Tier 2's ABLATION knobs (`heirloom_cpu_abi::ppc::KNOB_*`, the Z83's guard-cost A/B): a change drops the compiled code.
    pub fn set_jit_knobs(&mut self, knobs: u32) {
        self.jit_knobs = knobs;
    }
    /// The ablation knobs set (what the next run compiles under).
    #[doc(hidden)]
    pub fn jit_knobs(&self) -> u32 { self.jit_knobs }
    pub fn plant_jit_mutant(&mut self, m: u8) {
        self.jit_mutant = m;
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
                Tier::Jit | Tier::JitLockstep | Tier::JitBlocks | Tier::JitWide => {
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
            if self.jit.is_none() {
                if let Some(mem) = self.code_mem.take() {
                    self.jit = crate::jit::Jit::new(mem, self.jit_mutant, self.jit_arena.unwrap_or(crate::jit::ARENA_BYTES)).map(Box::new);
                    if let (Some(j), Some(n)) = (self.jit.as_mut(), self.jit_publish_after) { j.publish_after = n; }
                    if let Some(j) = self.jit.as_mut() { j.timer = self.jit_clock; }
                    if let (Some(j), Some(n)) = (self.jit.as_mut(), self.jit_region_after) { j.set_region_after(n); }
                    if let Some(j) = self.jit.as_mut() { j.cold = self.jit_cold; }
                    if let (Some(j), Some(s)) = (self.jit.as_mut(), self.jit_region_shape) { j.region_shape = s; }
                    if let Some(j) = self.jit.as_mut() { j.region_census = self.jit_region_census; }
                    if let Some(j) = self.jit.as_mut().filter(|_| self.jit_tags) { j.start_tagmap(); }
                }
            }
            // lockstep journaling, a planted mutant or MOVBE changed since the code was made: it is dropped and remade
            if let Some(j) = self.jit.as_mut() {
                j.reconfigure(lock, self.jit_mutant, self.jit_movbe != Some(false), self.tier == Tier::JitWide, self.jit_knobs);
                // Tier::JitBlocks: no new regions (those already formed stay until evicted or replaced)
                let after = if self.tier == Tier::JitBlocks { 0 } else { self.jit_region_after.unwrap_or(crate::jit::REGION_AFTER) };
                if j.region_after != after { j.set_region_after(after); }
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
