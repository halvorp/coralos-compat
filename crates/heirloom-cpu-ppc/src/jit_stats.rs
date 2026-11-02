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
    /// Guest fields (GPRs, CR, XER, LR, CTR) the register cache held in host registers, summed over compiled blocks.
    pub pinned: u64,
    /// Compiled blocks that are superblocks (go on through a conditional branch's not-taken path into the next block).
    pub superblocks: u64,
    /// Native calls made from JIT code (native blocks), and native blocks compiled.
    pub natives: u64,
    pub natives_compiled: u64,
    /// Bytes of code in the arena now.
    pub code_bytes: u64,
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
    /// Tier::Jit was asked for but no executable memory was provided (or it was refused): tier 1 ran instead.
    pub refused: bool,
}
