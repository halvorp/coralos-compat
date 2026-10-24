//! Tier 1's run loop, and the LOCKSTEP mode that checks it against tier 0 block by block.
//!
//! Lockstep runs every pure-guest block twice from the same state: tier 1 with guest RAM journaled, then (state and
//! RAM rolled back) tier 0 for the same number of instructions, and compares every register, the time base, the
//! reservation, the trap, and every byte either run wrote. Tier 0's result is the one kept. Native hooks and `sc`
//! (which call the host and cannot be rolled back) run once, on tier 0's path: both tiers share that code.
use crate::cpu::{Cpu, RunStats, Trap};
use crate::t1cache::{Cache, Found};
use crate::t1run;
use crate::t1op::K;
use crate::tier::Prof;
use crate::{exec, Host};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// The first difference lockstep found between tier 1 and tier 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The block's entry pc.
    pub block: u32,
    /// What differed, e.g. `gpr[5] t1=0x2 t0=0x1`.
    pub what: Vec<String>,
}

/// The architectural state lockstep compares (and rolls back). The MSR is not modelled by this user-mode
/// interpreter (no MSR state exists), so there is nothing of it to diverge.
#[derive(Clone, PartialEq, Eq)]
struct Arch { gpr: [u32; 32], fpr: [u64; 32], cr: u32, lr: u32, ctr: u32, xer: u32, fpscr: u32, pc: u32, tb: u64, reservation: Option<u32> }

impl Arch {
    fn of(c: &Cpu) -> Arch { Arch { gpr: c.gpr, fpr: c.fpr, cr: c.cr, lr: c.lr, ctr: c.ctr, xer: c.xer, fpscr: c.fpscr, pc: c.pc, tb: c.tb, reservation: c.reservation } }
    fn restore(&self, c: &mut Cpu) {
        c.gpr = self.gpr; c.fpr = self.fpr; c.cr = self.cr; c.lr = self.lr; c.ctr = self.ctr; c.xer = self.xer;
        c.fpscr = self.fpscr; c.pc = self.pc; c.tb = self.tb; c.reservation = self.reservation;
    }
    fn diff(&self, o: &Arch, out: &mut Vec<String>) {
        for i in 0..32 { if self.gpr[i] != o.gpr[i] { out.push(alloc::format!("gpr[{i}] t1={:#x} t0={:#x}", self.gpr[i], o.gpr[i])); } }
        for i in 0..32 { if self.fpr[i] != o.fpr[i] { out.push(alloc::format!("fpr[{i}] t1={:#x} t0={:#x}", self.fpr[i], o.fpr[i])); } }
        for (n, a, b) in [("cr", self.cr, o.cr), ("lr", self.lr, o.lr), ("ctr", self.ctr, o.ctr), ("xer", self.xer, o.xer), ("fpscr", self.fpscr, o.fpscr), ("pc", self.pc, o.pc)] {
            if a != b { out.push(alloc::format!("{n} t1={a:#x} t0={b:#x}")); }
        }
        if self.tb != o.tb { out.push(alloc::format!("tb t1={} t0={}", self.tb, o.tb)); }
        if self.reservation != o.reservation { out.push(alloc::format!("reservation t1={:?} t0={:?}", self.reservation, o.reservation)); }
    }
}

/// Lockstep's tally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LockstepStats { pub blocks: u64, pub insns: u64, pub bytes_compared: u64 }

/// Where the next instructions come from: a native hook (tier 0's path), a volatile page (tier 0), or a block.
enum Next { Native, Interp, Block { start: usize, len: usize, gen: u64 } }

fn next(cache: &mut Cache, mem: &mut retro_guest_mem::GuestMem, cpu: &mut Cpu) -> Result<Next, Trap> {
    let pc = cpu.pc;
    match cache.find(mem, pc) {
        Ok(Found::Native) => Ok(Next::Native),
        Ok(Found::Interp) => Ok(Next::Interp),
        Ok(Found::Block { start, len, gen }) => Ok(Next::Block { start, len, gen }),
        Err(t) => { cpu.note_pc(pc); cpu.redirected = false; Err(t) }
    }
}

fn native<H: Host>(host: &mut H, cpu: &mut Cpu, cache: &mut Cache, prof: &mut Prof) -> Result<(), Trap> {
    let pc = cpu.pc;
    cpu.note_pc(pc);
    prof.natives += 1;
    let r = exec::native(host, cpu, pc);
    cache.sync_natives(cpu);
    r
}

/// Tier 0 while pc stays on the volatile page it is on (natives included: `step` handles them), up to `budget`.
fn interp_page<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64, prof: &mut Prof, cache: &mut Cache, trap: &mut Option<Trap>) -> u64 {
    let page = cpu.pc >> retro_guest_mem::CODE_PAGE_SHIFT;
    let mut n = 0;
    while n < budget && !cpu.halted() && !cpu.blocked() {
        match exec::step(host, cpu) {
            Ok(native) => prof.natives += native as u64,
            Err(t) => { *trap = Some(t); break; }
        }
        n += 1;
        if cpu.pc >> retro_guest_mem::CODE_PAGE_SHIFT != page { break; }
    }
    cache.stats.interp_insns += n;
    cache.sync_natives(cpu);
    n
}

/// Tier 1: `budget` instructions on decoded blocks.
pub(crate) fn run<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64, prof: &mut Prof, cache: &mut Cache) -> RunStats {
    let mut n = 0u64;
    let mut blocks = 0u64;
    let mut trap = None;
    cache.sync_natives(cpu);
    let check = !cache.ignore_generations;
    while n < budget && !cpu.halted() && !cpu.blocked() {
        let pc = cpu.pc;
        let (start, len, gen) = match cache.hit(host.mem(), pc) {
            Some(b) => b,
            None => match next(cache, host.mem(), cpu) {
                Err(t) => { trap = Some(t); break; }
                Ok(Next::Native) => {
                    if let Err(t) = native(host, cpu, cache, prof) { trap = Some(t); break; }
                    n += 1;
                    continue;
                }
                Ok(Next::Interp) => {
                    n += interp_page(host, cpu, budget - n, prof, cache, &mut trap);
                    if trap.is_some() { break; }
                    continue;
                }
                Ok(Next::Block { start, len, gen }) => (start, len, gen),
            },
        };
        let take = len.min((budget - n) as usize);
        let ops = &cache.ops[start..start + take];
        let host_call = ops[0].k == K::Sc;
        n += t1run::run(host, cpu, ops, pc, gen, check, &mut trap);
        blocks += 1;
        if host_call { cache.sync_natives(cpu); }
        if trap.is_some() { break; }
    }
    prof.blocks += blocks;
    RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }
}

/// Lockstep: tier 1 checked against tier 0 on every pure-guest block. Stops at the first divergence (returned; the
/// CPU and RAM are then tier 0's).
pub(crate) fn lockstep<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64, prof: &mut Prof, cache: &mut Cache, ls: &mut LockstepStats) -> (RunStats, Option<Divergence>) {
    let mut n = 0u64;
    let mut trap = None;
    cache.sync_natives(cpu);
    let check = !cache.ignore_generations;
    while n < budget && !cpu.halted() && !cpu.blocked() {
        match next(cache, host.mem(), cpu) {
            Err(t) => { trap = Some(t); break; }
            Ok(Next::Native) => {
                if let Err(t) = native(host, cpu, cache, prof) { trap = Some(t); break; }
                n += 1;
            }
            Ok(Next::Interp) => {
                // a volatile page: tier 1 itself runs tier 0 there, so there is nothing to compare
                n += interp_page(host, cpu, budget - n, prof, cache, &mut trap);
                if trap.is_some() { break; }
            }
            Ok(Next::Block { start, len, gen }) => {
                let take = len.min((budget - n) as usize);
                let pc = cpu.pc;
                if cache.ops[start].k == K::Sc {
                    // calls the host: run once (tier 0's execute, through the generic handler)
                    n += t1run::run(host, cpu, &cache.ops[start..start + take], pc, gen, check, &mut trap);
                    cache.sync_natives(cpu);
                    if trap.is_some() { break; }
                    continue;
                }
                let before = Arch::of(cpu);
                let trace = cpu.tracing().then(|| cpu.trace_snapshot());
                host.mem().journal_begin();
                let mut trap1 = None;
                let n1 = t1run::run(host, cpu, &cache.ops[start..start + take], pc, gen, check, &mut trap1);
                let j1 = host.mem().journal_end();
                let after1 = Arch::of(cpu);
                let mut t1_bytes: BTreeMap<u32, u8> = BTreeMap::new();
                for &(a, _) in &j1 { t1_bytes.insert(a, host.mem().read_u8(a).unwrap_or(0)); }
                // roll back, then the reference
                host.mem().journal_undo(&j1);
                before.restore(cpu);
                if let Some(t) = &trace { cpu.trace_restore(t); }
                let k = n1 + trap1.is_some() as u64;
                host.mem().journal_begin();
                let (n0, trap0) = tier0(host, cpu, k);
                let j0 = host.mem().journal_end();
                let after0 = Arch::of(cpu);
                ls.blocks += 1;
                ls.insns += n0;
                prof.blocks += 1;
                let mut what = Vec::new();
                if n0 != n1 { what.push(alloc::format!("retired t1={n1} t0={n0}")); }
                if trap0 != trap1 { what.push(alloc::format!("trap t1={trap1:?} t0={trap0:?}")); }
                after1.diff(&after0, &mut what);
                for &(a, old) in &j0 { t1_bytes.entry(a).or_insert(old); }
                for (&a, &v1) in &t1_bytes {
                    ls.bytes_compared += 1;
                    let v0 = host.mem().read_u8(a).unwrap_or(0);
                    if v0 != v1 && what.len() < 24 { what.push(alloc::format!("mem[{a:#x}] t1={v1:#04x} t0={v0:#04x}")); }
                }
                n += n0;
                if !what.is_empty() { return (RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap: trap0 }, Some(Divergence { block: pc, what })); }
                if let Some(t) = trap0 { trap = Some(t); break; }
            }
        }
    }
    (RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }, None)
}

/// Tier 0 for at most `k` instructions: (retired, trap).
fn tier0<H: Host>(host: &mut H, cpu: &mut Cpu, k: u64) -> (u64, Option<Trap>) {
    let mut n = 0;
    while n < k {
        if let Err(t) = exec::step(host, cpu) { return (n, Some(t)); }
        n += 1;
    }
    (n, None)
}
