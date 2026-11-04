//! Tier 2's COLD-TAIL rule (h86 fnreg): which tier-1 blocks are worth compiling. The cumulative rule (jit.rs HOT_AFTER: the 2nd
//! run ever) compiles every block that runs twice in the whole session, so code a game runs once every few frames joins the
//! JIT's per-frame footprint. On the Z83 the front end is the cost (NO_ALLOC_CYCLES.NOT_DELIVERED ~10.7 of ~17.3 cycles per
//! guest instruction, 0.31 I-cache misses per guest instruction), and the host cache simulation (Airmont geometry, WC3
//! gameplay) puts 47% of the JIT's L2 code misses in lines that run fewer than 4 times a frame - 3.2% of its instructions.
//! The rule: a block compiles once it ran `min` times within ONE window of `window` guest instructions (the counts start over
//! each window), so code that runs less often than that stays on tier 1, whose own code is small and always hot.
//!
//! MEASURED on the host's Airmont cache model (WC3 gameplay, 576 frames, per guest instruction against the cumulative rule):
//! 2.5 M / 4 cut L2 code misses 15% but tier 1's work added ~1.2 host instructions (a loss); 2.5 M / 2 and 10 M / 4 are within
//! a few percent of the cumulative rule on every count (10 M / 4: distinct JIT lines -21%, L2 code misses -5%). So it stays a
//! knob - off by default - for the Z83's jitablate window, where the L2 is shared with the GL worker and may weigh misses more.
/// The knob's setting (KNOB_COLD_TAIL): about one Z83 WC3 frame of guest instructions, 4 runs in it.
pub(crate) const DEFAULT: (u64, u32) = (2_500_000, 4);
