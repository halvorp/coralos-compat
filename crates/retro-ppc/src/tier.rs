//! The execution tiers behind one entry point, [`Exec::resume`], and the counters a profile line is made of.
//!
//! Tier 0 is the reference interpreter ([`crate::resume`]: fetch, decode, execute one instruction at a time). Every
//! other tier must leave the SAME architectural state at every exit: same registers, memory, time base, trap and pc.
use crate::census::Census;
use crate::cpu::{Cpu, RunStats};
use crate::{exec, Host};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;

/// Which interpreter [`Exec::resume`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Tier 0: the reference, one instruction at a time.
    Interp,
}

impl Tier {
    /// The number a `[PPC-PROF ... tier=]` line prints.
    pub fn number(self) -> u32 { match self { Tier::Interp => 0 } }
    /// Parse `0`; None for anything else.
    pub fn parse(s: &str) -> Option<Tier> { match s { "0" | "interp" => Some(Tier::Interp), _ => None } }
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
}

/// Per native address: (calls, time inside), timed by a host-supplied clock.
pub struct NativeProfile { clock: fn() -> u64, pub by_addr: BTreeMap<u32, (u64, u64)> }

impl Default for Exec { fn default() -> Self { Exec::new(Tier::Interp) } }

impl Exec {
    pub fn new(tier: Tier) -> Exec { Exec { tier, prof: Prof::default(), census: None, natives: None } }
    /// Collect an opcode/block census from now on (tier 0 runs it; it costs a table update per instruction).
    pub fn enable_census(&mut self) { self.census.get_or_insert_with(Box::default); }
    pub fn census(&self) -> Option<&Census> { self.census.as_deref() }
    /// Time every native hook with `clock` (any monotonic unit) and keep a per-address histogram. Costs two clock
    /// reads per native call, nothing per instruction.
    pub fn profile_natives(&mut self, clock: fn() -> u64) { self.natives = Some(Box::new(NativeProfile { clock, by_addr: BTreeMap::new() })); }
    pub fn native_profile(&self) -> Option<&NativeProfile> { self.natives.as_deref() }

    /// Execute `cpu` until [`Cpu::halt`], [`Cpu::block`], a trap, or `budget` instructions: [`crate::resume`] on the
    /// selected tier.
    pub fn resume<H: Host>(&mut self, host: &mut H, cpu: &mut Cpu, budget: u64) -> RunStats {
        cpu.clear_blocked();
        let st = if self.census.is_some() || self.natives.is_some() {
            interp_census(host, cpu, budget, &mut self.prof, self.census.as_deref_mut(), self.natives.as_deref_mut())
        } else {
            interp(host, cpu, budget, &mut self.prof)
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
