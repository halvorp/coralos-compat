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
//!
//! REGIONS (jit_region, council step 4): every compiled block counts its entries in the Ctx; when a head's count runs out the
//! dispatcher forms a region from it and compiles it as one more publish unit, which replaces the block at that pc in the
//! dispatch table and every link slot naming it. JitLockstep compares at every JIT EXIT (whatever ran in between: blocks,
//! regions, chained jumps), with budgets that stop a region anywhere - so regions are verified as they run in production.
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
pub(crate) use crate::jit_region::REGION_AFTER;
/// Code arena per guest machine: a cache of [`SEGMENTS`] segments reused in turn (FIFO) when full - the oldest segment's
/// blocks are dropped, never all of them (an arena-wide drop re-translated ~7k blocks inside ONE WC3 gameplay frame: the
/// worst frame of the host run, 22 ms of 36). Never a refusal.
/// 64 MiB (CoralOS's exec window per address space), halved down to [`ARENA_MIN`] while the provider refuses to reserve it
/// (EXEC_CODE_RESERVE allocates the whole reservation as contiguous frames: a large one can fail where a smaller one would
/// not). Measured on WC3 (h86 2026-11-03): at 32 MiB a gameplay frame recompiled 1,288 blocks after an eviction dropped a
/// segment of hot code - the hot code is spread over every segment, so neither FIFO nor a second-chance clock (also measured:
/// -31% gameplay recompiles, the same bursts, +1.2% JIT instructions for its per-entry reference store) avoids it; at 64 MiB
/// the gameplay working set and the loading code it replaces both fit: no burst over 150 blocks, gameplay compiles 8,344 ->
/// 3,257, worst frame 59 -> 46 ms.
pub(crate) const ARENA_BYTES: usize = 64 << 20;
/// The smallest arena the JIT reserves before it gives up (tier 2 is then refused by name, tier 1 runs).
pub(crate) const ARENA_MIN: usize = 8 << 20;
/// Segments of the arena (4 MiB each at the default size): one is dropped at a time.
pub(crate) const SEGMENTS: usize = 16;
const TABLE_BITS: u32 = 14;
/// The INDIRECT BRANCH TARGET CACHE that blr / bctr / a native's return probe (jit_emit `probe`): 64K direct-mapped entries
/// of 8 bytes (guest pc, entry offset in the arena) - 512 KiB, the dispatch table's size, 4x its entries. Z83 2026-11-03
/// (tag blue forest peak): with the probe on the 16K-entry dispatch table (4bf4dfb), dispatcher entries rose 4.4x per guest
/// instruction (1 per ~3.1k, IBTC misses ~all of them); host repro 1 per ~2.8k. ppc-jit4's own census: 64K hits 99.6%.
pub(crate) const IBTC_BITS: u32 = 16;
const IBTC_EMPTY: u64 = EMPTY_PC as u64;
/// Longest superblock (guest instructions): a block ending in a conditional branch goes on through the NOT-taken path into
/// the next tier-1 block on the same code page, the taken edge a side exit (jit_emit). A new constant of this tier (WC3
/// blocks average 3.8 instructions; tuned only by measurement).
pub(crate) const SUPER_MAX: usize = 48;
const LINKS: usize = 1 << 18;
/// Publish when this much code waits, or when the oldest waiting block has waited this many guest instructions (across
/// runs); waiting blocks run on tier 1. Every publish is a provider call - on CoralOS one EXEC_CODE_PROTECT syscall with a
/// TLB shootdown round per page - and rounds up to a whole page never written again (the first Z83 run of tier 2 through
/// that provider, publishing a page per 3 KiB batch, crawled at 1/40 of tier 1: h86, 2026-11-02). Eight pages per call.
const PUBLISH_BYTES: usize = 8 * 4096 - 1024;
/// Publish calls refused IN A ROW after which tier 2 stops (refused by name; tier 1 runs): a refused batch is dropped,
/// never run, so a provider that keeps refusing would only make the JIT compile code that never runs.
const PUBLISH_REFUSALS: u32 = 8;
pub(crate) const PUBLISH_AFTER: u64 = 2_000_000;
/// Once the arena has wrapped, a batch smaller than this waits SMALL_WAIT x publish_after before it is published: in steady WC3
/// gameplay single blocks
/// trickle in (~60 per 100M guest instructions) and each publish rounded one up to a whole 4 KiB page - ~1 page per block,
/// which kept the wrapped 64 MiB arena cycling (Z83 2026-11-03: publish = withdraw pages, 30-60 per 5 s, every window).
const PUBLISH_SMALL: usize = 2048;
const SMALL_WAIT: u64 = 8;
const EMPTY_PC: u32 = 1; // never a PowerPC instruction address (not word aligned)
/// `link_owner` of a slot the block being compiled took (its segment is known once its code is placed).
const LINK_NEW: u8 = 0xFF;

/// One dispatch-table entry: a published block (the dispatcher's; JIT code's indirect branches probe the IBTC instead).
#[repr(C)]
#[derive(Clone, Copy)]
/// `inner` = the entry past the page-generation check: where a chained jump from a block on the SAME code page compiled at
/// the SAME generation lands (that block's own check passed and nothing it does can move the generation without leaving
/// first: a store to a watched page goes through the helper, which leaves when any code page was written). A native
/// block has no generation check: inner = entry.
pub(crate) struct Slot { pub(crate) pc: u32, pub(crate) len: u32, pub(crate) entry: u64, pub(crate) gen: u64, pub(crate) inner: u64 }
const EMPTY: Slot = Slot { pc: EMPTY_PC, len: 0, entry: 0, gen: 0, inner: 0 };
/// `link_gen` of a link slot whose owner is on another page than its target (or that is free): always the full entry.
const ANY_GEN: u64 = u64::MAX;

/// What JIT code reads through r13, and what its helpers find. Rebuilt before every entry.
#[repr(C)]
/// FIELD ORDER (h86 ppc-jit6): what JIT code reads on its hot paths comes first - the page generations, the budget, the common
/// exit, the store map, the four RAM limits and the CR-field constants fill the first 128 bytes, so each is `[r13 + disp8]`
/// (one byte) instead of a 4-byte displacement; WC3's hot JIT code read crk at 0x1011c past the 64K-entry counters.
pub(crate) struct Ctx {
    /// The code watch's generation array (block entries compare their page's element).
    pub(crate) gen: *const u64,
    pub(crate) budget: u64,
    pub(crate) exit: u64,
    /// The store map (jit_smap): one byte per guest page, 1 = an inline store may write there.
    pub(crate) smap: *const u8,
    /// `len - n` for the n-byte accesses: an access at a (masked) EA is inside RAM iff EA <= lim_n.
    pub(crate) lim1: u64,
    pub(crate) lim2: u64,
    pub(crate) lim4: u64,
    pub(crate) lim8: u64,
    /// Per CR field f: the field's LT and GT nibbles already in place (8 << s, 4 << s; s = 28 - 4f) - the CMOV sources of
    /// jit_emit's cr_set.
    pub(crate) crk: [[u32; 2]; 8],
    // ---- 128 bytes: below, a 4-byte displacement ----
    /// The IBTC (Jit::ibtc) and the arena base its entry offsets are relative to.
    pub(crate) ibtc: *const u64,
    pub(crate) abase: u64,
    /// Indirect branches (blr / bctr / a native's return) that found their target in the dispatch table, and that did not.
    pub(crate) ibtc_hits: u64,
    pub(crate) ibtc_misses: u64,
    /// The shared code stubs JIT code calls (jit_inline): FPSCR[FPRF] of a single / double result, stfs's bits.
    pub(crate) fprf_s: u64,
    pub(crate) fprf_d: u64,
    pub(crate) stfs: u64,
    /// The shared slow stub, and `h_one::<H>` it calls.
    pub(crate) slow: u64,
    pub(crate) helper: u64,
    /// `h_native::<H>` (native blocks call it), the natives it ran this entry, and whether one changed what the dispatcher
    /// must resync (the natives set, guest RAM).
    pub(crate) native: u64,
    pub(crate) natives: u64,
    /// h_one calls this entry.
    pub(crate) helpers: u64,
    pub(crate) cpu: *mut Cpu,
    pub(crate) ram: *mut u8,
    pub(crate) len: u64,
    pub(crate) mask: u32,
    pad: u32,
    pub(crate) marked: *const u64,
    pub(crate) retired: u64,
    pub(crate) tb0: u64,
    pub(crate) host: *mut u8,
    pub(crate) table: *const Slot,
    pub(crate) trap: Option<Trap>,
    pub(crate) resync: bool,
    /// Planted mutant MUT_NO_RESYNC: h_native never asks for the resync.
    pub(crate) no_resync: bool,
    /// Opt-in census of the instructions JIT code hands to h_one (key: primary opcode and extended opcode bits).
    pub(crate) census: Option<BTreeMap<u32, u64>>,
    /// The head of a block whose entry counter ran out (it left with nothing of it retired): the dispatcher forms a region
    /// there (jit_region). EMPTY_PC = none.
    pub(crate) hot: u32,
    /// Block entry counters, one per dispatch-table index (a colliding block shares one: it only miscounts), counting DOWN
    /// from the region threshold; a block's counter is reset when it is compiled. Inline: JIT code reaches one with a
    /// single `sub [r13 + disp32], 1`.
    pub(crate) counters: [u32; 1 << TABLE_BITS],
    /// Opt-in region census (Exec::set_jit_region_census): region entries and internal edges taken.
    pub(crate) rentries: u64,
    pub(crate) redges: u64,
}


const _: () = assert!(offset_of!(Ctx, crk) + core::mem::size_of::<[[u32; 2]; 8]>() <= 128, "Ctx's hot fields must stay within a one-byte displacement of r13");

pub(crate) struct Jit {
    pub(crate) arena: Arena,
    enter: u64,
    exit_stub: u64,
    table: Box<[Slot]>,
    links: Box<[u64]>,
    nlinks: usize,
    /// Per link slot: 0 free (or never used), [`LINK_NEW`] taken by the block being compiled, else 1 + the arena segment
    /// holding the code that jumps through it (the slot is freed when that segment is reused).
    link_owner: Box<[u8]>,
    free_links: Vec<u32>,
    /// Per link slot: the generation its owner was compiled at when the target is on the owner's own code page (the slot
    /// then holds the target's INNER entry if the target was compiled at that generation too), else [`ANY_GEN`].
    link_gen: Box<[u64]>,
    /// Per link slot: its owner's exit tail (stores the target pc, leaves) - where the slot points while its target is not
    /// published.
    link_home: Box<[u64]>,
    by_target: BTreeMap<u32, Vec<u32>>,
    /// The IBTC: per entry, guest pc (low half) and the block's entry offset in the arena (high half); IBTC_EMPTY = none.
    ibtc: Box<[u64]>,
    /// Every published block by pc (the dispatch table is a direct-mapped cache in front of it: a colliding block is
    /// refilled from here, never recompiled). Hashed: every dispatcher miss probes it, and in WC3 gameplay most misses are
    /// native calls (pcs that are never a block), ~1 per 500 guest instructions.
    blocks: PcMap<Slot>,
    /// Tier-1 whole-block runs per pc (direct-mapped, IBTC_BITS: at 16K entries two hot blocks 64 KiB apart reset each
    /// other's count and never compiled).
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
    /// The ablation knobs the code is compiled with (heirloom_cpu_abi::ppc::KNOB_*).
    pub(crate) knobs: u32,
    lock_seed: u32,
    /// The address mask the code was compiled for.
    mask: u32,
    /// Big-endian accesses by MOVBE (CPUID, at creation; Exec::set_jit_movbe(false) forces the BSWAP forms).
    pub(crate) movbe: bool,
    /// Tier::JitWide: code with every displacement and immediate 4 bytes long (jit_asm::Asm::wide).
    pub(crate) wide: bool,
    /// The CPU has MOVBE (CPUID once, at creation).
    hw_movbe: bool,
    /// Opt-in clock timing translation (JitStats.translate_time); None = untimed, no clock reads.
    pub(crate) timer: Option<fn() -> u64>,
    /// Consecutive refused publishes; and true once the provider failed in a way the JIT cannot work around (a withdraw,
    /// or PUBLISH_REFUSALS publishes in a row): tier 2 is refused from then on (JitStats::refused), tier 1 runs.
    refusals: u32,
    pub(crate) dead: bool,
    /// The store map inline stores test (mirrors the code watch; synced before every entry).
    smap: crate::jit_smap::StoreMap,
    /// Block entries after which a block's head forms a region (jit_region::REGION_AFTER; 0 = no regions).
    pub(crate) region_after: u32,
    pub(crate) region_shape: crate::jit_region::RegionShape,
    /// Regions compiled from now on count their entries and internal edges (diagnostics: two `add [r13+..], 1` more).
    pub(crate) region_census: bool,
    /// Per published or pending REGION (by its entry address): the code pages it was decoded from, (page, generation, a pc
    /// on it) - the dispatcher's entry test reads the head's page only, so a region left stale by a write to another of its
    /// pages is found here (and recompiled as a block) instead of bailing at every entry.
    regions: BTreeMap<u64, Vec<(usize, u64, u32)>>,
    /// Per region compile, the translation time (the `timer` units; only while a timer is set): the compile-time tail.
    pub(crate) region_times: Vec<u64>,
    /// The last emit_place refused code longer than a segment (`may_refuse`).
    refused_last: bool,
}

impl Jit {
    pub(crate) fn new(mem: Box<dyn CodeMem>, mutant: u8, arena_bytes: usize) -> Option<Jit> {
        let arena = Arena::new(mem, arena_bytes, ARENA_MIN.min(arena_bytes), SEGMENTS)?;
        let ctx = Box::new(Ctx { cpu: core::ptr::null_mut(), ram: core::ptr::null_mut(), len: 0, mask: 0, pad: 0, marked: core::ptr::null(), smap: core::ptr::null(), budget: 0, retired: 0, tb0: 0, host: core::ptr::null_mut(), table: core::ptr::null(), exit: 0, gen: core::ptr::null(), lim1: 0, lim2: 0, lim4: 0, lim8: 0, slow: 0, helper: 0, fprf_s: 0, fprf_d: 0, stfs: 0, trap: None, native: 0, natives: 0, helpers: 0, ibtc_hits: 0, ibtc_misses: 0, ibtc: core::ptr::null(), abase: 0, resync: false, no_resync: false, census: None, hot: EMPTY_PC, counters: [crate::jit_region::REGION_AFTER; 1 << TABLE_BITS], crk: core::array::from_fn(|f| [8 << (28 - 4 * f), 4 << (28 - 4 * f)]), rentries: 0, redges: 0 });
        let mut j = Jit {
            arena, enter: 0, exit_stub: 0, table: vec![EMPTY; 1 << TABLE_BITS].into_boxed_slice(), links: vec![0; LINKS].into_boxed_slice(),
            nlinks: 1, link_owner: vec![0; LINKS].into_boxed_slice(), free_links: Vec::new(), link_gen: vec![ANY_GEN; LINKS].into_boxed_slice(), link_home: vec![0; LINKS].into_boxed_slice(), ibtc: vec![IBTC_EMPTY; 1 << IBTC_BITS].into_boxed_slice(), by_target: BTreeMap::new(), blocks: PcMap::new(EMPTY), hot: vec![(EMPTY_PC, 0); 1 << IBTC_BITS].into_boxed_slice(), pending: Vec::new(),
            pending_pcs: PcMap::new(()), pending_at: 0, clock: 0, publish_after: PUBLISH_AFTER, ctx, stats: JitStats::default(), seen_flushes: 0, gen_base: core::ptr::null_mut(), helper: 0,
            logged: false, mutant, lock_seed: 0x2545_F491, mask: u32::MAX, movbe: false, wide: false, hw_movbe: crate::jit_asm::cpu_has_movbe(), timer: None, refusals: 0, dead: false, smap: crate::jit_smap::StoreMap::new(), knobs: 0,
            region_after: crate::jit_region::REGION_AFTER, region_shape: Default::default(), region_census: false, regions: BTreeMap::new(), region_times: Vec::new(), refused_last: false,
        };
        j.stats.arena_bytes = j.arena.len as u64;
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
        a.lea64(RBX, m(RBX, crate::jit_emit::CPU_BIAS));
        a.ld64(R12, m(R13, offset_of!(Ctx, ram) as i32));
        a.alu_rr(Alu::Xor, R15, R15);
        crate::jit_regs::load_globals(&mut a);
        a.jmp_r(RSI);
        let exit = a.pos();
        // every way out of JIT code passes here: the global pins go back to the Cpu
        if self.mutant != crate::jit_emit::MUT_GLOBAL_NO_EXIT_STORE { crate::jit_regs::store_globals(&mut a); }
        a.st64(m(R13, offset_of!(Ctx, retired) as i32), R15);
        a.alu_ri64(Alu::Add, RSP, 8);
        for r in [R15, R14, R13, R12, RBP, RBX] { a.pop(r); }
        a.ret();
        let stub = a.pos();
        a.mov_ri(RAX, EXIT_NORMAL);
        a.jmp_m(m(R13, offset_of!(Ctx, exit) as i32));
        // the shared slow stub: called from a block with esi = word, edx = pc, ecx = block-relative index i (kept on the
        // stack across the call: rbp and r14 hold global pins); runs h_one and returns to the block (answer 0)
        // or leaves with i (trap) or i + 1 (anything else) instructions of it retired
        let slow = a.pos();
        a.push(RCX);
        a.mov_rr64(RDI, R13);
        a.lea64(RCX, mx(R15, RCX, 1));
        a.call_m(m(R13, offset_of!(Ctx, helper) as i32));
        // the handler may have written any field: the global pins come back from the Cpu (rax, the answer, untouched)
        if self.mutant != crate::jit_emit::MUT_GLOBAL_NO_RELOAD { crate::jit_regs::load_globals(&mut a); }
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
        let fprf_s = a.pos();
        crate::jit_inline::emit_fprf(&mut a, true);
        let fprf_d = a.pos();
        crate::jit_inline::emit_fprf(&mut a, false);
        let stfs = a.pos();
        crate::jit_inline::emit_stfs_bits(&mut a);
        let Some(p) = self.arena.put(&a.b) else { return false };
        (self.ctx.fprf_s, self.ctx.fprf_d, self.ctx.stfs) = (p as u64 + fprf_s as u64, p as u64 + fprf_d as u64, p as u64 + stfs as u64);
        self.enter = p as u64;
        self.ctx.exit = p as u64 + exit as u64;
        self.exit_stub = p as u64 + stub as u64;
        self.ctx.slow = p as u64 + slow as u64;
        self.links[0] = self.exit_stub;
        let ok = self.arena.publish();
        self.arena.begin();
        ok
    }

    /// Regions form after `n` block entries (0 = never): every block compiled from now on counts with this threshold.
    pub(crate) fn set_region_after(&mut self, n: u32) {
        self.region_after = n;
        self.ctx.counters.fill(n);
    }

    /// Change how code is made (lockstep journaling, a planted mutant, MOVBE): everything compiled is dropped.
    pub(crate) fn reconfigure(&mut self, logged: bool, mutant: u8, movbe: bool, wide: bool, knobs: u32) {
        let movbe = movbe && self.hw_movbe;
        if (logged, mutant, movbe, wide, knobs) == (self.logged, self.mutant, self.movbe, self.wide, self.knobs) { return; }
        self.knobs = knobs;
        let stubs_change = mutant != self.mutant; // the stubs depend on the mutant only (MUT_EXIT_COUNT)
        self.logged = logged;
        self.mutant = mutant;
        self.movbe = movbe;
        self.wide = wide;
        if stubs_change { self.flush(); } else if self.stats.compiled != 0 { self.drop_code(); }
    }

    /// Drop every block and start again (the arena back to RW, the stubs re-emitted): only when the stubs change.
    fn flush(&mut self) {
        self.drop_code();
        self.link_gen.fill(ANY_GEN);
        self.links.fill(0);
        if !self.arena.reset() { self.dead = true; }
        self.stubs();
    }

    /// Make every block unreachable WITHOUT touching the arena's pages (no provider call: on CoralOS a whole-arena withdraw
    /// is a TLB shootdown per published page): the dispatch table, block map, link slots (to the exit stub) and hot counts
    /// are cleared and unpublished code is forgotten; the old code stays published but nothing jumps to it, and its
    /// segments are withdrawn when the FIFO reuses them, as always. For a natives-set, RAM or link-table change.
    fn drop_code(&mut self) {
        self.stats.flushes += 1;
        if self.mutant != crate::jit_emit::MUT_TABLE_STALE { self.table.fill(EMPTY); self.ibtc.fill(IBTC_EMPTY); }
        self.hot.fill((EMPTY_PC, 0));
        self.links.fill(self.exit_stub);
        self.link_owner.fill(0);
        self.link_gen.fill(ANY_GEN);
        self.free_links.clear();
        self.nlinks = 1;
        self.by_target.clear();
        self.blocks.clear();
        self.pending.clear();
        self.pending_pcs.clear();
        self.regions.clear();
        self.arena.discard();
    }

    /// Before a run: drop everything compiled against another natives set, another RAM's watch arrays or address mask, or
    /// another host type.
    fn sync<H: Host>(&mut self, cache: &Cache, raw: &heirloom_mem_flat32::RawRam) {
        let helper = h_one::<H> as HOne as usize as u64;
        if cache.stats.flushes != self.seen_flushes || raw.gen != self.gen_base || helper != self.helper || raw.mask != self.mask {
            if self.stats.compiled != 0 { self.drop_code(); }
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
            self.ibtc_set(&s);
        }
        (s.gen == gen_now && s.len as u64 <= left).then_some((s.entry, s.len))
    }

    /// The IBTC's index mask: KNOB_IBTC_BITS when set (8..=16), else the full table.
    fn ibtc_mask(&self) -> u32 {
        let b = self.knobs >> heirloom_cpu_abi::ppc::KNOB_IBTC_BITS_SHIFT & heirloom_cpu_abi::ppc::KNOB_IBTC_BITS_MASK;
        if (8..=IBTC_BITS).contains(&b) { (1 << b) - 1 } else { (1 << IBTC_BITS) - 1 }
    }

    /// Point the IBTC at published block `s`.
    #[inline(always)]
    fn ibtc_set(&mut self, s: &Slot) {
        self.ibtc[(s.pc >> 2) as usize & self.ibtc_mask() as usize] = s.pc as u64 | (s.entry - self.arena.base as u64) << 32;
    }

    /// Count a whole-block tier-1 run at `pc`; true when it should be compiled now (also when the published code there is a
    /// region one of whose other pages was written: its entry bails, tier 1 runs, and a block replaces it).
    fn hot(&mut self, pc: u32, gen_now: u64, mem: &heirloom_mem_flat32::GuestMem) -> bool {
        let h = &mut self.hot[(pc >> 2) as usize & ((1 << IBTC_BITS) - 1)];
        if h.0 != pc { *h = (pc, 1); return HOT_AFTER <= 1; }
        h.1 = h.1.saturating_add(1);
        if h.1 < HOT_AFTER || self.pending_pcs.contains(pc) { return false; }
        let regions = &self.regions;
        !self.blocks.get(pc).is_some_and(|s| s.gen == gen_now && regions.get(&s.entry).is_none_or(|p| p.iter().all(|&(_, g, at)| mem.code_gen(at) == g)))
    }

    /// The counter of the block at `pc` (an index into Ctx.counters).
    #[inline(always)]
    fn counter(pc: u32) -> usize { (pc >> 2) as usize & ((1 << TABLE_BITS) - 1) }

    /// Form and compile the region headed by `head` (its block's counter ran out), if two or more nodes qualify.
    fn form_region<H: Host>(&mut self, cache: &Cache, mem: &heirloom_mem_flat32::GuestMem, head: u32, at: u64) {
        let t0 = self.timer.map(|c| c());
        let (blocks, counters, after) = (&self.blocks, &self.ctx.counters, self.region_after);
        let hot = |pc: u32| blocks.contains(pc) && after.wrapping_sub(counters[Self::counter(pc)]) >= crate::jit_region::edge_min(after);
        let Some(plan) = crate::jit_region::form(cache, mem, head, mem.raw().gen_pages as usize, self.region_shape, hot) else { self.stats.region_refused += 1; return };
        if self.region_shape.spend {
            // its other nodes form no region of their own (their counters wrap far from zero; they still read as hot)
            for n in &plan.nodes[1..] { self.ctx.counters[Self::counter(n.0)] = after.wrapping_sub(1 << 31); }
        }
        let pages: Vec<(usize, u64)> = plan.pages.iter().map(|p| (p.0, p.1)).collect();
        let len = plan.maxfwd[0];
        let entry = self.emit_place(head, plan.pages[0].1, &pages, at, len, false, h_note::<H> as HNote as usize as u64, |env| jit_emit::compile_region(&plan.nodes, &plan.maxfwd, &plan.pages, &plan.rets, env));
        if let Some(e) = entry {
            self.regions.insert(e, plan.pages);
            self.stats.regions += 1;
            self.stats.region_nodes += plan.nodes.len() as u64;
            self.stats.region_insns += plan.nodes.iter().map(|n| n.1.len() as u64).sum::<u64>();
            self.stats.region_fused += crate::jit_rflow::fuse_marks(&plan.nodes, &plan.rets, false).iter().flatten().filter(|&&f| f).count() as u64;
        }
        if let (Some(c), Some(t0)) = (self.timer, t0) {
            let d = c().wrapping_sub(t0);
            self.stats.translate_time += d;
            self.region_times.push(d);
        }
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
        let c = Self::counter(pc);
        let counter = (self.region_after != 0).then(|| (offset_of!(Ctx, counters) + 4 * c) as i32);
        self.ctx.counters[c] = self.region_after;
        let note = h_note::<H> as HNote as usize as u64;
        let r = self.emit_place(pc, gen, &[(page, gen)], at, ops.len() as u32, may_refuse, note, |env| {
            env.counter = counter;
            jit_emit::compile(ops, pc, gen, page, env)
        });
        r.is_some() || !self.refused_last
    }

    /// Emit code with `emit` (given the Env: link slots, mask, mutant) and place it as the block or region at `pc` (its
    /// head page's generation `gen`, the code pages it checks `pages`, `len` = the instructions its entry's budget test
    /// covers). None when nothing was placed (`refused_last` = because `may_refuse` and it is longer than a segment).
    fn emit_place(&mut self, pc: u32, gen: u64, pages: &[(usize, u64)], at: u64, len: u32, may_refuse: bool, note: u64, emit: impl FnOnce(&mut Env) -> (Vec<u8>, usize)) -> Option<u64> {
        self.refused_last = false;
        let ibtc_mask = self.ibtc_mask();
        let (blocks, links, by_target, owner, free, link_gen) = (&self.blocks, &mut self.links, &mut self.by_target, &mut self.link_owner, &mut self.free_links, &mut self.link_gen);
        let (mask, mutant) = (self.mask, self.mutant);
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
            let tp = ((target & mask) >> heirloom_mem_flat32::CODE_PAGE_SHIFT) as usize;
            link_gen[i] = pages.iter().find(|p| p.0 == tp).map_or(ANY_GEN, |p| p.1);
            links[i] = blocks.get(target).map_or(exit_stub, |s| landing(s, link_gen[i], mutant));
            by_target.entry(target).or_default().push(i as u32);
            &links[i] as *const u64
        };
        let mut env = Env { note, mask: self.mask, link: &mut link, chain: true, logged: self.logged, mutant: self.mutant, table_mask: if self.mutant == crate::jit_emit::MUT_IBTC_SMALL { (1 << 14) - 1 } else { ibtc_mask }, knobs: self.knobs, movbe: self.movbe, wide: self.wide, counter: None, census: self.region_census, homes: Vec::new() };
        let (code, inner) = emit(&mut env);
        let homes = core::mem::take(&mut env.homes);
        if out_of_links { self.drop_code(); return None; }
        if may_refuse && code.len() > self.arena.segment_bytes() {
            for (i, target) in taken {
                self.link_owner[i as usize] = 0;
                self.link_gen[i as usize] = ANY_GEN;
                self.links[i as usize] = self.exit_stub;
                self.free_links.push(i);
                if let Some(v) = self.by_target.get_mut(&target) { v.retain(|&j| j != i); }
            }
            self.refused_last = true;
            return None;
        }
        let p = self.place(&code, inner, taken.iter().map(|t| t.0).collect(), pc, len, gen, at)?;
        // each slot's home is its exit tail in the placed code; it lands in its target if that is published (after any
        // eviction place() made), else at home
        debug_assert_eq!(homes.len(), taken.len());
        for (&(i, target), &off) in taken.iter().zip(&homes) {
            let i = i as usize;
            self.link_home[i] = p + off as u64;
            self.links[i] = self.blocks.get(target).map_or(self.link_home[i], |s| landing(s, self.link_gen[i], self.mutant));
        }
        Some(p)
    }

    /// Put compiled code in the arena (reusing the oldest segment when full) and queue it for publishing; `taken` = the
    /// link slots its code jumps through.
    fn place(&mut self, code: &[u8], inner: usize, taken: Vec<u32>, pc: u32, len: u32, gen: u64, at: u64) -> Option<u64> {
        let p = match self.arena.put(code) {
            Some(p) => p,
            None if self.arena.withdraw_failed => { self.dead = true; self.drop_code(); return None }
            None => {
                // the current segment is full: publish it, drop the next one's blocks, write there
                self.publish_untimed();
                self.evict(self.arena.next_segment());
                self.arena.reuse();
                match self.arena.put(code) {
                    Some(p) => p,
                    // a withdraw the provider refused leaves pages it may still execute elsewhere: never write them
                    None if self.arena.withdraw_failed => { self.dead = true; self.drop_code(); return None }
                    None => { self.stats.arena_full += 1; self.drop_code(); return None }
                }
            }
        };
        let seg = self.arena.segment_of(p as u64).map_or(0, |s| s as u8 + 1);
        for i in taken { self.link_owner[i as usize] = seg; }
        self.stats.compiled += 1;
        self.stats.movbe = self.movbe;
        if self.blocks.contains(pc) { self.stats.recompiled += 1; }
        self.stats.code_bytes = self.arena.used() as u64;
        self.stats.emitted_bytes = self.arena.put_bytes;
        if self.pending.is_empty() { self.pending_at = self.clock + at; }
        self.pending.push(Slot { pc, len, entry: p as u64, gen, inner: p as u64 + inner as u64 });
        if !self.pending_pcs.insert(pc, ()) { self.drop_code(); return None; }
        Some(p as u64)
    }

    /// Compile the block of the bound native at `pc` (jit_emit::compile_native); `gen` is its page's generation now (only
    /// so the dispatcher's entry test reads the same way for every block: the code does not depend on the page).
    fn compile_native(&mut self, pc: u32, gen: u64, at: u64) {
        let t0 = self.timer.map(|c| c());
        let mut none = |_: u32| -> *const u64 { core::ptr::null() };
        let mut env = Env { note: 0, mask: self.mask, link: &mut none, chain: true, logged: false, mutant: self.mutant, table_mask: if self.mutant == crate::jit_emit::MUT_IBTC_SMALL { (1 << 14) - 1 } else { self.ibtc_mask() }, knobs: self.knobs, movbe: self.movbe, wide: self.wide, counter: None, census: false, homes: Vec::new() };
        let code = jit_emit::compile_native(pc, &mut env);
        self.place(&code, 0, Vec::new(), pc, 1, gen, at);
        self.stats.natives_compiled += 1;
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.stats.translate_time += c().wrapping_sub(t0); }
    }

    /// Drop every block whose code is in arena segment `s` (about to be reused): out of the block map and the dispatch
    /// table, every link slot that jumps to one back to the exit stub, and the link slots their own code jumped through
    /// freed. Nothing can enter the segment afterwards, so its pages may go back to RW. (Runs between JIT entries only.)
    fn evict(&mut self, s: usize) {
        let (a, b) = self.arena.segment(s);
        let inside = |e: u64| e >= a && e < b;
        self.regions.retain(|&e, _| !inside(e));
        let mut gone = Vec::new();
        let stale = self.mutant == crate::jit_emit::MUT_EVICT_STALE;
        self.blocks.retain(|pc, slot| { let k = inside(slot.entry); if k { gone.push(pc); } !k || stale });
        if !stale {
            for t in self.table.iter_mut() { if inside(t.entry) { *t = EMPTY; } }
            let base = self.arena.base as u64;
            for v in self.ibtc.iter_mut() { if *v != IBTC_EMPTY && inside(base + (*v >> 32)) { *v = IBTC_EMPTY; } }
        }
        for pc in &gone {
            if let Some(v) = self.by_target.get(pc) { for &i in v { self.links[i as usize] = self.link_home[i as usize]; } }
        }
        let tag = s as u8 + 1;
        for i in 1..self.nlinks {
            if self.link_owner[i] == tag { self.link_owner[i] = 0; self.link_gen[i] = ANY_GEN; self.links[i] = self.exit_stub; self.free_links.push(i as u32); }
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
        if !self.arena.pending() { return false; }
        let (bytes, age) = (self.arena.unpublished(), (self.clock + n).saturating_sub(self.pending_at));
        // a publish rounds up to a whole page that is never written again: once the arena has wrapped (every page then costs
        // older code), a batch under half a page waits longer
        let small_ok = bytes >= PUBLISH_SMALL || !self.arena.wrapped || age >= SMALL_WAIT * self.publish_after;
        bytes >= PUBLISH_BYTES || (age >= self.publish_after && small_ok)
    }

    /// Publish what was compiled during this run and make it reachable (dispatch table + every link slot naming it).
    fn publish(&mut self) {
        if !self.arena.pending() { return; }
        let t0 = self.timer.map(|c| c());
        self.publish_untimed();
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.stats.translate_time += c().wrapping_sub(t0); }
    }

    fn publish_untimed(&mut self) {
        if !self.arena.publish() {
            if self.mutant == crate::jit_emit::MUT_FLUSH_ON_REFUSAL { self.flush(); return; }
            // refused (on CoralOS: a TLB shootdown not acknowledged in time): this batch never runs - its blocks are dropped
            // (compiled again when hot; nothing was linked to them: link slots name new code only here, after a publish), its
            // pages skipped until their segment is reused. Never a whole-arena reset.
            self.pending.clear();
            self.pending_pcs.clear();
            self.refusals += 1;
            if self.refusals >= PUBLISH_REFUSALS { self.dead = true; }
            return;
        }
        self.refusals = 0;
        let mut grown = true;
        for s in core::mem::take(&mut self.pending) {
            if !self.blocks.insert(s.pc, s) { grown = false; break; }
            self.table[(s.pc >> 2) as usize & ((1 << TABLE_BITS) - 1)] = s;
            self.ibtc_set(&s);
            if let Some(v) = self.by_target.get(&s.pc) { for &i in v { self.links[i as usize] = landing(&s, self.link_gen[i as usize], self.mutant); } }
        }
        self.pending_pcs.clear();
        // the block map could not grow: drop the code (a block not in the map would never be refilled into the table)
        if !grown { self.drop_code(); }
    }

    /// Run JIT code from `entry` for at most `budget` instructions: (exit status, instructions retired).
    fn run_code<H: Host>(&mut self, host: &mut H, cpu: &mut Cpu, entry: u64, budget: u64) -> (u32, u64) {
        let raw = host.mem().raw();
        // the store map first: it must mirror the watch before any inline store runs (no memory for it: tier 1 runs)
        if !self.smap.sync(host.mem(), &raw, self.mutant == crate::jit_emit::MUT_SMAP_STALE) { return (EXIT_BAIL, 0); }
        self.ctx.smap = self.smap.map.as_ptr();
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
        c.ibtc = self.ibtc.as_ptr();
        c.abase = self.arena.base as u64;
        c.trap = None;
        c.native = h_native::<H> as HNative as usize as u64;
        c.natives = 0;
        c.helpers = 0;
        c.ibtc_hits = 0;
        c.ibtc_misses = 0;
        c.resync = false;
        c.no_resync = self.mutant == crate::jit_emit::MUT_NO_RESYNC;
        // SAFETY: `enter` is the trampoline this Jit emitted and published (RX) at the arena's start, with the System V
        // signature extern "C" fn(*mut Ctx, u64) -> u32; `entry` is a published block's entry in the same arena. The
        // generated code touches only the Cpu, the Ctx, guest RAM inside `len` bytes at `ram` (bounds-checked per access)
        // and calls h_one / h_note with the Ctx - all live for this call; `cpu` and `host` are not used by Rust meanwhile.
        let f: extern "C" fn(*mut Ctx, u64) -> u32 = unsafe { core::mem::transmute(self.enter as usize) };
        let st = f(&mut *self.ctx, entry);
        let r = self.ctx.retired;
        self.stats.helpers += self.ctx.helpers;
        self.stats.ibtc_hits += self.ctx.ibtc_hits;
        self.stats.ibtc_misses += self.ctx.ibtc_misses;
        (self.stats.region_entries, self.stats.region_edges) = (self.ctx.rentries, self.ctx.redges);
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

/// Where a link slot whose owner was compiled at `link_gen` (ANY_GEN: another page) jumps into published block `s`.
#[inline(always)]
fn landing(s: &Slot, link_gen: u64, mutant: u8) -> u64 {
    if link_gen != ANY_GEN && (link_gen == s.gen || mutant == crate::jit_emit::MUT_INNER_ANY_GEN) { s.inner } else { s.entry }
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
    c.helpers += 1;
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
pub(crate) fn superblock(cache: &Cache, mem: &heirloom_mem_flat32::GuestMem, first: &[crate::t1op::Op], pc: u32, gen: u64, page: usize) -> Vec<crate::t1op::Op> {
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
    jit.arena.timer = jit.timer;
    let mut usable = usable_for(&raw) && !jit.dead;
    if !usable { jit.stats.refused = true; }
    while n < budget && !cpu.halted() && !cpu.blocked() {
        let pc = cpu.pc;
        let left = budget - n;
        // publish a page's worth of new blocks, or anything compiled a while ago (a long run must not wait for its end)
        if jit.due(n) { jit.publish(); }
        // the provider failed for good (jit.dead): no more compiling, publishing or entering, from this instruction on
        if usable && jit.dead { usable = false; jit.stats.refused = true; }
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
                            usable = usable_for(&raw) && !jit.dead;
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
                // a block's entry counter ran out in this entry: form the region it heads (compiled now, published with the
                // next batch; the block runs meanwhile - its counter wrapped, so it does not ask again)
                let head = core::mem::replace(&mut jit.ctx.hot, EMPTY_PC);
                if head != EMPTY_PC && what.is_empty() && !jit.dead && jit.region_after != 0 { jit.form_region::<H>(cache, host.mem(), head, n); }
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
                        if jit.hot(pc, gen, host.mem()) { jit.compile_native(pc, gen, n); }
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
        if usable && take == len && !host_call && jit.hot(pc, gen, host.mem()) {
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
    if !jit.dead && jit.due(n) { jit.publish(); }
    jit.clock += n;
    jit.stats.code_bytes = jit.arena.used() as u64;
    jit.stats.emitted_bytes = jit.arena.put_bytes;
    let io = jit.arena.io;
    (jit.stats.publishes, jit.stats.publish_pages, jit.stats.withdraws, jit.stats.withdraw_pages, jit.stats.codemem_failures, jit.stats.codemem_time) =
        (io.publishes, io.publish_pages, io.withdraws, io.withdraw_pages, io.failures, io.time);
    if jit.dead { jit.stats.refused = true; }
    (RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }, None)
}
