//! Tier 2's code census (diagnostics only, Exec::set_jit_tag_census): what each emitted byte is FOR. The emitter tags its code
//! as it goes (jit_asm `tag_in` / `set_tag`: a tag = the guest form's `K` << 8 | one of the categories below); the JIT keeps a
//! tag per arena byte; a host tool joins that map with a profiler's per-address counts (callgrind Ir over the arena) into
//! "host instructions per guest instruction, by what they do". The tags change no emitted byte.

/// Not tagged: the stubs, a native's block, a translated function (`NATIVE`, `XLATE` when the JIT knows which).
pub(crate) const OTHER: u8 = 0;
/// A block's / region's entry: its page-generation test, its budget test, its entry counter.
pub(crate) const ENTRY_GEN: u8 = 1;
pub(crate) const ENTRY_BUDGET: u8 = 2;
pub(crate) const ENTRY_COUNTER: u8 = 3;
/// Region locals loaded (at the region's entry and after a helper call) / stored (at exits and before a helper call).
pub(crate) const LOCALS_LOAD: u8 = 4;
pub(crate) const LOCALS_STORE: u8 = 33;
/// The guest form's own work (what is left once everything below is told apart).
pub(crate) const BODY: u8 = 5;
/// A load's or store's effective address.
pub(crate) const EA: u8 = 6;
/// The RAM bounds test of a load; the store map's test (bounds + watched code pages).
pub(crate) const GUARD_BOUNDS: u8 = 7;
pub(crate) const GUARD_SMAP: u8 = 9;
/// The access itself (MOVBE, or a load / store with its byte swap).
pub(crate) const ACCESS: u8 = 10;
/// A CR field built from flags (cr_set); XER[CA] from a carry.
pub(crate) const CR_SET: u8 = 11;
pub(crate) const CA: u8 = 12;
/// A GPR that is not in a host register read into / written from a scratch register; a copy between host registers.
pub(crate) const GPR_LD: u8 = 13;
pub(crate) const GPR_ST: u8 = 14;
pub(crate) const GPR_MOV: u8 = 15;
/// A conditional branch's tests (CTR decrement, CR bit, a fused compare's jcc); LR set by a branch-and-link.
pub(crate) const BRANCH: u8 = 16;
pub(crate) const LR_SET: u8 = 17;
/// Leaving for a constant target: the retired count added, the link-slot jump.
pub(crate) const RETIRED: u8 = 18;
pub(crate) const CHAIN: u8 = 19;
/// Leaving for a target in a register (blr / bctr): pc stored, the IBTC probe; a region's inline return-point tests.
pub(crate) const DYN: u8 = 20;
pub(crate) const RET_CHECK: u8 = 21;
/// A region's internal edge (a back edge's budget test, the jump).
pub(crate) const EDGE: u8 = 22;
/// A form with no inline code calling tier 1's handler on the HOT path; the out-of-line slow paths of inline forms.
pub(crate) const HELPER: u8 = 23;
pub(crate) const SLOW: u8 = 24;
/// The exit tails (a chained exit whose target is not published), the bail paths.
pub(crate) const TAIL: u8 = 26;
pub(crate) const BAIL: u8 = 27;
/// FP guards (NaN / finite / FPSCR[RN] tests) and the FPSCR[FPRF] stub call.
pub(crate) const FP_GUARD: u8 = 28;
pub(crate) const FPRF: u8 = 29;
/// The lockstep journal call (never in production code).
pub(crate) const NOTE: u8 = 30;
pub(crate) const NATIVE: u8 = 31;
pub(crate) const XLATE: u8 = 32;
/// The shared stubs (jit.rs `stubs`): trampoline + common exit + exit stub, the slow stub, the FPSCR[FPRF] stubs, stfs's.
pub(crate) const STUB_EXIT: u8 = 34;
pub(crate) const STUB_SLOW: u8 = 35;
pub(crate) const STUB_FPRF: u8 = 36;
pub(crate) const STUB_STFS: u8 = 37;

/// `K` of a tag outside any guest instruction (entries, exits after the body, tails).
pub(crate) const NO_K: u16 = 0xFF;
/// A tag run of this value marks where a guest instruction's code starts (the map ORs [`START_BIT`] into that byte's tag
/// and keeps the tag in force): the profiler's count at that address is how often the instruction ran.
pub(crate) const START: u16 = 0xFFFF;
pub(crate) const START_BIT: u16 = 0x80;

/// The census kind of a guest form: its `K`, finer for the FP and Generic forms (opcode 59 / 63: 100 + / 132 + the low five
/// bits of the extended opcode; Generic: 170 + the primary opcode).
pub(crate) fn kind(op: &crate::t1op::Op) -> u16 {
    use crate::t1op::K;
    let xo5 = ((op.w >> 1) & 31) as u16;
    match op.k {
        K::Fp59 => 100 + xo5,
        K::Fp63 => 132 + xo5,
        K::Generic | K::GenericEnd => 170 + (op.w >> 26) as u16,
        k => k as u16,
    }
}
