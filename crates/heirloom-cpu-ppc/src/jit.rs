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
use crate::pc_map::PcMap;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::mem::offset_of;

pub(crate) const EXIT_NORMAL: u32 = 0;
pub(crate) const EXIT_TRAP: u32 = 2;
pub(crate) const EXIT_BAIL: u32 = 3;
/// Whole-block tier-1 runs before a block is compiled (a new constant of this tier; tuned only by measurement).
pub(crate) const HOT_AFTER: u32 = 2;
/// Code arena per guest machine: a cache of [`SEGMENTS`] segments reused in turn (FIFO) when full - the oldest segment's
/// blocks are dropped, never all of them (an arena-wide drop re-translated ~7k blocks inside ONE WC3 gameplay frame: the
/// worst frame of the host run, 22 ms of 36). Never a refusal.
pub(crate) const ARENA_BYTES: usize = 32 << 20;
/// Segments of the arena (2 MiB each at the default size): one is dropped at a time.
pub(crate) const SEGMENTS: usize = 16;
const TABLE_BITS: u32 = 16;
/// Longest superblock (guest instructions): a block ending in a conditional branch goes on through the NOT-taken path into
/// the next tier-1 block on the same code page, the taken edge a side exit (jit_emit). A new constant of this tier (WC3
/// blocks average 3.8 instructions; tuned only by measurement).
pub(crate) const SUPER_MAX: usize = 48;
const LINKS: usize = 1 << 18;
/// Publish when this much code waits (most of a page), or when the oldest waiting block has waited this many guest
/// instructions (across runs). Every publish rounds up to a whole page that is never written again, so publishing a few
/// blocks per run slice filled the 16 MiB arena with mostly empty pages (WC3: an arena flush every ~15k blocks, measured).
const PUBLISH_BYTES: usize = 3072;
pub(crate) const PUBLISH_AFTER: u64 = 2_000_000;
const EMPTY_PC: u32 = 1; // never a PowerPC instruction address (not word aligned)
/// `link_owner` of a slot the block being compiled took (its segment is known once its code is placed).
const LINK_NEW: u8 = 0xFF;

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
    /// `h_native::<H>` (native blocks call it), the natives it ran this entry, and whether one changed what the dispatcher
    /// must resync (the natives set, guest RAM).
    pub(crate) native: u64,
    pub(crate) natives: u64,
    pub(crate) resync: bool,
    /// Planted mutant MUT_NO_RESYNC: h_native never asks for the resync.
    pub(crate) no_resync: bool,
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
    /// Per link slot: 0 free (or never used), [`LINK_NEW`] taken by the block being compiled, else 1 + the arena segment
    /// holding the code that jumps through it (the slot is freed when that segment is reused).
    link_owner: Box<[u8]>,
    free_links: Vec<u32>,
    by_target: BTreeMap<u32, Vec<u32>>,
    /// Every published block by pc (the dispatch table is a direct-mapped cache in front of it: a colliding block is
    /// refilled from here, never recompiled). Hashed: every dispatcher miss probes it, and in WC3 gameplay most misses are
    /// native calls (pcs that are never a block), ~1 per 500 guest instructions.
    blocks: PcMap<Slot>,
    hot: Box<[(u32, u32)]>,
    pending: Vec<Slot>,
    pending_pcs: PcMap<()>,
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
    pub(crate) fn new(mem: Box<dyn CodeMem>, mutant: u8, arena_bytes: usize) -> Option<Jit> {
        let arena = Arena::new(mem, arena_bytes, SEGMENTS)?;
        let ctx = Box::new(Ctx { cpu: core::ptr::null_mut(), ram: core::ptr::null_mut(), len: 0, mask: 0, pad: 0, marked: core::ptr::null(), budget: 0, retired: 0, tb0: 0, host: core::ptr::null_mut(), table: core::ptr::null(), exit: 0, gen: core::ptr::null(), lim1: 0, lim2: 0, lim4: 0, lim8: 0, slow: 0, helper: 0, trap: None, native: 0, natives: 0, resync: false, no_resync: false, census: None });
        let mut j = Jit {
            arena, enter: 0, exit_stub: 0, table: vec![EMPTY; 1 << TABLE_BITS].into_boxed_slice(), links: vec![0; LINKS].into_boxed_slice(),
            nlinks: 1, link_owner: vec![0; LINKS].into_boxed_slice(), free_links: Vec::new(), by_target: BTreeMap::new(), blocks: PcMap::new(EMPTY), hot: vec![(EMPTY_PC, 0); 1 << TABLE_BITS].into_boxed_slice(), pending: Vec::new(),
            pending_pcs: PcMap::new(()), pending_at: 0, clock: 0, publish_after: PUBLISH_AFTER, ctx, stats: JitStats::default(), seen_flushes: 0, gen_base: core::ptr::null_mut(), helper: 0,
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
        // the shared slow stub: called from a block with esi = word, edx = pc, ecx = block-relative index i (kept on the
        // stack across the call: rbp and r14 belong to the register cache); runs h_one and returns to the block (answer 0)
        // or leaves with i (trap) or i + 1 (anything else) instructions of it retired
        let slow = a.pos();
        a.push(RCX);
        a.mov_rr64(RDI, R13);
        a.lea64(RCX, mx(R15, RCX, 1));
        a.call_m(m(R13, offset_of!(Ctx, helper) as i32));
        a.pop(RCX);
        a.test_rr(RAX, RAX);
        let out = a.jcc(Cc::Ne);
        a.ret();
        a.here(out);
        a.alu_ri64(Alu::Add, RSP, 8);
        a.alu_ri(Alu::Cmp, RAX, 2);
        let t = a.jcc(Cc::E);
        a.lea64(R15, mxd(R15, RCX, 1, if self.mutant == crate::jit_emit::MUT_EXIT_COUNT { 2 } else { 1 }));
        a.mov_ri(RAX, EXIT_NORMAL);
        a.jmp_m(m(R13, offset_of!(Ctx, exit) as i32));
        a.here(t);
        a.alu_rr64(Alu::Add, R15, RCX);
        a.mov_ri(RAX, EXIT_TRAP);
        a.jmp_m(m(R13, offset_of!(Ctx, exit) as i32));
        let Some(p) = self.arena.put(&a.b) else { return false };
        self.enter = p as u64;
        self.ctx.exit = p as u64 + exit as u64;
        self.exit_stub = p as u64 + stub as u64;
        self.ctx.slow = p as u64 + slow as u64;
        self.links[0] = self.exit_stub;
        let ok = self.arena.publish();
        self.arena.begin();
        ok
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
        if self.mutant != crate::jit_emit::MUT_TABLE_STALE { self.table.fill(EMPTY); }
        self.hot.fill((EMPTY_PC, 0));
        self.links.fill(0);
        self.link_owner.fill(0);
        self.free_links.clear();
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
            s = *self.blocks.get(pc)?;
            self.table[i] = s;
        }
        (s.gen == gen_now && s.len as u64 <= left).then_some((s.entry, s.len))
    }

    /// Count a whole-block tier-1 run at `pc`; true when it should be compiled now.
    fn hot(&mut self, pc: u32, gen_now: u64) -> bool {
        let h = &mut self.hot[(pc >> 2) as usize & ((1 << TABLE_BITS) - 1)];
        if h.0 != pc { *h = (pc, 1); return HOT_AFTER <= 1; }
        h.1 = h.1.saturating_add(1);
        if h.1 < HOT_AFTER || self.pending_pcs.contains(pc) { return false; }
        !self.blocks.get(pc).is_some_and(|s| s.gen == gen_now)
    }

    /// Compile `ops` (a superblock whose first tier-1 block is `ops[..first]`) at `pc`.
    fn compile<H: Host>(&mut self, ops: &[crate::t1op::Op], first: usize, pc: u32, gen: u64, page: usize, at: u64) {
        let t0 = self.timer.map(|c| c());
        // a superblock whose code would not fit an empty arena segment is compiled as its first block alone (a block that
        // big would only ever flush the whole cache)
        if !self.compile_untimed::<H>(ops, pc, gen, page, at, first < ops.len()) { self.compile_untimed::<H>(&ops[..first], pc, gen, page, at, false); }
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.stats.translate_time += c().wrapping_sub(t0); }
    }

    /// False (nothing compiled, its link slots given back) when `may_refuse` and the code is longer than a segment.
    fn compile_untimed<H: Host>(&mut self, ops: &[crate::t1op::Op], pc: u32, gen: u64, page: usize, at: u64, may_refuse: bool) -> bool {
        let (blocks, links, by_target, owner, free) = (&self.blocks, &mut self.links, &mut self.by_target, &mut self.link_owner, &mut self.free_links);
        let nlinks = &mut self.nlinks;
        let exit_stub = self.exit_stub;
        let mut out_of_links = false;
        let mut taken: Vec<(u32, u32)> = Vec::new();
        let mut link = |target: u32| -> *const u64 {
            let i = match free.pop() {
                Some(i) => i as usize,
                None if *nlinks < links.len() => { *nlinks += 1; *nlinks - 1 }
                None => { out_of_links = true; return &links[0] as *const u64; }
            };
            owner[i] = LINK_NEW;
            taken.push((i as u32, target));
            links[i] = blocks.get(target).map_or(exit_stub, |s| s.entry);
            by_target.entry(target).or_default().push(i as u32);
            &links[i] as *const u64
        };
        let mut env = Env { note: h_note::<H> as HNote as usize as u64, mask: self.mask, link: &mut link, chain: true, logged: self.logged, mutant: self.mutant, table_mask: (1 << TABLE_BITS) - 1, movbe: self.movbe };
        let (code, pinned) = jit_emit::compile(ops, pc, gen, page, &mut env);
        if out_of_links { self.flush(); return true; }
        if may_refuse && code.len() > self.arena.segment_bytes() {
            for (i, target) in taken {
                self.link_owner[i as usize] = 0;
                self.links[i as usize] = self.exit_stub;
                self.free_links.push(i);
                if let Some(v) = self.by_target.get_mut(&target) { v.retain(|&j| j != i); }
            }
            return false;
        }
        self.place(&code, taken.iter().map(|t| t.0).collect(), pc, ops.len() as u32, gen, pinned, at);
        true
    }

    /// Put compiled code in the arena (reusing the oldest segment when full) and queue it for publishing; `taken` = the
    /// link slots its code jumps through.
    fn place(&mut self, code: &[u8], taken: Vec<u32>, pc: u32, len: u32, gen: u64, pinned: usize, at: u64) {
        let p = match self.arena.put(code) {
            Some(p) => p,
            None => {
                // the current segment is full: publish it, drop the next one's blocks, write there
                self.publish_untimed();
                self.evict(self.arena.next_segment());
                if !self.arena.reuse() { self.flush(); return; }
                match self.arena.put(code) { Some(p) => p, None => { self.stats.arena_full += 1; self.flush(); return } }
            }
        };
        let seg = self.arena.segment_of(p as u64).map_or(0, |s| s as u8 + 1);
        for i in taken { self.link_owner[i as usize] = seg; }
        self.stats.compiled += 1;
        self.stats.pinned += pinned as u64;
        self.stats.movbe = self.movbe;
        if self.blocks.contains(pc) { self.stats.recompiled += 1; }
        self.stats.code_bytes = self.arena.used() as u64;
        if self.pending.is_empty() { self.pending_at = self.clock + at; }
        self.pending.push(Slot { pc, len, entry: p as u64, gen, pad: 0 });
        if !self.pending_pcs.insert(pc, ()) { self.flush(); }
    }

    /// Compile the block of the bound native at `pc` (jit_emit::compile_native); `gen` is its page's generation now (only
    /// so the dispatcher's entry test reads the same way for every block: the code does not depend on the page).
    fn compile_native(&mut self, pc: u32, gen: u64, at: u64) {
        let t0 = self.timer.map(|c| c());
        let mut none = |_: u32| -> *const u64 { core::ptr::null() };
        let mut env = Env { note: 0, mask: self.mask, link: &mut none, chain: true, logged: false, mutant: self.mutant, table_mask: (1 << TABLE_BITS) - 1, movbe: self.movbe };
        let code = jit_emit::compile_native(pc, &mut env);
        self.place(&code, Vec::new(), pc, 1, gen, 0, at);
        self.stats.natives_compiled += 1;
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.stats.translate_time += c().wrapping_sub(t0); }
    }

    /// Drop every block whose code is in arena segment `s` (about to be reused): out of the block map and the dispatch
    /// table, every link slot that jumps to one back to the exit stub, and the link slots their own code jumped through
    /// freed. Nothing can enter the segment afterwards, so its pages may go back to RW. (Runs between JIT entries only.)
    fn evict(&mut self, s: usize) {
        let (a, b) = self.arena.segment(s);
        let inside = |e: u64| e >= a && e < b;
        let mut gone = Vec::new();
        let stale = self.mutant == crate::jit_emit::MUT_EVICT_STALE;
        self.blocks.retain(|pc, slot| { let k = inside(slot.entry); if k { gone.push(pc); } !k || stale });
        if !stale { for t in self.table.iter_mut() { if inside(t.entry) { *t = EMPTY; } } }
        for pc in &gone {
            if let Some(v) = self.by_target.get(pc) { for &i in v { self.links[i as usize] = self.exit_stub; } }
        }
        let tag = s as u8 + 1;
        for i in 1..self.nlinks {
            if self.link_owner[i] == tag { self.link_owner[i] = 0; self.links[i] = self.exit_stub; self.free_links.push(i as u32); }
        }
        let owner = &self.link_owner;
        self.by_target.retain(|_, v| { v.retain(|&i| owner[i as usize] != 0); !v.is_empty() });
        self.stats.evictions += 1;
        self.stats.evicted += gone.len() as u64;
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
        let mut grown = true;
        for s in core::mem::take(&mut self.pending) {
            if !self.blocks.insert(s.pc, s) { grown = false; break; }
            self.table[(s.pc >> 2) as usize & ((1 << TABLE_BITS) - 1)] = s;
            if let Some(v) = self.by_target.get(&s.pc) { for &i in v { self.links[i as usize] = s.entry; } }
        }
        self.pending_pcs.clear();
        // the block map could not grow: drop the code (a block not in the map would never be refilled into the table)
        if !grown { self.flush(); }
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
        c.native = h_native::<H> as HNative as usize as u64;
        c.natives = 0;
        c.resync = false;
        c.no_resync = self.mutant == crate::jit_emit::MUT_NO_RESYNC;
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
type HNative = extern "C" fn(*mut Ctx, u32, u64) -> u32;

/// A native block's call: the host's native bound at `pc` with `done` instructions of this entry retired before it, as
/// tier 1's dispatcher runs it (t1::native): the time base exact, the pc noted, one retired instruction. Answers 0 = go on
/// at cpu.pc, 1 = leave after it (halted or blocked, or the natives set or guest RAM changed: `resync` tells the
/// dispatcher), 2 = trap (in the Ctx; pc at the native, nothing retired). A native that sets the time base itself moves
/// `tb0`, so the exit's `tb0 + retired` is what it wrote plus what ran after.
pub(crate) extern "C" fn h_native<H: Host>(ctx: *mut Ctx, pc: u32, done: u64) -> u32 {
    // SAFETY: as in h_one.
    let c = unsafe { &mut *ctx };
    let (host, cpu) = unsafe { (&mut *(c.host as *mut H), &mut *c.cpu) };
    let natives_gen = cpu.natives_gen;
    cpu.tb = c.tb0.wrapping_add(done);
    cpu.note_pc(pc);
    c.natives += 1;
    if let Err(t) = crate::exec::native(host, cpu, pc) { cpu.pc = pc; c.trap = Some(t); return 2; }
    if cpu.tb != c.tb0.wrapping_add(done + 1) { c.tb0 = cpu.tb.wrapping_sub(done + 1); }
    let raw = host.mem().raw();
    if c.no_resync { return 0; }
    if cpu.natives_gen != natives_gen || raw.base != c.ram || raw.len as u64 != c.len || raw.mask != c.mask || raw.gen as *const u64 != c.gen || raw.marked as *const u64 != c.marked {
        c.resync = true;
        return 1;
    }
    if cpu.halted() || cpu.blocked() { 1 } else { 0 }
}

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

/// A superblock: the tier-1 block `first` at `pc` and, while its last instruction is a FORWARD conditional branch (not
/// branch-always) whose next instruction is on the same code page, the blocks of the not-taken path after it
/// (t1cache::Cache::peek: the same cuts, the same page generation), up to [`SUPER_MAX`] instructions: contiguous code, one
/// page, so one generation check covers it, and the taken edge is a side exit. A BACKWARD branch is usually a loop's back
/// edge, taken, its fall-through the loop's exit: cold code that would only be duplicated. (Tracing through `b` / `bl` too
/// was measured on WC3: 0.6% fewer JIT host instructions than this for 38% more compiles and 2x the evictions.)
fn superblock(cache: &Cache, mem: &heirloom_mem_flat32::GuestMem, first: &[crate::t1op::Op], pc: u32, gen: u64, page: usize) -> Vec<crate::t1op::Op> {
    let mut ops = first.to_vec();
    while let Some(last) = ops.last() {
        let next = pc.wrapping_add(4 * ops.len() as u32);
        let forward = matches!(last.k, K::Bc | K::Bcl) && last.d & 0x14 != 0x14 && last.imm.wrapping_sub(next) < 0x8000_0000;
        if !forward || ops.len() >= SUPER_MAX || (mem.strip(next) >> heirloom_mem_flat32::CODE_PAGE_SHIFT) as usize != page { break; }
        if !cache.peek(mem, next, gen, SUPER_MAX - ops.len(), &mut ops) { break; }
    }
    ops
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
    let tracing = cpu.tracing();
    let usable_for = |raw: &heirloom_mem_flat32::RawRam| !tracing && raw.len >= 8 && raw.len as u64 <= raw.mask as u64 + 1;
    let mut usable = usable_for(&raw);
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
                        prof.natives += jit.ctx.natives;
                        jit.stats.natives += jit.ctx.natives;
                        if jit.ctx.resync {
                            // a native in JIT code changed the natives set or guest RAM (t1::native's sync, done here)
                            cache.sync_natives(cpu);
                            let raw = host.mem().raw();
                            jit.sync::<H>(cache, &raw);
                            usable = usable_for(&raw);
                        }
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
                    // natives run inside JIT code once hot (never in lockstep: a native cannot be rolled back and run twice)
                    if usable && !jit.logged {
                        let gen = host.mem().code_gen(pc);
                        if jit.hot(pc, gen) { jit.compile_native(pc, gen, n); }
                    }
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
            if page < host.mem().raw().gen_pages as usize {
                let ops = superblock(cache, host.mem(), &cache.ops[start..start + len], pc, gen, page);
                jit.stats.superblocks += (ops.len() > len) as u64;
                jit.compile::<H>(&ops, len, pc, gen, page, n);
            }
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
