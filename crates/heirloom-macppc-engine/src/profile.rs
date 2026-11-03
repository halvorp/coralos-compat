//! THE GUEST-PC PROFILE (h86 aot-profile, 2026-11-03; the WC3 council's prototype step 1): where a PowerPC program's
//! instructions and its JIT time go, function by function, so the whole-function translation prototype picks its 3-10
//! targets by MEASURED exclusive time, not by a static guess. Opt-in (cpu-tier "jitprofile"), on the stage clock.
//!
//! HOW. [`CodeMap`] is kept for every fragment the CFM instantiates (cfm.rs): each section's guest base and size, and the
//! function STARTS found in its code - the targets of every `bl` inside the section, every data word that (after relocation)
//! points into it (CFM transition vectors: C++ virtuals and callbacks - the WC3 static map found 15,625 of the game's
//! functions reached only that way and almost no traceback tables), and the entry points and exports. A start is a guess
//! (a data word may point mid-function); the host-side symboliser (tools/ppc-profile-symbolise.py) snaps each to the
//! function of a real map, by (fragment, section offset).
//!
//! The run is SAMPLED: with the profile on, [`crate::MacPpc::step`] takes the guest pc at the end of each CPU-library run
//! - an instruction-count sample - with the run's JIT ticks (the frame ledger's `jit` phase: natives excluded); a slice
//! longer than twice [`CHUNK_MEAN`] is cut into runs of a pseudo-random length (mean [`CHUNK_MEAN`], so a loop cannot alias
//! with the cuts) and sampled after each. The profile's own cost shows in the ledger (its `jit` bucket against a run without
//! it), never hidden. Per function: samples (its share estimates its share of the guest's INSTRUCTIONS) and ticks (each
//! sample weighted by its chunk's cost per instruction: an estimate of its share of JIT TIME). The natives are counted
//! exactly: calls and ticks per glue entry (inclusive of a callback into guest code, rare). Every [`PROFILE_WINDOWS`] perf
//! windows the profile is printed through the stage clock's one output path and restarted.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Mean guest instructions between two samples when a slice is cut (a slice of at most twice this is sampled once, at its
/// end, and not cut). Every cut costs: the budget runs out inside a block or region, which bails to the dispatcher and
/// tier 1 for the slice's tail - on the host harness 50,000 cost +36 % of the JIT's time a frame, so cuts stay rare.
pub const CHUNK_MEAN: u64 = 200_000;
/// Perf windows per printed profile (4 x 256 frames: ~1 minute of WC3 on the Z83).
pub const PROFILE_WINDOWS: u64 = 4;
/// Functions and natives printed.
pub const TOP: usize = 40;
pub const TOP_NATIVES: usize = 20;

/// One instantiated section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sec { pub index: usize, pub code: bool, pub base: u32, pub size: u32 }

/// One fragment as placed in guest memory.
#[derive(Debug, Clone, Default)]
pub struct Frag { pub name: String, pub secs: Vec<Sec>, pub starts: usize }

/// Every instantiated fragment's sections and the function starts found in their code (sorted guest addresses).
#[derive(Debug, Clone, Default)]
pub struct CodeMap {
    pub frags: Vec<Frag>,
    starts: Vec<u32>,
    /// `[HEIRLOOM-CFM ...]` lines not yet taken (one per section at instantiation).
    pub lines: Vec<String>,
}

impl CodeMap {
    /// Record fragment `name`: its sections (`(index, is_code, base, size)`), its code read through `word`, the extra starts
    /// `entries` (entry points, exports). Returns the starts it found.
    pub fn add(&mut self, name: &str, secs: &[(usize, bool, u32, u32)], word: &dyn Fn(u32) -> Option<u32>, entries: &[u32]) -> usize {
        let code: Vec<(u32, u32)> = secs.iter().filter(|s| s.1).map(|s| (s.2, s.2.saturating_add(s.3))).collect();
        let in_code = |a: u32| a & 3 == 0 && code.iter().any(|&(lo, hi)| a >= lo && a < hi);
        let mut found: Vec<u32> = entries.iter().copied().filter(|&a| in_code(a)).collect();
        for &(lo, hi, is_code) in secs.iter().map(|s| (s.2, s.2.saturating_add(s.3), s.1)).collect::<Vec<_>>().iter() {
            let mut at = lo;
            while at.saturating_add(4) <= hi {
                if let Some(w) = word(at) {
                    if is_code {
                        // bl target (I-form, opcode 18, AA = 0, LK = 1): a function start in this fragment's code
                        if w >> 26 == 18 && w & 3 == 1 {
                            let rel = ((w & 0x03FF_FFFC) as i32) << 6 >> 6;
                            let t = at.wrapping_add(rel as u32);
                            if in_code(t) { found.push(t); }
                        }
                    } else if in_code(w) {
                        found.push(w); // a code pointer in data: a TVector's code word (or a jump table's target)
                    }
                }
                at += 4;
            }
        }
        for &(lo, _) in &code { found.push(lo); }
        found.sort_unstable();
        found.dedup();
        let n = found.len();
        for s in secs {
            self.lines.push(format!("[HEIRLOOM-CFM frag={name:?} sec={} kind={} base={:#x} size={}{}]", s.0, if s.1 { "code" } else { "data" }, s.2, s.3,
                if s.1 { format!(" starts={n}") } else { String::new() }));
        }
        self.frags.push(Frag { name: name.into(), secs: secs.iter().map(|&(index, code, base, size)| Sec { index, code, base, size }).collect(), starts: n });
        self.starts.extend(found);
        self.starts.sort_unstable();
        self.starts.dedup();
        n
    }

    /// The function start at or below `pc` inside a code section, and that section's fragment index: None outside every
    /// instantiated code section (the glue stubs, the heap).
    pub fn function_of(&self, pc: u32) -> Option<(usize, u32)> {
        let f = self.frags.iter().position(|f| f.secs.iter().any(|s| s.code && pc >= s.base && pc < s.base.saturating_add(s.size)))?;
        let i = self.starts.partition_point(|&s| s <= pc);
        let start = *self.starts.get(i.checked_sub(1)?)?;
        Some((f, start))
    }

    /// (fragment name, section index, section offset) of a guest address in a code section.
    pub fn locate(&self, a: u32) -> Option<(&str, usize, u32)> {
        self.frags.iter().find_map(|f| f.secs.iter().find(|s| s.code && a >= s.base && a < s.base.saturating_add(s.size)).map(|s| (f.name.as_str(), s.index, a - s.base)))
    }
}

/// One function's (or bucket's) account: samples, guest instructions and JIT ticks attributed to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Acc { pub samples: u64, pub insns: u64, pub ticks: u64 }

/// The running profile (on the stage clock: `crate::perf::Perf::profile`).
#[derive(Debug, Clone, Default)]
pub struct Profile {
    /// By function start (u32::MAX = a pc outside every code section).
    pub by_fn: BTreeMap<u32, Acc>,
    /// Natives by glue id: (calls, ticks).
    pub natives: Vec<(u64, u64)>,
    pub samples: u64,
    pub frames: u64,
    lfsr: u32,
}

impl Profile {
    /// The next chunk's length: CHUNK_MEAN +- 50 %, from a 32-bit LFSR (a loop's period cannot line up with it).
    pub fn next_chunk(&mut self) -> u64 {
        if self.lfsr == 0 { self.lfsr = 0xACE1_2468; }
        self.lfsr ^= self.lfsr << 13;
        self.lfsr ^= self.lfsr >> 17;
        self.lfsr ^= self.lfsr << 5;
        CHUNK_MEAN / 2 + u64::from(self.lfsr) % CHUNK_MEAN
    }
    /// A chunk ended at `pc` after `insns` instructions and `ticks` of JIT time.
    pub fn sample(&mut self, map: &CodeMap, pc: u32, insns: u64, ticks: u64) {
        let key = map.function_of(pc).map_or(u32::MAX, |(_, s)| s);
        let a = self.by_fn.entry(key).or_default();
        (a.samples, a.insns, a.ticks) = (a.samples + 1, a.insns + insns, a.ticks + ticks);
        self.samples += 1;
    }
    /// A native call of glue entry `id` that took `ticks`.
    pub fn native(&mut self, id: usize, ticks: u64) {
        if self.natives.len() <= id && self.natives.try_reserve(id + 1 - self.natives.len()).is_err() { return; }
        if self.natives.len() <= id { self.natives.resize(id + 1, (0, 0)); }
        let n = &mut self.natives[id];
        (n.0, n.1) = (n.0 + 1, n.1 + ticks);
    }
}

fn pct(a: u64, b: u64) -> String { if b == 0 { String::from("0.0") } else { let x = a.saturating_mul(1000) / b; format!("{}.{}", x / 10, x % 10) } }

/// The printed profile: a header with the map, [`TOP`] functions by JIT ticks (four lines of ten), [`TOP_NATIVES`] natives by
/// ticks; `name_of(id)` names a glue entry. At `per_us` clock units a microsecond; `frames_total` names the lines.
pub fn lines(p: &Profile, map: &CodeMap, name_of: &dyn Fn(usize) -> String, per_us: u64, frames_total: u64) -> Vec<String> {
    let per_us = per_us.max(1);
    let frames = p.frames.max(1);
    let total_ticks: u64 = p.by_fn.values().map(|a| a.ticks).sum();
    let mut out = Vec::new();
    let frags: Vec<String> = map.frags.iter().enumerate().map(|(i, f)| {
        let c: Vec<String> = f.secs.iter().filter(|s| s.code).map(|s| format!("sec{} {:#x}+{}", s.index, s.base, s.size)).collect();
        format!("f{i}={:?} {} starts={}", f.name, c.join(" "), f.starts)
    }).collect();
    out.push(format!("[HEIRLOOM-PROFILE frames={frames_total} profiled={} samples={} chunk_mean={CHUNK_MEAN} jit_us/frame={}] map: {}",
        p.frames, p.samples, total_ticks / per_us / frames, frags.join(" | ")));
    let mut f: Vec<(&u32, &Acc)> = p.by_fn.iter().collect();
    f.sort_by(|a, b| b.1.ticks.cmp(&a.1.ticks));
    let cell = |(&start, a): (&u32, &Acc)| {
        let at = map.locate(start).map_or_else(|| String::from("outside"), |(_, _, off)| {
            let fi = map.frags.iter().position(|fr| fr.secs.iter().any(|s| s.code && start >= s.base && start < s.base.saturating_add(s.size))).unwrap_or(0);
            format!("f{fi}+{off:#x}")
        });
        format!("{at} t={}% s={}% n={}", pct(a.ticks, total_ticks), pct(a.samples, p.samples), a.samples)
    };
    for (k, chunk) in f.iter().take(TOP).collect::<Vec<_>>().chunks(10).enumerate() {
        let cells: Vec<String> = chunk.iter().map(|e| cell(**e)).collect();
        out.push(format!("[HEIRLOOM-PROFILE-TOP frames={frames_total} rank={}-{}] {}", k * 10 + 1, k * 10 + chunk.len(), cells.join(" | ")));
    }
    let mut n: Vec<(usize, &(u64, u64))> = p.natives.iter().enumerate().filter(|(_, c)| c.0 > 0).collect();
    n.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    let cells: Vec<String> = n.iter().take(TOP_NATIVES).map(|(id, c)| format!("{} calls/frame={} us/frame={}", name_of(*id), c.0 / frames, c.1 / per_us / frames)).collect();
    out.push(format!("[HEIRLOOM-PROFILE-NATIVE frames={frames_total}] {}", cells.join(" | ")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code section with two functions (the second reached by a `bl` from the first, a third only through a data word, as a
    /// TVector reaches it), and a data section holding that pointer: the starts are the section start, the bl target and the
    /// data pointer; a pc maps to the start at or below it.
    #[test]
    fn the_code_map_finds_bl_targets_and_tvector_code_words() {
        let code = 0x1000u32;
        let mut mem = BTreeMap::new();
        let bl = |from: u32, to: u32| 18 << 26 | (to.wrapping_sub(from) & 0x03FF_FFFC) | 1;
        mem.insert(code, 0x7C08_02A6u32); // mflr r0
        mem.insert(code + 4, bl(code + 4, code + 0x40));
        mem.insert(code + 8, 0x4E80_0020); // blr
        mem.insert(code + 0x40, 0x4E80_0020);
        mem.insert(code + 0x80, 0x4E80_0020);
        mem.insert(0x2000, code + 0x80); // a TVector's code word
        mem.insert(0x2004, 0x2100); // its TOC: not code
        let word = |a: u32| Some(mem.get(&a).copied().unwrap_or(0));
        let mut m = CodeMap::default();
        let n = m.add("App", &[(0, true, code, 0x100), (1, false, 0x2000, 0x10)], &word, &[]);
        assert_eq!(n, 3);
        assert_eq!(m.function_of(code + 8), Some((0, code)));
        assert_eq!(m.function_of(code + 0x44), Some((0, code + 0x40)));
        assert_eq!(m.function_of(code + 0x90), Some((0, code + 0x80)));
        assert_eq!(m.function_of(0x2000), None, "data is no function");
        assert_eq!(m.locate(code + 0x40), Some(("App", 0, 0x40)));
        assert!(m.lines[0].starts_with("[HEIRLOOM-CFM frag=\"App\" sec=0 kind=code base=0x1000 size=256 starts=3]"), "{:?}", m.lines);
    }

    /// Samples attribute instructions and ticks to the function at the pc; the lines rank by ticks and name the natives.
    #[test]
    fn the_profile_ranks_functions_by_jit_ticks_and_counts_natives_exactly() {
        let mut m = CodeMap::default();
        let word = |_: u32| Some(0u32);
        m.add("App", &[(0, true, 0x1000, 0x1000)], &word, &[0x1000, 0x1800]);
        let mut p = Profile::default();
        for _ in 0..3 { p.sample(&m, 0x1004, 100, 1000); }
        p.sample(&m, 0x1900, 100, 9000);
        p.native(2, 500);
        p.native(2, 700);
        p.frames = 2;
        let l = lines(&p, &m, &|id| format!("n{id}"), 1, 512);
        assert!(l[0].starts_with("[HEIRLOOM-PROFILE frames=512 profiled=2 samples=4 chunk_mean=200000 jit_us/frame=6000] map: f0=\"App\" sec0 0x1000+4096 starts=2"), "{}", l[0]);
        assert!(l[1].starts_with("[HEIRLOOM-PROFILE-TOP frames=512 rank=1-2] f0+0x800 t=75.0% s=25.0% n=1 | f0+0x0 t=25.0% s=75.0% n=3"), "{}", l[1]);
        assert_eq!(l[2], "[HEIRLOOM-PROFILE-NATIVE frames=512] n2 calls/frame=1 us/frame=600");
        let c: Vec<u64> = (0..1000).map(|_| p.next_chunk()).collect();
        assert!(c.iter().all(|&n| (CHUNK_MEAN / 2..CHUNK_MEAN * 3 / 2).contains(&n)) && c.windows(2).any(|w| w[0] != w[1]));
    }
}
