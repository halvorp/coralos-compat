//! Tier 2's planted mutants: each one a deliberate fault the negative controls (tests/tier2.rs, tier2_dispatch.rs) must
//! catch - a gate that stays green with one planted proves nothing. jit_emit re-exports them.

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
/// The common exit does not store the global pins (jit.rs): the Cpu keeps stale CR / r0 / r1 / r3 / r4 after JIT code.
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
/// A helper call does not store the global pins first (jit_emit): tier 1's handler reads stale CR / r0 / r1 / r3 / r4.
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
/// The store map's entry covers its own page only (jit_smap): an unaligned store that starts on a free page and ends on a
/// watched code page runs inline and the code page's generation is never bumped (stale code runs).
pub(crate) const MUT_SMAP_ONE_PAGE: u8 = 31;
/// The common exit does not settle a pending FPSCR[FPRF] (jit.rs, jit_inline): FPSCR is left with an older class.
pub(crate) const MUT_FPRF_EXIT: u8 = 32;
/// Tier 1's handler setting XER[SO] under SO-free code does not make JIT code leave (jit.rs h_one): a compare after it in
/// the same entry builds its CR field without SO.
pub(crate) const MUT_SO_STAYS: u8 = 33;
/// fmadds rounds s straight to single at a single midpoint too (jit_fp): the exact path only for a tiny s.
pub(crate) const MUT_FMA_NO_MIDPOINT: u8 = 34;
/// A load group's head tests a span that leaves out its last member's bytes (jit_lgroup): a member that ends past RAM runs.
pub(crate) const MUT_LGROUP_SHORT: u8 = 35;
/// A form that makes an FPR fact has slow paths that come back (jit_ffacts): after tier 1's handler left the FPR as it was
/// (an enabled invalid operation), later code still trusts the fact.
pub(crate) const MUT_FACTS_REJOIN: u8 = 36;
/// A store group may span more than a page (jit_lgroup): a member two pages above its head's test runs inline on a code page.
pub(crate) const MUT_SGROUP_WIDE: u8 = 37;
/// The matrix compare's vector fast path takes NaN / infinity lanes for equal floats (jit_xlate): a bit-identical NaN pair is
/// "equal" (fcmpu says unordered: the routine answers 1).
pub(crate) const MUT_XLATE_FAST_NAN: u8 = 38;
/// The blend loop's translation runs on a destination that partly overlaps a source (jit_xbytes): the original's byte order
/// reads bytes the loop itself wrote; the vector pass does not.
pub(crate) const MUT_XLATE_OVERLAP: u8 = 39;
/// The byte compare's translation retires 16 + 6k at a difference whatever its sign (jit_xbytes): the original's `li r3,-1`
/// and its second blr (a[k] < b[k]) go uncounted.
pub(crate) const MUT_XLATE_COUNT: u8 = 40;
