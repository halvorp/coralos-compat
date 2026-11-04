//! Tier 2's counters: one definition for every target (off x86-64 there is no JIT and `refused` says so when asked).

/// Tier 2's counters.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitStats {
    /// Blocks compiled.
    pub compiled: u64,
    /// Of those, blocks compiled again because their code page was written after they were compiled.
    pub recompiled: u64,
    /// Whole-cache drops because a block did not fit even in an emptied segment.
    pub arena_full: u64,
    /// Arena segments reused (FIFO) when the current one was full, and the blocks dropped with them.
    pub evictions: u64,
    pub evicted: u64,
    /// Entries from the dispatcher into JIT code (each may chain through many blocks).
    pub entries: u64,
    /// Instructions retired in JIT code.
    pub insns: u64,
    /// Entries or chained jumps that bailed (stale page or budget) back to the dispatcher.
    pub bails: u64,
    /// Whole-cache drops (a block larger than a segment, link table full, natives changed, RAM moved, reconfigured).
    pub flushes: u64,
    /// Compiled blocks that are superblocks (go on through a conditional branch's not-taken path into the next block).
    pub superblocks: u64,
    /// Whole guest functions compiled as one translated block (jit_xlate: a recognised shape).
    pub translated: u64,
    /// Instructions JIT code handed to tier 1's handler (jit::h_one), one call each.
    pub helpers: u64,
    /// Indirect branches (blr / bctr / a native's return) whose target the dispatch table held (a chained jump), and those
    /// that left for the dispatcher.
    pub ibtc_hits: u64,
    pub ibtc_misses: u64,
    /// Native calls made from JIT code (native blocks), and native blocks compiled.
    pub natives: u64,
    pub natives_compiled: u64,
    /// Bytes of code in the arena now.
    pub code_bytes: u64,
    /// Bytes of code written into the arena since the JIT was made (no page rounding): code size per compiled block.
    pub emitted_bytes: u64,
    /// Bytes of executable memory the arena reserved (64 MiB, or less when the provider refused that much).
    pub arena_bytes: u64,
    /// Big-endian loads and stores use MOVBE (the CPU has it and it was not turned off).
    pub movbe: bool,
    /// Time spent translating (compiling blocks, publishing pages, flushing), in the units of the clock given to
    /// `Exec::set_jit_clock`; 0 without one.
    pub translate_time: u64,
    /// The executable-memory provider: publish calls (pages made RX) and the pages they covered; withdraw calls (pages
    /// back to RW: a segment reused, or the whole arena) and their pages; calls it refused; time inside it (the
    /// `Exec::set_jit_clock` units). On CoralOS a call is one EXEC_CODE_PROTECT syscall and a page one TLB shootdown round.
    pub publishes: u64,
    pub publish_pages: u64,
    pub withdraws: u64,
    pub withdraw_pages: u64,
    pub codemem_failures: u64,
    pub codemem_time: u64,
    /// Regions compiled (jit_region: a hot block's head and the hot blocks reachable from it along constant edges, one
    /// publish unit), their nodes and guest instructions in all; heads whose counter ran out but that formed no region
    /// (fewer than two nodes qualified).
    pub regions: u64,
    pub region_nodes: u64,
    pub region_insns: u64,
    pub region_refused: u64,
    /// Compares in regions fused with their branch (jit_rflow: the CR field dead after it, never written).
    pub region_fused: u64,
    /// With the region census on (Exec::set_jit_region_census): region entries and internal edges taken, all runs.
    pub region_entries: u64,
    pub region_edges: u64,
    /// Tier::Jit was asked for but no executable memory was provided (or it was refused): tier 1 ran instead.
    pub refused: bool,
}

/// The region's shape: nodes (superblocks), guest instructions and code pages at most (the council's caps 16 / 128; tuned by
/// measurement), whether call edges (`bl`) are followed into the callee - the instruction after the `bl` is then a node too,
/// reached from the callee's `blr` by an inline return check (`cmp target, return pc; je`) before the IBTC probe - and whether
/// forming a region SPENDS the counters of its other nodes (they form no region of their own: the same hot code compiled into
/// several overlapping regions multiplies the code the hot path runs through).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionShape { pub max_nodes: usize, pub max_insns: usize, pub max_pages: usize, pub follow_calls: bool, pub spend: bool }
impl Default for RegionShape {
    fn default() -> Self { RegionShape { max_nodes: 16, max_insns: 128, max_pages: 4, follow_calls: false, spend: false } }
}
