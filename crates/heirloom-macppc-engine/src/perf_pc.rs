//! A guest-PC sampling profiler (h86 perf0, WC3 30 fps step 0): where the GUEST's own instructions are. The stage clock says how a
//! frame's time splits between the interpreter, the glue and the host; this says which guest FUNCTIONS the interpreter's share is.
//!
//! Method: with sampling on, [`crate::machine::MacPpc::step`] runs the CPU in chunks of [`PcProf::every`] instructions (a prime, so a
//! loop of a power-of-two length does not alias with it) and after each chunk records the guest PC the CPU stopped at, weighted by the
//! instructions the chunk retired and by the host time the chunk took outside glue calls. A chunk ends where its budget ran out, i.e.
//! at the start of the NEXT block, so the PC is the block the CPU was about to run: over thousands of samples the estimate is of the
//! share of INSTRUCTIONS (and, through the time weight, of interpreter time) per code address, not of calls.
//!
//! The game's PEF carries no function symbols, so a PC is mapped to a function by a static scan of its code section: the targets of
//! every `bl` in it are function entries ([`function_starts`]); a PC belongs to the nearest entry at or below it. A function that no
//! `bl` reaches (called through a pointer only) is merged into its predecessor: the table is a floor on the function count. The
//! host's Ghidra export (/build/ghidra/wc3-game, base 0x10000000 = guest 0x200000) has the real 25k boundaries; the engine does not
//! need it, so a silicon line can be joined to it by address afterwards.
//!
//! Idle and poll loops: every glue call of a clock ([`crate::perf::Class::Clock`]) or thread ([`crate::perf::Class::Thread`]) class
//! records its caller (the call's return address) in [`PcProf::callers`]; a sample inside a function that makes such calls is counted
//! as `clockfn` / `threadfn`. That is an UPPER bound on spinning (a game loop that also reads the clock is counted whole); the exact
//! figure for a clock busy-wait is the stage clock's `clock_poll` (instructions between two reads of one tick).
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Guest instructions between two samples (prime).
pub const EVERY: u64 = 16_381;
/// The interval Heirloom.app uses on silicon (prime): a quarter of the host sampling rate, so the chunking costs the Atom less
/// (~66 samples a frame at 4.3M instructions a frame, ~17k a report window: a 5% function is known to about 0.2%).
pub const SILICON_EVERY: u64 = 65_521;
/// Distinct sampled PCs kept for the cumulative table; more are counted in [`PcProf::dropped`] (a PC is a block start: a game has
/// tens of thousands, so this is not reached in practice, but the table is bounded).
pub const MAX_PCS: usize = 1 << 18;
/// Distinct (class, caller) pairs kept.
pub const MAX_CALLERS: usize = 1 << 12;
/// A function this small (bytes) is a glue thunk, not a caller worth naming.
pub const THUNK: u32 = 64;
/// Functions named in a window's line.
pub const TOP: usize = 5;

/// Instructions and interpreter time (host clock units) charged to one PC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Weight { pub insns: u64, pub time: u64 }

/// The profiler's state; the per-window table lives in the window ([`crate::perf::Window::pcs`]).
#[derive(Default)]
pub struct PcProf {
    /// Instructions between samples.
    pub every: u64,
    /// Every sample since the profiler was switched on, by guest PC.
    pub cum: BTreeMap<u32, Weight>,
    /// Samples that did not fit [`MAX_PCS`].
    pub dropped: u64,
    /// Glue calls of the idle-ish classes by (class index, caller address): how many.
    pub callers: BTreeMap<(u8, u32), u64>,
    /// Function entries of the scanned range, built on first use.
    pub starts: Vec<u32>,
}

impl PcProf {
    pub fn new(every: u64) -> PcProf { PcProf { every: every.max(1), ..Default::default() } }

    pub(crate) fn caller(&mut self, class: u8, lr: u32) {
        if self.callers.len() < MAX_CALLERS || self.callers.contains_key(&(class, lr)) { *self.callers.entry((class, lr)).or_insert(0) += 1; }
    }

    /// One sample into the cumulative table and the window's.
    pub(crate) fn record(&mut self, win: &mut BTreeMap<u32, Weight>, pc: u32, w: Weight) {
        for t in [&mut *win, &mut self.cum] {
            if t.len() < MAX_PCS || t.contains_key(&pc) {
                let e = t.entry(pc).or_default();
                e.insns += w.insns;
                e.time += w.time;
            }
        }
        if self.cum.len() >= MAX_PCS && !self.cum.contains_key(&pc) { self.dropped += 1; }
    }
}

/// Function entries in the code `words` (address, word) of `[lo, hi)`: the targets of the `bl`s (primary opcode 18, AA = 0, LK = 1)
/// that land inside the range, sorted and unique. A `bl` to the next instruction (the position-independent `get pc` idiom) is not a call.
pub fn function_starts(words: impl Iterator<Item = (u32, u32)>, lo: u32, hi: u32) -> Vec<u32> {
    let mut t: Vec<u32> = Vec::new();
    for (pc, w) in words {
        if w >> 26 != 18 || w & 3 != 1 { continue; }
        let li = ((w & 0x03FF_FFFC) as i32) << 6 >> 6; // 26-bit signed displacement
        let target = pc.wrapping_add(li as u32);
        if target >= lo && target < hi && target != pc.wrapping_add(4) { t.push(target); }
    }
    t.sort_unstable();
    t.dedup();
    t
}

/// The function entry `pc` belongs to: the greatest entry at or below it, or `pc` itself when none is.
pub fn function_of(starts: &[u32], pc: u32) -> u32 {
    match starts.binary_search(&pc) { Ok(_) => pc, Err(0) => pc, Err(i) => starts[i - 1] }
}

/// Where a PC lives for the table: a PC inside the scanned code range `[lo, hi)` is its function entry; a PC OUTSIDE it (another
/// fragment: StormCarbon, Miles - the table has no entries for their code) is its 64 KiB region, so library code is not lumped into
/// the last function of the main section. The returned bool is true for a region.
pub fn place(starts: &[u32], range: Option<(u32, u32)>, pc: u32) -> (u32, bool) {
    match range {
        Some((lo, hi)) if pc < lo || pc >= hi => (pc & !0xFFFF, true),
        _ => (function_of(starts, pc), false),
    }
}

/// Samples folded to functions, heaviest instruction share first: (entry, weight).
pub fn by_function(starts: &[u32], range: Option<(u32, u32)>, table: &BTreeMap<u32, Weight>) -> Vec<(u32, bool, Weight)> {
    let mut f: BTreeMap<(u32, bool), Weight> = BTreeMap::new();
    for (&pc, w) in table {
        let e = f.entry(place(starts, range, pc)).or_default();
        e.insns += w.insns;
        e.time += w.time;
    }
    let mut v: Vec<(u32, bool, Weight)> = f.into_iter().map(|((a, r), w)| (a, r, w)).collect();
    v.sort_by(|a, b| b.2.insns.cmp(&a.2.insns).then(a.0.cmp(&b.0)));
    v
}

fn pct10(part: u64, whole: u64) -> String {
    let t = if whole == 0 { 0 } else { part.saturating_mul(1000) / whole };
    format!("{}.{}", t / 10, t % 10)
}

/// The share (in tenths of a percent of instructions) of the samples in functions that made a glue call of `class` (see the module doc).
pub fn caller_share(starts: &[u32], range: Option<(u32, u32)>, table: &BTreeMap<u32, Weight>, callers: &BTreeMap<(u8, u32), u64>, class: u8) -> (u64, u64) {
    // a pointer-glue thunk (the `ptrgl` every call through a function pointer passes) is the caller of record of every native called
    // by pointer; it says nothing about who spins, so functions of at most THUNK bytes are not callers (a region has no size: kept)
    let fns: BTreeSet<(u32, bool)> = callers.keys().filter(|(c, _)| *c == class).map(|&(_, lr)| place(starts, range, lr.wrapping_sub(4)))
        .filter(|&(a, region)| region || starts.binary_search(&a).ok().and_then(|i| starts.get(i + 1)).map_or(true, |n| n - a > THUNK)).collect();
    let total: u64 = table.values().map(|w| w.insns).sum();
    let in_fns: u64 = table.iter().filter(|(&pc, _)| fns.contains(&place(starts, range, pc))).map(|(_, w)| w.insns).sum();
    (in_fns, total)
}

/// The window's `[HEIRLOOM-PERF-PC ...]` line: the top [`TOP`] functions by instruction share (and their interpreter-time share), and
/// the share of samples in functions that read a clock or call the thread manager.
pub fn pc_line(starts: &[u32], range: Option<(u32, u32)>, table: &BTreeMap<u32, Weight>, callers: &BTreeMap<(u8, u32), u64>, frames_total: u64, every: u64) -> String {
    let total: u64 = table.values().map(|w| w.insns).sum();
    let time: u64 = table.values().map(|w| w.time).sum();
    let funcs = by_function(starts, range, table);
    let top: String = funcs.iter().take(TOP).map(|(f, region, w)| format!(" {}={f:#x} {}%/{}%t", if *region { "ext" } else { "fn" }, pct10(w.insns, total), pct10(w.time, time))).collect();
    let ext: u64 = funcs.iter().filter(|f| f.1).map(|f| f.2.insns).sum();
    let (clock, _) = caller_share(starts, range, table, callers, crate::perf::Class::Clock as u8);
    let (thread, _) = caller_share(starts, range, table, callers, crate::perf::Class::Thread as u8);
    format!("[HEIRLOOM-PERF-PC frames={frames_total} every={every} samples_insns={total} fns={} top5:{top} | other_fragments={}% clockfn={}% threadfn={}% (upper bounds: whole functions that call the clock / the thread manager; ext= is a 64 KiB region of another fragment)]",
        funcs.len(), pct10(ext, total), pct10(clock, total), pct10(thread, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bl(pc: u32, target: u32) -> (u32, u32) { (pc, 0x4800_0001 | (target.wrapping_sub(pc) & 0x03FF_FFFC)) }

    #[test]
    fn a_function_entry_is_the_target_of_a_bl_and_a_pc_belongs_to_the_nearest_entry_below_it() {
        // 0x1000 calls 0x1100 and 0x1200 (forward), 0x1200 calls 0x1100 back, 0x1300 calls 0x0f00 (outside the range: not an entry);
        // a bl to the next instruction is the get-pc idiom, an `b` (opcode 18 LK=0) and a `bla` (AA=1) are not calls
        let words = [bl(0x1000, 0x1100), bl(0x1004, 0x1200), bl(0x1210, 0x1100), bl(0x1300, 0x0f00), bl(0x1304, 0x1308), (0x1400, 0x4800_0100), (0x1404, 0x4800_1003)];
        let s = function_starts(words.into_iter(), 0x1000, 0x2000);
        assert_eq!(s, vec![0x1100, 0x1200]);
        assert_eq!(function_of(&s, 0x1100), 0x1100);
        assert_eq!(function_of(&s, 0x11fc), 0x1100);
        assert_eq!(function_of(&s, 0x1350), 0x1200);
        assert_eq!(function_of(&s, 0x1050), 0x1050, "below the first entry a PC is its own function");
    }

    #[test]
    fn samples_fold_to_functions_by_instruction_share_and_the_clock_callers_mark_their_functions() {
        let starts = vec![0x1100, 0x1200];
        let mut t = BTreeMap::new();
        for (pc, insns) in [(0x1104, 600u64), (0x1180, 100), (0x1210, 250), (0x1050, 50)] { t.insert(pc, Weight { insns, time: insns * 2 }); }
        let f = by_function(&starts, Some((0x1000, 0x2000)), &t);
        assert_eq!(f[0], (0x1100, false, Weight { insns: 700, time: 1400 }));
        assert_eq!(f[1].0, 0x1200);
        // 0x1210's function calls the clock (its return address 0x1208 is inside 0x1200): 250 of 1000 instructions
        let mut callers = BTreeMap::new();
        callers.insert((crate::perf::Class::Clock as u8, 0x1208u32), 9u64);
        assert_eq!(caller_share(&starts, Some((0x1000, 0x2000)), &t, &callers, crate::perf::Class::Clock as u8), (250, 1000));
        assert_eq!(caller_share(&starts, Some((0x1000, 0x2000)), &t, &callers, crate::perf::Class::Thread as u8), (0, 1000));
        let line = pc_line(&starts, Some((0x1000, 0x2000)), &t, &callers, 256, EVERY);
        assert!(line.starts_with("[HEIRLOOM-PERF-PC frames=256 every=16381 samples_insns=1000 fns=3 top5: fn=0x1100 70.0%/70.0%t fn=0x1200 25.0%/25.0%t fn=0x1050 5.0%/5.0%t | other_fragments=0.0%"), "{line}");
        assert!(line.contains("clockfn=25.0% threadfn=0.0%"), "{line}");
        // code of another fragment is its 64 KiB region, not the last function of the main section; a 20-byte thunk is no caller
        let mut t2 = t.clone();
        t2.insert(0xcb1234, Weight { insns: 1000, time: 1000 });
        let mut c2 = callers.clone();
        c2.insert((crate::perf::Class::Thread as u8, 0x1304u32), 99); // inside 0x1200's function: counted
        let line = pc_line(&starts, Some((0x1000, 0x2000)), &t2, &c2, 256, EVERY);
        assert!(line.contains("top5: ext=0xcb0000 50.0%/33.3%t fn=0x1100 35.0%/") && line.contains("other_fragments=50.0%"), "{line}");
        let thunk_starts = vec![0x1100, 0x1120, 0x1200]; // 0x1100 is 32 bytes: a thunk
        let mut c3 = BTreeMap::new();
        c3.insert((crate::perf::Class::Clock as u8, 0x1108u32), 5u64);
        assert_eq!(caller_share(&thunk_starts, Some((0x1000, 0x2000)), &t, &c3, crate::perf::Class::Clock as u8).0, 0);
    }

    #[test]
    fn the_cumulative_table_is_bounded_and_a_window_table_keeps_its_own_counts() {
        let mut p = PcProf::new(EVERY);
        let mut win = BTreeMap::new();
        p.record(&mut win, 0x2000, Weight { insns: 10, time: 5 });
        p.record(&mut win, 0x2000, Weight { insns: 10, time: 5 });
        assert_eq!(p.cum[&0x2000], Weight { insns: 20, time: 10 });
        assert_eq!(win[&0x2000], Weight { insns: 20, time: 10 });
        win.clear();
        p.record(&mut win, 0x2000, Weight { insns: 1, time: 1 });
        assert_eq!((p.cum[&0x2000].insns, win[&0x2000].insns), (21, 1));
    }
}
