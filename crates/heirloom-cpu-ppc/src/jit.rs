//! Tier 2: the x86-64 JIT's runtime - the dispatcher, the code cache, chaining, the helpers JIT code calls, and the
//! JIT-against-tier-0 lockstep. Block boundaries, decode, page generations and volatile pages are tier 1's (t1cache); a
//! block is compiled the [`HOT_AFTER`]-th time tier 1 runs it whole, and published (RX) at the end of the run slice
//! (jit_mem: code is never written once executable). Until then, and for anything the JIT does not take (a budget-cut
//! block, `sc`, tracing on, a volatile page), tier 1 runs.
//!
//! Chaining without patching: a constant exit jumps through its own RW link slot (initially the exit stub, repointed when
//! its target is published), an indirect one (bclr/bcctr) probes the dispatch table; either way the target block's entry
//! re-checks the budget and its page generation and bails to the dispatcher if either fails, so nothing is unlinked when
//! code goes stale. Plan: /build/patches/h86/ppc-fast-tier-plan.md (Argon's ruling 2026-11-02).
use crate::cpu::{Cpu, RunStats, Trap};
use crate::jit_asm::*;
use crate::jit_emit::{self, Env};
use crate::jit_mem::{Arena, CodeMem};
use crate::jit_stats::JitStats;
use crate::t1::{self, Divergence, LockstepStats, Next};
use crate::t1cache::Cache;
use crate::t1op::K;
use crate::t1run::{self, Flow};
use crate::tier::Prof;
use crate::Host;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;
use core::mem::offset_of;

pub(crate) const EXIT_NORMAL: u32 = 0;
pub(crate) const EXIT_TRAP: u32 = 2;
pub(crate) const EXIT_BAIL: u32 = 3;
/// Whole-block tier-1 runs before a block is compiled (a new constant of this tier; tuned only by measurement).
pub(crate) const HOT_AFTER: u32 = 2;
/// Code arena per guest machine: a cache, dropped whole and refilled when full (never a refusal).
pub(crate) const ARENA_BYTES: usize = 32 << 20;
const TABLE_BITS: u32 = 16;
const LINKS: usize = 1 << 18;
/// Publish when this much code waits (most of a page), or when the oldest waiting block has waited this many guest
/// instructions (across runs). Every publish rounds up to a whole page that is never written again, so publishing a few
/// blocks per run slice filled the 16 MiB arena with mostly empty pages (WC3: an arena flush every ~15k blocks, measured).
const PUBLISH_BYTES: usize = 3072;
pub(crate) const PUBLISH_AFTER: u64 = 2_000_000;
const EMPTY_PC: u32 = 1; // never a PowerPC instruction address (not word aligned)

/// One dispatch-table entry: a published block (32 bytes; jit_emit's indirect probe shifts by its size).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Slot { pub(crate) pc: u32, pub(crate) len: u32, pub(crate) entry: u64, pub(crate) gen: u64, pad: u64 }
const EMPTY: Slot = Slot { pc: EMPTY_PC, len: 0, entry: 0, gen: 0, pad: 0 };

/// What JIT code reads through r13, and what its helpers find. Rebuilt before every entry.
#[repr(C)]
pub(crate) struct Ctx {
    pub(crate) cpu: *mut Cpu,
    pub(crate) ram: *mut u8,
    pub(crate) len: u64,
    pub(crate) mask: u32,
    pad: u32,
    pub(crate) marked: *const u64,
    pub(crate) budget: u64,
    pub(crate) retired: u64,
    pub(crate) tb0: u64,
    pub(crate) host: *mut u8,
    pub(crate) table: *const Slot,
    pub(crate) exit: u64,
    /// The code watch's generation array (block entries compare their page's element).
    pub(crate) gen: *const u64,
    /// `len - n` for the n-byte accesses: an access at a (masked) EA is inside RAM iff EA <= lim_n.
    pub(crate) lim1: u64,
    pub(crate) lim2: u64,
    pub(crate) lim4: u64,
    pub(crate) lim8: u64,
    /// The shared slow stub, and `h_one::<H>` it calls.
    pub(crate) slow: u64,
    pub(crate) helper: u64,
    pub(crate) trap: Option<Trap>,
    /// Opt-in census of the instructions JIT code hands to h_one (key: primary opcode and extended opcode bits).
    pub(crate) census: Option<BTreeMap<u32, u64>>,
}


pub(crate) struct Jit {
    arena: Arena,
    enter: u64,
    exit_stub: u64,
    table: Box<[Slot]>,
    links: Box<[u64]>,
    nlinks: usize,
    by_target: BTreeMap<u32, Vec<u32>>,
    /// Every published block by pc (the dispatch table is a direct-mapped cache in front of it: a colliding block is
    /// refilled from here, never recompiled).
    blocks: BTreeMap<u32, Slot>,
    hot: Box<[(u32, u32)]>,
    pending: Vec<Slot>,
    pending_pcs: BTreeSet<u32>,
    /// `clock` when the oldest unpublished block was compiled.
    pending_at: u64,
    /// Guest instructions run on this tier so far (all runs): the age of waiting blocks.
    clock: u64,
    pub(crate) publish_after: u64,
    ctx: Box<Ctx>,
    pub(crate) stats: JitStats,
    seen_flushes: u64,
    gen_base: *mut u64,
    helper: u64,
    pub(crate) logged: bool,
    pub(crate) mutant: u8,
    lock_seed: u32,
    /// The address mask the code was compiled for.
    mask: u32,
    /// Big-endian accesses by MOVBE (CPUID, at creation; Exec::set_jit_movbe(false) forces the BSWAP forms).
    pub(crate) movbe: bool,
    /// The CPU has MOVBE (CPUID once, at creation).
    hw_movbe: bool,
    /// Opt-in clock timing translation (JitStats.translate_time); None = untimed, no clock reads.
    pub(crate) timer: Option<fn() -> u64>,
}

impl Jit {
    pub(crate) fn new(mem: Box<dyn CodeMem>, mutant: u8) -> Option<Jit> {
        let arena = Arena::new(mem, ARENA_BYTES)?;
        let ctx = Box::new(Ctx { cpu: core::ptr::null_mut(), ram: core::ptr::null_mut(), len: 0, mask: 0, pad: 0, marked: core::ptr::null(), budget: 0, retired: 0, tb0: 0, host: core::ptr::null_mut(), table: core::ptr::null(), exit: 0, gen: core::ptr::null(), lim1: 0, lim2: 0, lim4: 0, lim8: 0, slow: 0, helper: 0, trap: None, census: None });
        let mut j = Jit {
            arena, enter: 0, exit_stub: 0, table: vec![EMPTY; 1 << TABLE_BITS].into_boxed_slice(), links: vec![0; LINKS].into_boxed_slice(),
            nlinks: 1, by_target: BTreeMap::new(), blocks: BTreeMap::new(), hot: vec![(EMPTY_PC, 0); 1 << TABLE_BITS].into_boxed_slice(), pending: Vec::new(),
            pending_pcs: BTreeSet::new(), pending_at: 0, clock: 0, publish_after: PUBLISH_AFTER, ctx, stats: JitStats::default(), seen_flushes: 0, gen_base: core::ptr::null_mut(), helper: 0,
            logged: false, mutant, lock_seed: 0x2545_F491, mask: u32::MAX, movbe: false, hw_movbe: crate::jit_asm::cpu_has_movbe(), timer: None,
        };
        if !j.stubs() { return None; }
        Some(j)
    }

    /// The trampoline (`enter(ctx, entry) -> status`), the common exit and the exit stub, published at the arena's start.
    fn stubs(&mut self) -> bool {
        let mut a = Asm::default();
        for r in [RBX, RBP, R12, R13, R14, R15] { a.push(r); }
        a.alu_ri64(Alu::Sub, RSP, 8);
        a.mov_rr64(R13, RDI);
        a.ld64(RBX, m(R13, offset_of!(Ctx, cpu) as i32));
        a.ld64(R12, m(R13, offset_of!(Ctx, ram) as i32));
        a.ld64(R14, m(R13, offset_of!(Ctx, budget) as i32));
        a.alu_rr(Alu::Xor, R15, R15);
        a.jmp_r(RSI);
        let exit = a.pos();
        a.st64(m(R13, offset_of!(Ctx, retired) as i32), R15);
        a.alu_ri64(Alu::Add, RSP, 8);
        for r in [R15, R14, R13, R12, RBP, RBX] { a.pop(r); }
        a.ret();
        let stub = a.pos();
        a.mov_ri(RAX, EXIT_NORMAL);
        a.jmp_m(m(R13, offset_of!(Ctx, exit) as i32));
        // the shared slow stub: called from a block with esi = word, edx = pc, ecx = block-relative index i; runs h_one and
        // returns to the block (answer 0) or leaves with i (trap) or i + 1 (anything else) instructions of it retired
        let slow = a.pos();
        a.mov_rr(RBP, RCX);
        a.mov_rr64(RDI, R13);
        a.lea64(RCX, mx(R15, RBP, 1));
        a.alu_ri64(Alu::Sub, RSP, 8);
        a.call_m(m(R13, offset_of!(Ctx, helper) as i32));
        a.alu_ri64(Alu::Add, RSP, 8);
        a.test_rr(RAX, RAX);
        let out = a.jcc(Cc::Ne);
        a.ret();
        a.here(out);
        a.alu_ri64(Alu::Add, RSP, 8);
        a.alu_ri(Alu::Cmp, RAX, 2);
        let t = a.jcc(Cc::E);
        a.lea64(R15, mxd(R15, RBP, 1, if self.mutant == crate::jit_emit::MUT_EXIT_COUNT { 2 } else { 1 }));
        a.mov_ri(RAX, EXIT_NORMAL);
        a.jmp_m(m(R13, offset_of!(Ctx, exit) as i32));
        a.here(t);
        a.alu_rr64(Alu::Add, R15, RBP);
        a.mov_ri(RAX, EXIT_TRAP);
        a.jmp_m(m(R13, offset_of!(Ctx, exit) as i32));
        let Some(p) = self.arena.put(&a.b) else { return false };
        self.enter = p as u64;
        self.ctx.exit = p as u64 + exit as u64;
        self.exit_stub = p as u64 + stub as u64;
        self.ctx.slow = p as u64 + slow as u64;
        self.links[0] = self.exit_stub;
        self.arena.publish()
    }

    /// Change how code is made (lockstep journaling, a planted mutant, MOVBE): everything compiled is dropped.
    pub(crate) fn reconfigure(&mut self, logged: bool, mutant: u8, movbe: bool) {
        let movbe = movbe && self.hw_movbe;
        if (logged, mutant, movbe) == (self.logged, self.mutant, self.movbe) { return; }
        self.logged = logged;
        self.mutant = mutant;
        self.movbe = movbe;
        self.flush();
    }

    /// Drop every block and start again (the arena back to RW, the stubs re-emitted).
    fn flush(&mut self) {
        self.stats.flushes += 1;
        self.table.fill(EMPTY);
        self.hot.fill((EMPTY_PC, 0));
        self.links.fill(0);
        self.nlinks = 1;
        self.by_target.clear();
        self.blocks.clear();
        self.pending.clear();
        self.pending_pcs.clear();
        self.arena.reset();
        self.stubs();
    }

    /// Before a run: drop everything compiled against another natives set, another RAM's watch arrays or address mask, or
    /// another host type.
    fn sync<H: Host>(&mut self, cache: &Cache, raw: &heirloom_mem_flat32::RawRam) {
        let helper = h_one::<H> as HOne as usize as u64;
        if cache.stats.flushes != self.seen_flushes || raw.gen != self.gen_base || helper != self.helper || raw.mask != self.mask {
            if self.stats.compiled != 0 { self.flush(); }
            self.seen_flushes = cache.stats.flushes;
            self.gen_base = raw.gen;
            self.helper = helper;
            self.mask = raw.mask;
        }
    }

    /// The published block at `pc`, if its page is still at its generation and it fits in `left` instructions.
    #[inline(always)]
    fn entry(&mut self, pc: u32, gen_now: u64, left: u64) -> Option<(u64, u32)> {
        let i = (pc >> 2) as usize & ((1 << TABLE_BITS) - 1);
        let mut s = self.table[i];
        if s.pc != pc {
            s = *self.blocks.get(&pc)?;
            self.table[i] = s;
        }
        (s.gen == gen_now && s.len as u64 <= left).then_some((s.entry, s.len))
    }

    /// Count a whole-block tier-1 run at `pc`; true when it should be compiled now.
    fn hot(&mut self, pc: u32, gen_now: u64) -> bool {
        let h = &mut self.hot[(pc >> 2) as usize & ((1 << TABLE_BITS) - 1)];
        if h.0 != pc { *h = (pc, 1); return HOT_AFTER <= 1; }
        h.1 = h.1.saturating_add(1);
        if h.1 < HOT_AFTER || self.pending_pcs.contains(&pc) { return false; }
        !self.blocks.get(&pc).is_some_and(|s| s.gen == gen_now)
    }

    fn compile<H: Host>(&mut self, ops: &[crate::t1op::Op], pc: u32, gen: u64, page: usize, at: u64) {
        let t0 = self.timer.map(|c| c());
        self.compile_untimed::<H>(ops, pc, gen, page, at);
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.stats.translate_time += c().wrapping_sub(t0); }
    }

    fn compile_untimed<H: Host>(&mut self, ops: &[crate::t1op::Op], pc: u32, gen: u64, page: usize, at: u64) {
        let (blocks, links, by_target) = (&self.blocks, &mut self.links, &mut self.by_target);
        let nlinks = &mut self.nlinks;
        let exit_stub = self.exit_stub;
        let mut link = |target: u32| -> *const u64 {
            if *nlinks == links.len() { return &links[0] as *const u64; }
            let i = *nlinks;
            *nlinks += 1;
            links[i] = blocks.get(&target).map_or(exit_stub, |s| s.entry);
            by_target.entry(target).or_default().push(i as u32);
            &links[i] as *const u64
        };
        let mut env = Env { note: h_note::<H> as HNote as usize as u64, mask: self.mask, link: &mut link, chain: true, logged: self.logged, mutant: self.mutant, table_mask: (1 << TABLE_BITS) - 1, movbe: self.movbe };
        let code = jit_emit::compile(ops, pc, gen, page, &mut env);
        if self.nlinks == self.links.len() { self.flush(); return; }
        let Some(p) = self.arena.put(&code) else { self.stats.arena_full += 1; self.flush(); return };
        self.stats.compiled += 1;
        self.stats.movbe = self.movbe;
        if self.blocks.contains_key(&pc) { self.stats.recompiled += 1; }
        self.stats.code_bytes = self.arena.used() as u64;
        if self.pending.is_empty() { self.pending_at = self.clock + at; }
        self.pending.push(Slot { pc, len: ops.len() as u32, entry: p as u64, gen, pad: 0 });
        self.pending_pcs.insert(pc);
    }

    /// Count, per instruction form, what JIT code hands to tier 1's handler (diagnostics; a map update per helper call).
    pub(crate) fn census(&mut self) -> &mut Option<BTreeMap<u32, u64>> { &mut self.ctx.census }

    /// Is waiting code due to be published (`n` = instructions run so far in this run)?
    #[inline(always)]
    fn due(&self, n: u64) -> bool {
        self.arena.pending() && (self.arena.unpublished() >= PUBLISH_BYTES || (self.clock + n).saturating_sub(self.pending_at) >= self.publish_after)
    }

    /// Publish what was compiled during this run and make it reachable (dispatch table + every link slot naming it).
    fn publish(&mut self) {
        if !self.arena.pending() { return; }
        let t0 = self.timer.map(|c| c());
        self.publish_untimed();
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.stats.translate_time += c().wrapping_sub(t0); }
    }

    fn publish_untimed(&mut self) {
        if !self.arena.publish() { self.flush(); return; }
        for s in core::mem::take(&mut self.pending) {
            self.table[(s.pc >> 2) as usize & ((1 << TABLE_BITS) - 1)] = s;
            self.blocks.insert(s.pc, s);
            if let Some(v) = self.by_target.get(&s.pc) { for &i in v { self.links[i as usize] = s.entry; } }
        }
        self.pending_pcs.clear();
    }

    /// Run JIT code from `entry` for at most `budget` instructions: (exit status, instructions retired).
    fn run_code<H: Host>(&mut self, host: &mut H, cpu: &mut Cpu, entry: u64, budget: u64) -> (u32, u64) {
        let raw = host.mem().raw();
        let c = &mut *self.ctx;
        c.cpu = cpu as *mut Cpu;
        c.ram = raw.base;
        c.len = raw.len as u64;
        c.mask = raw.mask;
        c.marked = raw.marked;
        c.gen = raw.gen;
        c.lim1 = raw.len as u64 - 1;
        c.lim2 = raw.len as u64 - 2;
        c.lim4 = raw.len as u64 - 4;
        c.lim8 = raw.len as u64 - 8;
        c.helper = h_one::<H> as HOne as usize as u64;
        c.budget = budget;
        c.retired = 0;
        c.tb0 = cpu.tb;
        c.host = host as *mut H as *mut u8;
        c.table = self.table.as_ptr();
        c.trap = None;
        // SAFETY: `enter` is the trampoline this Jit emitted and published (RX) at the arena's start, with the System V
        // signature extern "C" fn(*mut Ctx, u64) -> u32; `entry` is a published block's entry in the same arena. The
        // generated code touches only the Cpu, the Ctx, guest RAM inside `len` bytes at `ram` (bounds-checked per access)
        // and calls h_one / h_note with the Ctx - all live for this call; `cpu` and `host` are not used by Rust meanwhile.
        let f: extern "C" fn(*mut Ctx, u64) -> u32 = unsafe { core::mem::transmute(self.enter as usize) };
        let st = f(&mut *self.ctx, entry);
        let r = self.ctx.retired;
        cpu.tb = self.ctx.tb0.wrapping_add(r);
        self.stats.entries += 1;
        self.stats.insns += r;
        if st == EXIT_BAIL { self.stats.bails += 1; }
        (st, r)
    }

    /// A small pseudo-random budget for lockstep entries (chaining must stop at arbitrary points too).
    fn lock_budget(&mut self, len: u32, left: u64) -> u64 {
        self.lock_seed ^= self.lock_seed << 13;
        self.lock_seed ^= self.lock_seed >> 17;
        self.lock_seed ^= self.lock_seed << 5;
        (len as u64 + (self.lock_seed % 64) as u64).min(left)
    }
}

type HOne = extern "C" fn(*mut Ctx, u32, u32, u64) -> u32;
type HNote = extern "C" fn(*mut Ctx, u32, u32);

/// JIT code's call for ONE instruction it has no inline code for (or whose inline guard failed): tier 1's handler, with the
/// exact pc and time base. Answers 0 = continue the block, 1 = leave (pc set: a branch, a halt or block), 2 = trap (in the
/// Ctx, pc at the instruction), 3 = leave after it (a watched code page was written: the rest of the block may be stale).
pub(crate) extern "C" fn h_one<H: Host>(ctx: *mut Ctx, w: u32, ipc: u32, done: u64) -> u32 {
    // SAFETY: JIT code passes the Ctx `run_code` filled for this entry; its `host` and `cpu` point at the live `&mut H` and
    // `&mut Cpu` of that call, which Rust does not use while JIT code runs.
    let c = unsafe { &mut *ctx };
    let (host, cpu) = unsafe { (&mut *(c.host as *mut H), &mut *c.cpu) };
    if let Some(m) = c.census.as_mut() { *m.entry(w & 0xFC00_07FE).or_default() += 1; }
    let op = crate::t1op::decode(ipc, w);
    let bumps = host.mem().code_bumps();
    match t1run::one(host, cpu, &op, ipc, c.tb0.wrapping_add(done)) {
        Ok(Flow::Next) => {
            if cpu.halted() || cpu.blocked() || host.mem().code_bumps() != bumps { cpu.pc = ipc.wrapping_add(4); return 3; }
            0
        }
        Ok(Flow::Jump(t)) => { cpu.pc = t; 1 }
        Err(t) => { cpu.pc = ipc; c.trap = Some(t); 2 }
    }
}

/// Lockstep only: journal the old bytes of an inline store before JIT code writes them.
pub(crate) extern "C" fn h_note<H: Host>(ctx: *mut Ctx, ea: u32, n: u32) {
    // SAFETY: as in h_one.
    let c = unsafe { &mut *ctx };
    let host = unsafe { &mut *(c.host as *mut H) };
    host.mem().journal_note(ea, n);
}

/// Tier 2 for `budget` instructions; with `ls` (JitLockstep) every JIT entry is checked against tier 0 (tier 0's result
/// kept) and the first divergence stops the run.
pub(crate) fn run<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64, prof: &mut Prof, cache: &mut Cache, jit: &mut Jit, mut ls: Option<&mut LockstepStats>) -> (RunStats, Option<Divergence>) {
    let mut n = 0u64;
    let mut trap = None;
    cache.sync_natives(cpu);
    let raw = host.mem().raw();
    jit.sync::<H>(cache, &raw);
    let check = !cache.ignore_generations;
    // the inline 8-byte accesses rely on a masked address + 8 never wrapping inside RAM
    let usable = !cpu.tracing() && raw.len >= 8 && raw.len as u64 <= raw.mask as u64 + 1;
    if !usable { jit.stats.refused = true; }
    while n < budget && !cpu.halted() && !cpu.blocked() {
        let pc = cpu.pc;
        let left = budget - n;
        // publish a page's worth of new blocks, or anything compiled a while ago (a long run must not wait for its end)
        if jit.due(n) { jit.publish(); }
        if usable {
            if let Some((entry, len)) = jit.entry(pc, host.mem().code_gen(pc), left) {
                let (st, r, t, what) = match ls.as_deref_mut() {
                    None => {
                        let (st, r) = jit.run_code(host, cpu, entry, left);
                        (st, r, if st == EXIT_TRAP { jit.ctx.trap.take() } else { None }, Vec::new())
                    }
                    Some(ls) => {
                        let k = jit.lock_budget(len, left);
                        let (n0, t0, what) = t1::compare(host, cpu, ls, |host, cpu| {
                            let (st, r) = jit.run_code(host, cpu, entry, k);
                            (r, if st == EXIT_TRAP { jit.ctx.trap.take() } else { None })
                        });
                        (if n0 == 0 { EXIT_BAIL } else { EXIT_NORMAL }, n0, t0, what)
                    }
                };
                n += r;
                prof.blocks += 1;
                if !what.is_empty() {
                    return (RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap: t }, Some(Divergence { block: pc, what }));
                }
                if let Some(t) = t { trap = Some(t); break; }
                if st != EXIT_BAIL || r != 0 { continue; }
            }
        }
        // tier 1 for one block (t1::run's loop body), counting it towards compilation
        let (start, len, gen) = match cache.hit(host.mem(), pc) {
            Some(b) => b,
            None => match t1::next(cache, host.mem(), cpu) {
                Err(t) => { trap = Some(t); break; }
                Ok(Next::Native) => {
                    if let Err(t) = t1::native(host, cpu, cache, prof) { trap = Some(t); break; }
                    n += 1;
                    jit.sync::<H>(cache, &host.mem().raw());
                    continue;
                }
                Ok(Next::Interp) => {
                    n += t1::interp_page(host, cpu, left, prof, cache, &mut trap);
                    if trap.is_some() { break; }
                    continue;
                }
                Ok(Next::Block { start, len, gen }) => (start, len, gen),
            },
        };
        let take = len.min(left as usize);
        let host_call = cache.ops[start].k == K::Sc;
        if usable && take == len && !host_call && jit.hot(pc, gen) {
            let page = (host.mem().strip(pc) >> heirloom_mem_flat32::CODE_PAGE_SHIFT) as usize;
            // a block's entry reads its page's generation: only pages inside the watch's array are compiled
            if page < host.mem().raw().gen_pages as usize { jit.compile::<H>(&cache.ops[start..start + len], pc, gen, page, n); }
        }
        n += t1run::run(host, cpu, &cache.ops[start..start + take], pc, gen, check, &mut trap);
        prof.blocks += 1;
        if host_call { cache.sync_natives(cpu); jit.sync::<H>(cache, &host.mem().raw()); }
        if trap.is_some() { break; }
    }
    if jit.due(n) { jit.publish(); }
    jit.clock += n;
    jit.stats.code_bytes = jit.arena.used() as u64;
    (RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }, None)
}
