//! Tier 2's block compiler: one tier-1 block (t1cache boundaries, t1op decode) to x86-64 - or a REGION of them
//! (`compile_region`, jit_region: internal edges are direct jumps, every other edge the same chained exit as a block's).
//!
//! Register model: rbx = &Cpu (a guest field that is not a global pin is a memory operand at its `offset_of!`), r12 = guest
//! RAM base, r13 = &Ctx (the remaining budget included), r15 = the instructions retired so far (across chained blocks); r9
//! r10 r11 rbp r14 hold the GLOBAL pins (jit_regs: CR, r0, r3, r4, XER across all JIT code, stored only where Rust can read
//! the Cpu). rax, rcx, rdx, rsi, rdi, r8 are scratch. Code size matters
//! (the WC3 working set is tens of thousands of blocks): every slow path is one
//! `call [r13 + slow]` into the shared stub (jit.rs), which calls the helper and either returns or leaves.
//!
//! Exactness (tier 1's contract): every exit leaves pc and the retired count (the dispatcher sets the time base from it); a
//! form without inline code, a load or store that would fault or touch a watched code page, and every FP arithmetic form
//! runs tier 1's own handler for that ONE instruction through `jit::h_one` with the exact pc and time base, which also
//! produces any trap - JIT code never builds a trap. A block starts with its own checks (the remaining budget covers the
//! whole block; its code page is still at the generation it was decoded at) and BAILS to the dispatcher otherwise, so a
//! chained jump to a stale or over-budget block is safe without unlinking anything.
use crate::cpu::{Cpu, XER_CA};
use crate::jit::{Ctx, EXIT_BAIL, EXIT_NORMAL};
use crate::jit_asm::*;
use crate::t1op::{Op, K};
use alloc::vec::Vec;
use core::mem::offset_of;

/// Planted mutants for the negative controls (each must make JitLockstep RED). 0 = none.
pub(crate) const MUT_SUBFIC_CA: u8 = 1;
pub(crate) const MUT_NO_SO: u8 = 2;
pub(crate) const MUT_NO_WATCH: u8 = 3;
pub(crate) const MUT_EXIT_COUNT: u8 = 4;
/// Single-precision FP arithmetic not rounded to single (jit_inline).
pub(crate) const MUT_FP_SINGLE: u8 = 5;
/// sraw's count > 31 path dropped (x86 masks the count to 5 bits: the wrong result and CA) (jit_inline).
pub(crate) const MUT_SRAW_BIG: u8 = 6;
/// adde / addze / subfe / subfze ignore XER[CA] as their carry in (jit_inline).
pub(crate) const MUT_CA_IN: u8 = 7;
/// The slow stub does not load the global pins back after the helper (jit.rs): a field tier 1's handler wrote is lost.
pub(crate) const MUT_GLOBAL_NO_RELOAD: u8 = 8;
/// The common exit does not store the global pins (jit.rs): the Cpu keeps stale CR / r0 / r3 / r4 / XER after JIT code.
pub(crate) const MUT_GLOBAL_NO_EXIT_STORE: u8 = 9;
/// CR0 of `add.` read from the add's own flags (its OF is not the result's sign test) instead of `test`.
pub(crate) const MUT_LAZY_FLAGS: u8 = 10;
/// Eviction leaves the dropped segment's blocks in the block map and the dispatch table (jit.rs): the dispatcher then
/// enters code whose pages went back to RW (no execute) or were rewritten.
pub(crate) const MUT_EVICT_STALE: u8 = 11;
/// fmadds rounded twice (the f64 sum, then single) instead of round-to-odd + one rounding (jit_fp).
pub(crate) const MUT_FMA_TWICE: u8 = 12;
/// A native block counts the native as two retired instructions (the time base and the budget drift from tier 0's).
pub(crate) const MUT_NATIVE_COUNT: u8 = 13;
/// The dispatcher keeps an evicted or flushed block's entry in its dispatch table (jit.rs): a stale jump-cache hit.
pub(crate) const MUT_TABLE_STALE: u8 = 14;
/// A native that changed the natives set or guest RAM does not make JIT code leave for the dispatcher's resync (jit.rs).
pub(crate) const MUT_NO_RESYNC: u8 = 15;
/// A native block does not store the global pins before h_native (jit_emit): the host's native reads stale r3 / r4 / ...
pub(crate) const MUT_NATIVE_GLOBALS: u8 = 16;
/// A chained jump from the target's own page lands past its page-generation check even when the two blocks were compiled
/// at different generations (jit.rs): a block left stale by a write to its page runs.
pub(crate) const MUT_INNER_ANY_GEN: u8 = 17;
/// A refused publish resets the whole arena and drops every block (jit.rs) - the pre-h86 behaviour that turned one slow
/// TLB shootdown on the Z83 into a withdraw-everything / recompile-everything loop.
pub(crate) const MUT_FLUSH_ON_REFUSAL: u8 = 18;
/// The store map stops mirroring the code watch after it is built (jit_smap): a page decoded later stays "store inline", so a
/// store into fresh code never bumps its generation.
pub(crate) const MUT_SMAP_STALE: u8 = 19;
/// A helper call does not store the global pins first (jit_emit): tier 1's handler reads stale CR / r0 / r3 / r4 / XER.
pub(crate) const MUT_GLOBAL_STALE: u8 = 20;
/// A region's internal forward edge lands on the node AFTER its target (jit_region): an internal branch target wrong.
pub(crate) const MUT_REGION_TARGET: u8 = 21;
/// A region checks only its head's page generation at entry (jit_region): a node decoded from another page that was
/// written since runs stale.
pub(crate) const MUT_REGION_PAGES: u8 = 22;
/// An internal region edge does not add the instructions its source node retired (the retired count, the one value every
/// side exit must write back, is dropped across the edge).
pub(crate) const MUT_REGION_COUNT: u8 = 23;
/// A region's back edge jumps without re-checking the budget (a hot loop runs past the dispatcher's budget).
pub(crate) const MUT_REGION_BACK_BUDGET: u8 = 24;
/// A region's `blr` goes to its first return point without the inline check (the wrong return taken in region: jit_region's
/// call following).
pub(crate) const MUT_REGION_RET: u8 = 25;
/// A region's exits do not store its locals (jit_regs): a side exit drops a dirty value.
pub(crate) const MUT_REGION_LOCAL_EXIT: u8 = 26;
/// A compare is fused with its branch (CR field never written: jit_rflow) even when the field is live after the branch.
pub(crate) const MUT_CR_LIVE_OUT: u8 = 27;
/// (28: measured, not shipped - forward chained jumps landing past the entry counter, h86 ppc-jit5 lever 1.)
/// A chained exit's tail stores the wrong pc (target + 4) when its target is not published (the dispatcher resumes there).
pub(crate) const MUT_TAIL_PC: u8 = 29;
/// The IBTC probe indexes with 14 bits (the 16K entries of 4bf4dfb's table) instead of 16: the Z83 regression of 2026-11-03,
/// dispatcher entries 4.4x per guest instruction.
pub(crate) const MUT_IBTC_SMALL: u8 = 30;

/// What a block's code may refer to outside itself.
pub(crate) struct Env<'a> {
    /// `jit::h_note::<H>` (lockstep only).
    pub(crate) note: u64,
    /// The guest RAM address mask the code is compiled for (all ones: no masking instruction at all).
    pub(crate) mask: u32,
    /// The link-table slot a constant exit to `pc` jumps through.
    pub(crate) link: &'a mut dyn FnMut(u32) -> *const u64,
    /// Chain to the next block (false in lockstep: every block returns to the dispatcher).
    pub(crate) chain: bool,
    /// Lockstep: record every inline store's old bytes in the guest RAM journal first (`h_note`).
    pub(crate) logged: bool,
    pub(crate) mutant: u8,
    pub(crate) table_mask: u32,
    /// Use MOVBE for big-endian loads and stores (CPUID-dispatched: jit_asm::cpu_has_movbe).
    pub(crate) movbe: bool,
    /// The Ctx offset of this block's entry counter (jit_region: a region forms when it runs out); None = no counter.
    pub(crate) counter: Option<i32>,
    /// Region census (diagnostics): count region entries and internal edges taken in the Ctx.
    pub(crate) census: bool,
    /// OUT: per link slot taken (in `link` call order), the offset of its EXIT TAIL in the code: `mov [pc], target` and the
    /// common exit. A slot whose target is not published points there (jit.rs), so the chained exit itself stores no pc.
    pub(crate) homes: Vec<usize>,
}

pub(crate) const fn gpr(n: u8) -> i32 { (offset_of!(Cpu, gpr) + 4 * (n as usize & 31)) as i32 }
pub(crate) const fn fpr(n: u8) -> i32 { (offset_of!(Cpu, fpr) + 8 * (n as usize & 31)) as i32 }
pub(crate) fn cpu(off: usize) -> M { m(RBX, off as i32) }
pub(crate) fn ctx(off: usize) -> M { m(R13, off as i32) }
pub(crate) const CR: usize = offset_of!(Cpu, cr);
pub(crate) const XER: usize = offset_of!(Cpu, xer);
pub(crate) const LR: usize = offset_of!(Cpu, lr);
pub(crate) const CTR: usize = offset_of!(Cpu, ctr);
pub(crate) const PC: usize = offset_of!(Cpu, pc);

/// How an access forms its EA: (rA|0) + rB, or rA + imm (rA != 0); `update` keeps the unmasked EA in r8d for rA.
#[derive(Clone, Copy)]
pub(crate) struct Ea { pub(crate) ra: u8, pub(crate) rb: Option<u8>, pub(crate) imm: u32, pub(crate) update: bool }
impl Ea {
    fn of(op: &Op, indexed: bool) -> Ea {
        Ea { ra: op.a, rb: indexed.then_some(op.b), imm: op.imm, update: matches!(op.k, K::Lwzu | K::Lbzu | K::Stwu | K::Stbu) }
    }
}

pub(crate) struct E<'a, 'b> {
    pub(crate) a: Asm,
    pub(crate) env: &'a mut Env<'b>,
    pc: u32,
    /// Inline fast paths that fell back: (jcc to patch, instruction index); each gets a slow stub after the body.
    slow: Vec<(Fix, usize)>,
    /// Superblock side exits: (the jumps taken to it, the branch's instruction index, its target); each gets an out-of-line
    /// chained exit after the body.
    sides: Vec<(Vec<Fix>, usize, u32)>,
    /// Instructions in the (super)block: a conditional branch before the last one is a side exit, not the end.
    n: usize,
    resume: Vec<usize>,
    words: Vec<u32>,
    /// Compiling a region: its nodes, labels and internal jumps.
    rg: Option<Rg>,
    /// Region locals (jit_regs): (Cpu offset, host register, written in the region).
    pub(crate) locals: Vec<(usize, u8, bool)>,
    /// GPR accesses / writes emitted so far (the dry run that picks the locals reads them).
    pub(crate) uses: [u32; 32],
    pub(crate) writes: [u32; 32],
    /// A dry run: no link slot is taken (the code is thrown away).
    dry: bool,
    /// The current region node's fused compares (jit_rflow), and a fused compare's flags waiting for its branch
    /// (Some(signed)).
    fuse: Vec<bool>,
    flags: Option<bool>,
    /// The targets of the chained exits emitted so far, in link-slot order: each gets an exit tail (`exit_tails`).
    tails_to: Vec<u32>,
}

/// A region being compiled (jit_region): the node pcs in layout order, each node's longest forward path, where each node's
/// code starts once emitted, the forward jumps waiting for a node, the back edges' budget exits, the node being emitted.
struct Rg { pcs: Vec<u32>, maxfwd: Vec<u32>, labels: Vec<Option<usize>>, fwd: Vec<(Fix, usize)>, backs: Vec<(Fix, u32)>, cur: usize, rets: Vec<u32> }

/// Compile `ops` (a whole tier-1 block or a superblock at `pc`, decoded from code page `page` at generation `gen`), one pass
/// (the global pins are fixed: jit_regs). Answers the code and the offset of its INNER entry.
pub(crate) fn compile(ops: &[Op], pc: u32, gen: u64, page: usize, env: &mut Env) -> (Vec<u8>, usize) {
    let mut e = E::new(ops, pc, env);
    let a = &mut e.a;
    // entry checks: the page generation (skipped by a chained jump from this page at this generation: the INNER entry),
    // then the budget
    a.ld64(RAX, ctx(offset_of!(Ctx, gen)));
    let at = m(RAX, (8 * page) as i32);
    if gen <= i32::MAX as u64 { a.alu_mi64(Alu::Cmp, at, gen as u32); } else { a.mov_ri64(RCX, gen); a.alu_rm64(Alu::Cmp, RCX, at); }
    let b2 = a.jcc(Cc::Ne);
    let inner = a.pos();
    a.lea64(RAX, m(R15, ops.len() as i32));
    a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, budget)));
    let b1 = a.jcc(Cc::A);
    // the entry counter (jit_region): when it runs out the dispatcher forms a region headed here (left with nothing retired)
    let hot = e.env.counter.map(|off| {
        e.a.alu_oi(Alu::Sub, O::M(m(R13, off)), 1);
        e.a.jcc(Cc::E)
    });
    e.body(ops);
    e.tails();
    e.exit_tails();
    if let Some(f) = hot {
        let a = &mut e.a;
        a.here(f);
        a.st32i(ctx(offset_of!(Ctx, hot)), pc);
        a.st32i(cpu(PC), pc);
        a.mov_ri(RAX, EXIT_NORMAL);
        a.jmp_m(ctx(offset_of!(Ctx, exit)));
    }
    let a = &mut e.a;
    a.here(b1);
    a.here(b2);
    // a chained jump stores no pc (jit.rs exit tails): the bail names its own
    a.st32i(cpu(PC), pc);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    (e.a.b, inner)
}

/// Compile a region (jit_region::Plan): its entry checks every page's generation (the head's first; the INNER entry is
/// past that one) and the budget for the longest forward path, then the nodes in layout order, each followed by its own
/// slow paths and side exits; internal edges are direct jumps (`go`). Answers the code and the offset of its inner entry.
pub(crate) fn compile_region(nodes: &[(u32, Vec<Op>)], maxfwd: &[u32], pages: &[(usize, u64, u32)], rets: &[u32], env: &mut Env) -> (Vec<u8>, usize) {
    // a dry run counts the GPR accesses: the most-used ones become locals for the real one
    let fuse = crate::jit_rflow::fuse_marks(nodes, rets, env.mutant == MUT_CR_LIVE_OUT);
    let locals = {
        let dry = emit_region(nodes, maxfwd, pages, rets, &fuse, env, Vec::new(), true);
        crate::jit_regs::pick_locals(&dry.uses, &dry.writes)
    };
    let e = emit_region(nodes, maxfwd, pages, rets, &fuse, env, locals, false);
    let inner = e.inner;
    (e.e.a.b, inner)
}

struct Emitted<'a, 'b> { e: E<'a, 'b>, inner: usize, uses: [u32; 32], writes: [u32; 32] }

fn emit_region<'a, 'b>(nodes: &[(u32, Vec<Op>)], maxfwd: &[u32], pages: &[(usize, u64, u32)], rets: &[u32], fuse: &[Vec<bool>], env: &'a mut Env<'b>, locals: Vec<(usize, u8, bool)>, dry: bool) -> Emitted<'a, 'b> {
    let mut e = E::new(&nodes[0].1, nodes[0].0, env);
    e.dry = dry;
    e.rg = Some(Rg { pcs: nodes.iter().map(|n| n.0).collect(), maxfwd: maxfwd.to_vec(), labels: alloc::vec![None; nodes.len()], fwd: Vec::new(), backs: Vec::new(), cur: 0, rets: rets.to_vec() });
    let mut bails = Vec::new();
    let checked = if e.env.mutant == MUT_REGION_PAGES { 1 } else { pages.len() };
    let mut inner = 0;
    for (k, &(page, gen, _)) in pages[..checked].iter().enumerate() {
        let a = &mut e.a;
        a.ld64(RAX, ctx(offset_of!(Ctx, gen)));
        let at = m(RAX, (8 * page) as i32);
        if gen <= i32::MAX as u64 { a.alu_mi64(Alu::Cmp, at, gen as u32); } else { a.mov_ri64(RCX, gen); a.alu_rm64(Alu::Cmp, RCX, at); }
        bails.push(a.jcc(Cc::Ne));
        if k == 0 { inner = a.pos(); }
    }
    let a = &mut e.a;
    a.lea64(RAX, m(R15, maxfwd[0] as i32));
    a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, budget)));
    bails.push(a.jcc(Cc::A));
    e.locals = locals;
    if e.env.census { e.a.alu_mi64(Alu::Add, ctx(offset_of!(Ctx, rentries)), 1); }
    e.load_locals();
    for (k, (pc, ops)) in nodes.iter().enumerate() {
        e.start_node(k, *pc, ops, &fuse[k]);
        e.body(ops);
        e.tails();
    }
    for (f, target) in core::mem::take(&mut e.rg.as_mut().expect("a region").backs) {
        e.a.here(f);
        e.chain(target, 0);
    }
    e.exit_tails();
    let a = &mut e.a;
    for f in bails { a.here(f); }
    a.st32i(cpu(PC), nodes[0].0);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    let (uses, writes) = (e.uses, e.writes);
    Emitted { e, inner, uses, writes }
}

/// Jump to the published block at the guest pc in r8d if the IBTC's entry for it holds that block; else fall through (rax,
/// rcx clobbered).
/// The table is the INDIRECT BRANCH TARGET CACHE of blr / bctr (and of a native's return): hits and misses are counted
/// (Ctx.ibtc_hits / ibtc_misses -> JitStats -> PpcProf).
fn probe(a: &mut Asm, table_mask: u32) {
    // the IBTC (jit.rs): 8-byte entries, guest pc then the entry's offset in the arena (rax, rcx clobbered)
    a.mov_rr(RAX, R8);
    a.sh_ri(Sh::Shr, RAX, 2);
    a.alu_ri(Alu::And, RAX, table_mask);
    a.ld64(RCX, ctx(offset_of!(Ctx, ibtc)));
    a.alu_rm(Alu::Cmp, R8, mxd(RCX, RAX, 8, 0));
    let miss = a.jcc(Cc::Ne);
    a.ld32(RCX, mxd(RCX, RAX, 8, 4));
    a.alu_rm64(Alu::Add, RCX, ctx(offset_of!(Ctx, abase)));
    a.alu_mi64(Alu::Add, ctx(offset_of!(Ctx, ibtc_hits)), 1);
    a.jmp_r(RCX);
    a.here(miss);
    a.alu_mi64(Alu::Add, ctx(offset_of!(Ctx, ibtc_misses)), 1);
}

/// The block of a bound NATIVE address: the host's native through `jit::h_native` (one retired instruction, as tier 0
/// counts it), then on to the pc it left (normally LR) through the dispatch table - so a guest call into the host and its
/// return stay in JIT code instead of a round trip through the dispatcher each (WC3 gameplay: ~1 native per 500 guest
/// instructions, ~750 host instructions of dispatcher each). The helper answers 0 = go on, 1 = leave after it (halted,
/// blocked, the natives set or guest RAM changed: the dispatcher must resync first), 2 = trap (nothing retired).
/// Every cached register was stored by the exit that got here, so the call clobbers nothing a block holds.
pub(crate) fn compile_native(pc: u32, env: &mut Env) -> Vec<u8> {
    let mut a = Asm::default();
    a.lea64(RAX, m(R15, 1));
    a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, budget)));
    let bail = a.jcc(Cc::A);
    a.mov_rr64(RDI, R13);
    a.mov_ri(RSI, pc);
    a.mov_rr64(RDX, R15);
    // the native reads and writes the Cpu: the global pins go there first and come back after (rax untouched)
    if env.mutant != MUT_NATIVE_GLOBALS { crate::jit_regs::store_globals(&mut a); }
    a.call_m(ctx(offset_of!(Ctx, native)));
    crate::jit_regs::load_globals(&mut a);
    a.test_rr(RAX, RAX);
    let out = a.jcc(Cc::Ne);
    a.alu_ri64(Alu::Add, R15, if env.mutant == MUT_NATIVE_COUNT { 2 } else { 1 });
    if env.chain {
        a.ld32(R8, cpu(PC));
        probe(&mut a, env.table_mask);
    }
    a.mov_ri(RAX, EXIT_NORMAL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    a.here(out);
    a.alu_ri(Alu::Cmp, RAX, 2);
    let t = a.jcc(Cc::E);
    a.alu_ri64(Alu::Add, R15, 1);
    a.mov_ri(RAX, EXIT_NORMAL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    a.here(t);
    a.mov_ri(RAX, crate::jit::EXIT_TRAP);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    a.here(bail);
    a.st32i(cpu(PC), pc);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    a.b
}

impl<'a, 'b> E<'a, 'b> {
    fn new(ops: &[Op], pc: u32, env: &'a mut Env<'b>) -> Self {
        E { a: Asm::default(), env, pc, slow: Vec::new(), sides: Vec::new(), n: ops.len(), resume: Vec::new(), words: ops.iter().map(|o| o.w).collect(), rg: None, locals: Vec::new(), uses: [0; 32], writes: [0; 32], dry: false, fuse: Vec::new(), flags: None, tails_to: Vec::new() }
    }

    /// Start region node `k` (its code begins here: forward jumps waiting for it land here).
    fn start_node(&mut self, k: usize, pc: u32, ops: &[Op], fuse: &[bool]) {
        (self.pc, self.n) = (pc, ops.len());
        self.fuse.clear();
        self.fuse.extend_from_slice(fuse);
        self.resume.clear();
        self.words.clear();
        self.words.extend(ops.iter().map(|o| o.w));
        let here = self.a.pos();
        let rg = self.rg.as_mut().expect("a region");
        rg.cur = k;
        rg.labels[k] = Some(here);
        let mut i = 0;
        while i < rg.fwd.len() {
            if rg.fwd[i].1 == k { let (f, _) = rg.fwd.swap_remove(i); self.a.patch(f, here); } else { i += 1; }
        }
    }

    /// The instructions, and the fall-through exit when the last one does not end the block.
    fn body(&mut self, ops: &[Op]) {
        let mut ended = false;
        for (i, op) in ops.iter().enumerate() {
            ended = self.op(i, op);
            self.resume.push(self.a.pos());
            if ended { break; }
        }
        if !ended { self.go(self.pc.wrapping_add(4 * ops.len() as u32), ops.len()); }
    }
}

impl E<'_, '_> {
    pub(crate) fn ipc(&self, i: usize) -> u32 { self.pc.wrapping_add(4 * i as u32) }

    /// On to `target` (a constant), `n` instructions of this block (node) retired: a direct jump when compiling a region that
    /// holds a node at `target` (a back edge re-checks the budget for that node's longest forward path first, and leaves
    /// through the chained exit when it does not fit), else the chained exit.
    fn go(&mut self, target: u32, n: usize) {
        let Some(j) = self.rg.as_ref().and_then(|r| r.pcs.iter().position(|&p| p == target)) else { return self.chain(target, n) };
        let mutant = self.env.mutant;
        let a = &mut self.a;
        if mutant != MUT_REGION_COUNT { a.alu_ri64(Alu::Add, R15, n as i32); }
        if self.env.census { a.alu_mi64(Alu::Add, ctx(offset_of!(Ctx, redges)), 1); }
        let rg = self.rg.as_mut().expect("a region");
        if j <= rg.cur {
            if mutant != MUT_REGION_BACK_BUDGET {
                a.lea64(RAX, m(R15, rg.maxfwd[j] as i32));
                a.alu_rm64(Alu::Cmp, RAX, ctx(offset_of!(Ctx, budget)));
                let out = a.jcc(Cc::A);
                rg.backs.push((out, target));
            }
            let f = a.jmp();
            a.patch(f, rg.labels[j].expect("an emitted node"));
        } else {
            let f = a.jmp();
            let j = if mutant == MUT_REGION_TARGET && j + 1 < rg.pcs.len() { j + 1 } else { j };
            rg.fwd.push((f, j));
        }
    }

    /// A region's `blr` (target in r8d): to each of the region's return points (the instruction after a followed `bl`) by an
    /// inline compare when the target is that one, `n` retired; else on to the dispatch-table probe.
    fn return_check(&mut self, n: usize) {
        let rets = match self.rg.as_ref() { Some(r) if !r.rets.is_empty() => r.rets.clone(), _ => return };
        for r in rets {
            if self.env.mutant == MUT_REGION_RET { return self.go(r, n); }
            self.a.alu_ri(Alu::Cmp, R8, r);
            let skip = self.a.jcc(Cc::Ne);
            self.go(r, n);
            self.a.here(skip);
        }
    }

    /// Leave for `target` (a constant), `n` instructions of this block retired.
    fn chain(&mut self, target: u32, n: usize) {
        self.store_locals();
        if n != 0 { self.a.alu_ri64(Alu::Add, R15, n as i32); }
        if self.env.chain {
            // the pc is stored by the slot's own exit tail when its target is not published (a chained target needs none)
            let slot = if self.dry { core::ptr::null() } else { (self.env.link)(target) };
            self.tails_to.push(target);
            self.a.mov_ri64(RAX, slot as u64);
            self.a.jmp_m(m(RAX, 0));
        } else {
            self.a.st32i(cpu(PC), target);
            self.exit_normal();
        }
    }
    /// The exit tails of every chained exit emitted (out of line, after the code): store the target pc, leave.
    fn exit_tails(&mut self) {
        for t in core::mem::take(&mut self.tails_to) {
            if !self.dry { self.env.homes.push(self.a.pos()); }
            let pc = if self.env.mutant == MUT_TAIL_PC { t.wrapping_add(4) } else { t };
            self.a.st32i(cpu(PC), pc);
            self.exit_normal();
        }
    }
    fn exit_normal(&mut self) {
        self.a.mov_ri(RAX, EXIT_NORMAL);
        self.a.jmp_m(ctx(offset_of!(Ctx, exit)));
    }
    /// Leave for the target in r8d, `n` retired: through the dispatch table when chaining.
    fn chain_dyn(&mut self, n: usize) {
        self.store_locals();
        let a = &mut self.a;
        a.alu_ri64(Alu::Add, R15, n as i32);
        a.st32(cpu(PC), R8);
        if self.env.chain { probe(a, self.env.table_mask); }
        self.exit_normal();
    }

    /// Run instruction `i` through tier 1's handler (the shared slow stub: it returns here when the block goes on, else
    /// leaves with the exact retired count). The global pins are stored first (the handler reads the Cpu); the stub loads
    /// them back after it (the handler may have written any field).
    pub(crate) fn helper(&mut self, i: usize) {
        if self.env.mutant != MUT_GLOBAL_STALE { crate::jit_regs::store_globals(&mut self.a); }
        self.store_locals();
        let (ipc, w) = (self.ipc(i), self.words[i]);
        let a = &mut self.a;
        a.mov_ri(RSI, w);
        a.mov_ri(RDX, ipc);
        a.mov_ri(RCX, i as u32);
        a.call_m(ctx(offset_of!(Ctx, slow)));
        self.load_locals();
    }

    /// edx (rdx for 8) = the big-endian `n`-byte value at [r12 + rax], zero- or sign-extended (16 bits).
    pub(crate) fn ld_be(&mut self, n: u8, sign: bool) { self.ld_be_to(RDX, n, sign) }
    /// Host register `r` = the big-endian `n`-byte value at [r12 + rax] (a guest field's own register: no copy after).
    pub(crate) fn ld_be_to(&mut self, r: u8, n: u8, sign: bool) {
        let (a, at) = (&mut self.a, mx(R12, RAX, 1));
        match (n, self.env.movbe) {
            (8, true) => a.movbe_ld(64, r, at),
            (8, false) => { a.ld64(r, at); a.bswap64(r); }
            (4, true) => a.movbe_ld(32, r, at),
            (4, false) => { a.ld32(r, at); a.bswap32(r); }
            (2, true) => { a.movbe_ld(16, r, at); if sign { a.movsx16_rr(r, r) } else { a.movzx16_rr(r, r) } }
            (2, false) => { a.movzx16(r, at); a.bswap32(r); a.sh_ri(if sign { Sh::Sar } else { Sh::Shr }, r, 16); }
            _ => a.movzx8(r, at),
        }
    }
    /// Store edx (rdx for 8) big-endian as `n` bytes at [r12 + rax] (edx is clobbered).
    pub(crate) fn st_be(&mut self, n: u8) {
        let (a, at) = (&mut self.a, mx(R12, RAX, 1));
        match (n, self.env.movbe) {
            (8, true) => a.movbe_st(64, at, RDX),
            (8, false) => { a.bswap64(RDX); a.st64(at, RDX); }
            (4, true) => a.movbe_st(32, at, RDX),
            (4, false) => { a.bswap32(RDX); a.st32(at, RDX); }
            (2, true) => a.movbe_st(16, at, RDX),
            (2, false) => { a.bswap32(RDX); a.sh_ri(Sh::Shr, RDX, 16); a.st16(at, RDX); }
            _ => a.st8(at, RDX),
        }
    }
    /// Store GPR `n` big-endian as `bytes` (1, 2 or 4) at [r12 + rax]: straight from its host register where the store needs
    /// no byte swap in a scratch register (MOVBE, or one byte), else through edx.
    fn st_gpr(&mut self, n: u8, bytes: u8) {
        match self.gr(n) {
            O::R(r) if bytes == 1 || self.env.movbe => {
                let at = mx(R12, RAX, 1);
                if bytes == 1 { self.a.st8(at, r) } else { self.a.movbe_st(8 * bytes, at, r) }
            }
            o => { self.a.ld32o(RDX, o); self.st_be(bytes) }
        }
    }
    /// The host register of GPR `n` when it has one (a global pin or a region local).
    pub(crate) fn greg(&mut self, n: u8) -> Option<u8> { if let O::R(r) = self.gw(n) { Some(r) } else { None } }
    /// rD = rS op imm (addi / ori / xori): in place in rD's register when it has one (`lea` for an add: no flags needed).
    fn op_imm(&mut self, alu: Alu, d: u8, s: u8, imm: u32) {
        match (self.greg(d), self.gr(s)) {
            (Some(rd), O::R(rs)) if matches!(alu, Alu::Add) => { if rd != rs || imm != 0 { self.a.lea32(rd, m(rs, imm as i32)) } }
            (Some(rd), os) => { self.a.ld32o(rd, os); self.a.alu_ri(alu, rd, imm); }
            (None, O::R(rs)) if matches!(alu, Alu::Add) => { self.a.lea32(RDX, m(rs, imm as i32)); self.stg(d, RDX); }
            _ => { self.ldg(RDX, s); self.a.alu_ri(alu, RDX, imm); self.stg(d, RDX); }
        }
    }
    /// rD = rS op rB for or / and / xor / add (+ Rc: CR0 from the result, the flags of the op itself): in rD's register when
    /// it has one - starting from rB instead when rD is rB's register (all four commute). Answers the result's register.
    fn op_rr(&mut self, alu: Alu, d: u8, s: u8, b: u8) -> u8 {
        let (rd, rb) = (self.greg(d), self.greg(b));
        let r = match rd {
            Some(r) if Some(r) == rb && s != b => { let os = self.gr(s); self.a.alu_ro(alu, r, os); return r }
            Some(r) => r,
            None => RDX,
        };
        let os = self.gr(s);
        self.a.ld32o(r, os);
        self.alug(alu, r, b);
        if r == RDX { self.stg(d, RDX) }
        r
    }

    /// A conditional jump to instruction `i`'s slow path (tier 1's handler for it, then back after it).
    pub(crate) fn slow_if(&mut self, cc: Cc, i: usize) {
        let f = self.a.jcc(cc);
        self.slow.push((f, i));
    }

    /// The slow paths of the inline forms (every fast path leaves before writing guest state): the helper, then back after
    /// the instruction. Then the superblock side exits: a chained exit to the branch target.
    fn tails(&mut self) {
        for (fixes, i, target) in core::mem::take(&mut self.sides) {
            for f in fixes { self.a.here(f); }
            self.go(target, i + 1);
        }
        for (f, i) in core::mem::take(&mut self.slow) {
            self.a.here(f);
            self.helper(i);
            let back = self.a.jmp();
            let r = self.resume[i];
            self.a.patch(back, r);
        }
    }

    /// CR field `bf` from the flags of a compare: LT / GT by `lt` / `gt`, EQ otherwise, SO from XER. The field's nibble is
    /// built in place by a move and two CMOVs from the Ctx's per-field constants (flag-free: the compare's flags are read
    /// here only), then SO (XER bit 31, the sign of XER's register) is or-ed in on the rarely taken path.
    pub(crate) fn cr_set(&mut self, bf: u8, lt: Cc, gt: Cc) {
        let no_so = self.env.mutant == MUT_NO_SO;
        let f = bf as usize & 7;
        let sh = 28 - 4 * f as u32;
        let k = offset_of!(Ctx, crk) + 8 * f;
        let a = &mut self.a;
        a.mov_ri(RAX, 0b0010 << sh);
        a.cmov_rm(lt, RAX, ctx(k));
        a.cmov_rm(gt, RAX, ctx(k + 4));
        if !no_so {
            match self.r(XER) {
                O::R(x) => self.a.test_rr(x, x),
                o => { self.a.ld32o(RCX, o); self.a.test_rr(RCX, RCX) }
            }
            let clear = self.a.jcc(Cc::Ns);
            self.a.alu_ri(Alu::Or, RAX, 1 << sh);
            self.a.here(clear);
        }
        let c = self.rw(CR);
        self.a.alu_oi(Alu::And, c, !(0xF << sh));
        self.a.alu_or(Alu::Or, c, RAX);
    }
    /// CR0 from the signed value in edx.
    pub(crate) fn cr0_of_edx(&mut self) { self.cr0_of(RDX) }
    /// CR0 from the signed value in host register `r`.
    pub(crate) fn cr0_of(&mut self, r: u8) {
        self.a.test_rr(r, r);
        self.cr_set(0, Cc::L, Cc::G);
    }
    /// CR0 from the flags the instruction that produced edx left (AND / OR / XOR: SF and ZF of the result, OF = 0, so
    /// they ARE `test edx, edx`'s); only moves may come between. Anything that sets OF (add, sub) must use cr0_of_edx.
    pub(crate) fn cr0_of_logic_flags(&mut self) {
        self.cr_set(0, Cc::L, Cc::G);
    }
    /// XER[CA] = the byte in r8b (0 or 1).
    pub(crate) fn ca_from_r8(&mut self) {
        let a = &mut self.a;
        a.movzx8_rr(R8, R8);
        a.sh_ri(Sh::Shl, R8, 29);
        let x = self.rw(XER);
        self.a.alu_oi(Alu::And, x, !XER_CA);
        self.a.alu_or(Alu::Or, x, R8);
    }

    /// eax = EA & mask (esi = EA unmasked too, for the update forms); a slow path unless `n` bytes from it are in RAM.
    pub(crate) fn ea(&mut self, i: usize, e: Ea, n: u8) {
        self.ea_regs(e);
        let lim = match n { 1 => offset_of!(Ctx, lim1), 2 => offset_of!(Ctx, lim2), 4 => offset_of!(Ctx, lim4), _ => offset_of!(Ctx, lim8) };
        self.a.alu_rm64(Alu::Cmp, RAX, ctx(lim));
        self.slow_if(Cc::A, i);
    }
    pub(crate) fn ea_regs(&mut self, e: Ea) {
        let r = if e.update { R8 } else { RAX };
        match (e.ra, e.rb) {
            (0, Some(rb)) => { let o = self.gr(rb); self.a.ld32o(r, o) }
            (ra, Some(rb)) => match (self.gr(ra), self.gr(rb)) {
                (O::R(x), O::R(y)) => self.a.lea32(r, mx(x, y, 1)),
                (oa, ob) => { self.a.ld32o(r, oa); self.a.alu_ro(Alu::Add, r, ob); }
            },
            (ra, None) => match self.gr(ra) {
                O::R(x) => self.a.lea32(r, m(x, e.imm as i32)),
                o => { self.a.ld32o(r, o); if e.imm != 0 { self.a.alu_ri(Alu::Add, r, e.imm); } }
            },
        }
        let a = &mut self.a;
        if e.update { a.mov_rr(RAX, R8); }
        if self.env.mask != u32::MAX { a.alu_rm(Alu::And, RAX, ctx(offset_of!(Ctx, mask))); }
    }
    /// The store guards after `ea_regs` (eax = the masked EA; no bounds test: the store map covers it): the store stays in
    /// one page - a 2- or 4-byte store by its ALIGNMENT (`test al, n - 1`: an unaligned one takes the slow path, exact and
    /// rare), an 8-byte one by its offset in the page - then `store_guard_n`.
    pub(crate) fn store_guard(&mut self, i: usize, e: Ea, n: u8) {
        match n {
            2 | 4 => {
                self.a.test_ri(RAX, n as u32 - 1);
                self.slow_if(Cc::Ne, i);
            }
            8 => {
                let a = &mut self.a;
                a.mov_rr(RCX, RAX);
                a.alu_ri(Alu::And, RCX, 0xFFF);
                a.alu_ri(Alu::Cmp, RCX, 0x1000 - 8);
                self.slow_if(Cc::A, i);
            }
            _ => {}
        }
        self.store_guard_n(i, n as u32);
        if self.env.logged { self.ea_regs(e); }
    }
    /// A store of `bytes` at eax that stays in one page: the slow path unless the store map (jit_smap) says the page is inside
    /// RAM and not watched (a watched page holds decoded code: tier 1's handler writes it through GuestMem, which bumps its
    /// generation); in lockstep the old bytes are journaled first (eax is then clobbered: the caller recomputes it).
    pub(crate) fn store_guard_n(&mut self, i: usize, bytes: u32) {
        let a = &mut self.a;
        if self.env.mutant != MUT_NO_WATCH {
            a.mov_rr(RCX, RAX);
            a.sh_ri(Sh::Shr, RCX, 12);
            a.alu_rm64(Alu::Add, RCX, ctx(offset_of!(Ctx, smap)));
            a.cmp8_mi(m(RCX, 0), 0);
            self.slow_if(Cc::E, i);
        } else {
            // the planted mutant drops the watch test only: RAM bounds still hold (the map was both)
            a.lea64(RCX, m(RAX, bytes as i32));
            a.alu_rm64(Alu::Cmp, RCX, ctx(offset_of!(Ctx, len)));
            self.slow_if(Cc::A, i);
        }
        if self.env.logged {
            // h_note is a C call: the cache's caller-saved registers are kept across it (rsp stays 16-byte aligned)
            let a = &mut self.a;
            for r in crate::jit_regs::CALLER_SAVED.iter().chain(&crate::jit_regs::LOCAL_REGS) { a.push(*r); }
            a.alu_ri64(Alu::Sub, RSP, 8);
            a.mov_rr64(RDI, R13);
            a.mov_rr(RSI, RAX);
            a.mov_ri(RDX, bytes);
            a.mov_ri64(RAX, self.env.note);
            a.call_r(RAX);
            a.alu_ri64(Alu::Add, RSP, 8);
            for r in crate::jit_regs::CALLER_SAVED.iter().chain(&crate::jit_regs::LOCAL_REGS).rev() { a.pop(*r); }
        }
    }

    /// One instruction; true when it ended the block.
    fn op(&mut self, i: usize, op: &Op) -> bool {
        let (d, s, b) = (op.d, op.a, op.b);
        match op.k {
            K::Nop => {}
            K::Li => { let o = self.gw(d); self.a.st32io(o, op.imm) }
            K::Addi | K::Ori | K::Xori => {
                let alu = match op.k { K::Addi => Alu::Add, K::Ori => Alu::Or, _ => Alu::Xor };
                self.op_imm(alu, d, s, op.imm);
            }
            K::AndiRc => {
                self.ldg(RDX, s);
                self.a.alu_ri(Alu::And, RDX, op.imm);
                self.stg(d, RDX);
                self.cr0_of_logic_flags();
            }
            K::Addic | K::AddicRc => {
                self.ldg(RDX, s);
                self.a.alu_ri(Alu::Add, RDX, op.imm);
                self.a.setcc(Cc::B, R8);
                self.stg(d, RDX);
                self.ca_from_r8();
                if op.k == K::AddicRc { self.cr0_of_edx(); }
            }
            K::Subfic => {
                // CA = carry of (!rA + imm + 1) = no borrow in imm - rA
                self.a.mov_ri(RDX, op.imm);
                let o = self.gr(s);
                self.a.alu_ro(Alu::Sub, RDX, o);
                self.a.setcc(if self.env.mutant == MUT_SUBFIC_CA { Cc::B } else { Cc::Ae }, R8);
                self.stg(d, RDX);
                self.ca_from_r8();
            }
            K::Mulli => { let o = self.gr(s); self.a.imul_roi(RDX, o, op.imm); self.stg(d, RDX); }
            K::Rlwinm | K::RlwinmRc => {
                let r = self.greg(d).unwrap_or(RDX);
                self.ldg(r, s);
                if b & 31 != 0 { self.a.sh_ri(Sh::Rol, r, b & 31); }
                self.a.alu_ri(Alu::And, r, op.imm);
                self.stg(d, r);
                if op.k == K::RlwinmRc { self.cr0_of_logic_flags(); }
            }
            K::Cmpi | K::Cmpli | K::Cmp | K::Cmpl => {
                if matches!(op.k, K::Cmpi | K::Cmpli) {
                    let o = self.gr(s);
                    self.a.alu_oi(Alu::Cmp, o, op.imm);
                } else {
                    self.ldg(RDX, s);
                    self.alug(Alu::Cmp, RDX, b);
                }
                let signed = matches!(op.k, K::Cmpi | K::Cmp);
                // fused (jit_rflow): the branch right after reads the flags; the field is never written
                if self.fuse.get(i) == Some(&true) { self.flags = Some(signed); }
                else if signed { self.cr_set(d, Cc::L, Cc::G) } else { self.cr_set(d, Cc::B, Cc::A) }
            }
            // mr: a move (no flags needed)
            K::Or if s == b => { let o = self.gr(s); match self.greg(d) { Some(r) => self.a.ld32o(r, o), None => { self.a.ld32o(RDX, o); self.stg(d, RDX) } } }
            K::Or | K::OrRc | K::And | K::AndRc | K::Xor | K::Add | K::AddRc => {
                let alu = match op.k { K::Or | K::OrRc => Alu::Or, K::And | K::AndRc => Alu::And, K::Xor => Alu::Xor, _ => Alu::Add };
                let r = self.op_rr(alu, d, s, b);
                match op.k {
                    K::OrRc | K::AndRc => self.cr0_of_logic_flags(),
                    // an add's OF is not the result's sign test: only the planted mutant reads CR0 from it
                    K::AddRc if self.env.mutant == MUT_LAZY_FLAGS => self.cr0_of_logic_flags(),
                    K::AddRc => self.cr0_of(r),
                    _ => {}
                }
            }
            K::Andc => { self.ldg(RDX, b); self.a.not(RDX); self.alug(Alu::And, RDX, s); self.stg(d, RDX); }
            K::Nor => { self.ldg(RDX, s); self.alug(Alu::Or, RDX, b); self.a.not(RDX); self.stg(d, RDX); }
            K::Subf | K::SubfRc => {
                self.ldg(RDX, b);
                self.alug(Alu::Sub, RDX, s);
                self.stg(d, RDX);
                if op.k == K::SubfRc { self.cr0_of_edx(); }
            }
            K::Neg => { self.ldg(RDX, s); self.a.neg(RDX); self.stg(d, RDX); }
            K::Mullw => { self.ldg(RDX, s); let o = self.gr(b); self.a.imul_ro(RDX, o); self.stg(d, RDX); }
            K::Extsh => { let o = self.gr(s); self.a.movsx16o(RDX, o); self.stg(d, RDX); }
            K::Extsb => { let o = self.gr(s); self.a.movsx8o(RDX, o); self.stg(d, RDX); }
            K::Srawi | K::SrawiRc => {
                let sh = b & 31;
                self.ldg(RDX, s);
                let a = &mut self.a;
                a.mov_rr(R8, RDX);
                if sh != 0 { a.sh_ri(Sh::Sar, RDX, sh); }
                self.stg(d, RDX);
                // CA = negative and a one bit shifted out
                let a = &mut self.a;
                a.test_ri(R8, if sh == 0 { 0 } else { (1u32 << sh) - 1 });
                a.setcc(Cc::Ne, RAX);
                a.test_rr(R8, R8);
                a.setcc(Cc::S, RCX);
                a.and8_rr(RAX, RCX);
                a.mov_rr(R8, RAX);
                self.ca_from_r8();
                if op.k == K::SrawiRc { self.cr0_of_edx(); }
            }
            K::Slw | K::Srw => {
                self.ldg(RCX, b);
                self.ldg(RDX, s);
                let a = &mut self.a;
                a.sh_cl(if op.k == K::Slw { Sh::Shl } else { Sh::Shr }, RDX);
                a.alu_rr(Alu::Xor, RAX, RAX);
                a.test_ri(RCX, 0x20);
                a.cmov(Cc::Ne, RDX, RAX);
                self.stg(d, RDX);
            }
            K::Mflr | K::Mfctr => { let o = self.r(if op.k == K::Mflr { LR } else { CTR }); self.a.ld32o(RDX, o); self.stg(d, RDX); }
            K::Mtlr | K::Mtctr => { self.ldg(RDX, d); let o = self.w(if op.k == K::Mtlr { LR } else { CTR }); self.a.st32o(o, RDX); }
            K::Lwz | K::Lbz | K::Lhz | K::Lha | K::Lwzu | K::Lbzu | K::Lwzx | K::Lbzx => {
                let n = match op.k { K::Lwz | K::Lwzu | K::Lwzx => 4, K::Lhz | K::Lha => 2, _ => 1 };
                self.ea(i, Ea::of(op, matches!(op.k, K::Lwzx | K::Lbzx)), n);
                let r = self.greg(d).unwrap_or(RDX);
                self.ld_be_to(r, n, op.k == K::Lha);
                self.stg(d, r);
                if matches!(op.k, K::Lwzu | K::Lbzu) { self.stg(s, R8); }
            }
            K::Lfd => {
                self.ea(i, Ea::of(op, false), 8);
                self.ld_be(8, false);
                self.a.st64(m(RBX, fpr(d)), RDX);
            }
            K::Stw | K::Stb | K::Sth | K::Stwu | K::Stbu | K::Stwx | K::Stbx | K::Stfd => {
                let n = match op.k { K::Stw | K::Stwu | K::Stwx => 4, K::Sth => 2, K::Stfd => 8, _ => 1 };
                let indexed = matches!(op.k, K::Stwx | K::Stbx);
                self.ea_regs(Ea::of(op, indexed));
                self.store_guard(i, Ea::of(op, indexed), n);
                if n == 8 { self.a.ld64(RDX, m(RBX, fpr(d))); self.st_be(n); } else { self.st_gpr(d, n) }
                if matches!(op.k, K::Stwu | K::Stbu) { self.stg(s, R8); }
            }
            K::B => { self.go(op.imm, i + 1); return true; }
            K::Bl => { self.set_lr(i); self.go(op.imm, i + 1); return true; }
            K::Bc | K::Bcl if i + 1 < self.n => {
                // inside a superblock: the taken edge leaves through an out-of-line side exit, the not-taken one runs on
                if op.k == K::Bcl { self.set_lr(i); }
                let taken = self.cond_taken(d, s);
                self.sides.push((taken, i, op.imm));
            }
            K::Bc | K::Bcl => {
                if op.k == K::Bcl { self.set_lr(i); }
                let not = self.cond(d, s);
                self.go(op.imm, i + 1);
                for f in not { self.a.here(f); }
                self.go(self.ipc(i).wrapping_add(4), i + 1);
                return true;
            }
            K::Bclr | K::Bclrl | K::Bcctr | K::Bcctrl => {
                let from = if matches!(op.k, K::Bcctr | K::Bcctrl) { CTR } else { LR };
                let o = self.r(from);
                self.a.ld32o(R8, o);
                self.a.alu_ri(Alu::And, R8, !3);
                if matches!(op.k, K::Bclrl | K::Bcctrl) { self.set_lr(i); }
                let not = self.cond(d, s);
                if op.k == K::Bclr { self.return_check(i + 1); }
                self.chain_dyn(i + 1);
                for f in not { self.a.here(f); }
                self.go(self.ipc(i).wrapping_add(4), i + 1);
                return true;
            }
            K::Generic | K::GenericEnd | K::Sc | K::Fp59 | K::Fp63 | K::Lfs | K::Stfs => {
                if self.inline(i, op) { return false; }
                self.helper(i);
                if op.terminal() {
                    // a block-ending Generic always leaves through its helper (it is a branch: answer 1)
                    self.chain_dyn_after_helper(i);
                    return true;
                }
            }
        }
        false
    }

    /// After a terminal Generic answered 0 (it never does: a branch answers 1 with pc set), leave with pc as it is.
    fn chain_dyn_after_helper(&mut self, i: usize) {
        self.a.alu_ri64(Alu::Add, R15, i as i32 + 1);
        self.exit_normal();
    }

    /// A branch fused with the compare before it (jit_rflow: no CTR, a LT / GT / EQ bit of the compared field): the condition
    /// under which it is TAKEN, on the compare's flags.
    fn fused_cc(&mut self, bo: u8, bi: u8) -> Option<Cc> {
        let signed = self.flags.take()?;
        let cc = match (bi & 3, signed) { (0, true) => Cc::L, (1, true) => Cc::G, (0, false) => Cc::B, (1, false) => Cc::A, _ => Cc::E };
        Some(if bo & 0x08 != 0 { cc } else { cc.not() })
    }

    /// The BO / BI tests of a conditional branch (CTR decremented first unless BO says not); the returned jumps go to
    /// the NOT-taken path.
    fn cond(&mut self, bo: u8, bi: u8) -> Vec<Fix> {
        if let Some(cc) = self.fused_cc(bo, bi) { return alloc::vec![self.a.jcc(cc.not())]; }
        let mut not = Vec::new();
        if bo & 0x04 == 0 {
            let o = self.rw(CTR);
            self.a.alu_oi(Alu::Sub, o, 1);
            not.push(self.a.jcc(if bo & 0x02 != 0 { Cc::Ne } else { Cc::E }));
        }
        if bo & 0x10 == 0 {
            let o = self.r(CR);
            self.a.bt_oi(o, 31 - (bi & 31));
            not.push(self.a.jcc(if bo & 0x08 != 0 { Cc::Ae } else { Cc::B }));
        }
        not
    }

    /// The same tests as [`Self::cond`], but the returned jumps go to the TAKEN path (the not-taken one falls through):
    /// one inverted jcc when only CTR or only a CR bit is tested.
    fn cond_taken(&mut self, bo: u8, bi: u8) -> Vec<Fix> {
        if let Some(cc) = self.fused_cc(bo, bi) { return alloc::vec![self.a.jcc(cc)]; }
        match (bo & 0x04 == 0, bo & 0x10 == 0) {
            (true, true) => {
                let not = self.cond(bo, bi);
                let t = self.a.jmp();
                for f in not { self.a.here(f); }
                alloc::vec![t]
            }
            (true, false) => {
                let o = self.rw(CTR);
                self.a.alu_oi(Alu::Sub, o, 1);
                alloc::vec![self.a.jcc(if bo & 0x02 != 0 { Cc::E } else { Cc::Ne })]
            }
            (false, true) => {
                let o = self.r(CR);
                self.a.bt_oi(o, 31 - (bi & 31));
                alloc::vec![self.a.jcc(if bo & 0x08 != 0 { Cc::B } else { Cc::Ae })]
            }
            // branch always: superblocks never continue past one (jit.rs), but stay exact if one is given
            (false, false) => alloc::vec![self.a.jmp()],
        }
    }
}
