//! ppc-fuzz [seed] [cases-per-family] [seq-cases]: run the differential fuzz and print discrepancies grouped by
//! (instruction names, differing fields), plus how often each named exclusion fired.
use heirloom_cpu_ppc_oracle::gen;
use heirloom_cpu_ppc_oracle::rng::Rng;
use heirloom_cpu_ppc_oracle::run::{run_case, Verdict};
use heirloom_cpu_ppc_oracle::table::table;
use heirloom_cpu_ppc_oracle::world::Pair;
use std::collections::BTreeMap;

fn main() {
    let a: Vec<u64> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let (seed, per, seqs) = (a.first().copied().unwrap_or(1), a.get(1).copied().unwrap_or(2000), a.get(2).copied().unwrap_or(20000));
    let t = table();
    let mut pair = Pair::new();
    let mut groups: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut excl: BTreeMap<&'static str, u64> = BTreeMap::new();
    let (mut total, mut agree) = (0u64, 0u64);
    let mut run = |c: &gen::Case, label: &str, groups: &mut BTreeMap<String, (u64, String)>| {
        total += 1;
        match run_case(&mut pair, c) {
            Verdict::Agree(_) => agree += 1,
            Verdict::Excluded(ns) => for n in ns { *excl.entry(n).or_default() += 1; },
            Verdict::Differ { step, word, diffs } => {
                let key = format!("{label} :: {}", diffs.iter().take(3).map(|d| d.field.clone()).collect::<Vec<_>>().join(","));
                let e = groups.entry(key).or_insert((0, String::new()));
                e.0 += 1;
                if e.1.is_empty() {
                    let f = |n: u32| format!("f{n}={:016x}", c.st.fpr[n as usize]);
                    let g = |n: u32| format!("r{n}={:08x}", c.st.gpr[n as usize]);
                    let regs = format!("{} {} {} {} {} {} fpscr={:08x} xer={:08x} cr={:08x}", f(word >> 21 & 31), f(word >> 16 & 31), f(word >> 11 & 31), f(word >> 6 & 31), g(word >> 16 & 31), g(word >> 21 & 31), c.st.fpscr, c.st.xer, c.st.cr);
                    let names: Vec<&str> = c.words.iter().map(|w| t.iter().find(|i| i.matches(*w)).map_or("?", |i| i.name)).collect();
                    e.1 = format!("step {step} {names:?} word {word:08x} words={:08x?} initial: {regs}\n   {}", c.words, diffs.iter().take(4).map(|d| format!("{}: heirloom {} oracle {}", d.field, d.heirloom, d.oracle)).collect::<Vec<_>>().join("\n   "));
                }
            }
        }
    };
    for (fi, ins) in t.iter().enumerate() {
        for i in 0..per {
            let mut r = Rng::new(seed.wrapping_mul(1_000_003) ^ (fi as u64) << 32 ^ i);
            let c = gen::single(&mut r, ins);
            run(&c, ins.name, &mut groups);
        }
    }
    for i in 0..seqs {
        let mut r = Rng::new(seed.wrapping_mul(7_000_003) ^ i ^ 1 << 60);
        let c = if i % 8 == 0 { gen::atomic(&mut r) } else { gen::sequence(&mut r, &t) };
        run(&c, "seq", &mut groups);
    }
    println!("cases run: {total}, agree: {agree}, discrepancy groups: {}", groups.len());
    for (n, c) in &excl { println!("excluded {c:6}  {n}"); }
    for (k, (n, ex)) in &groups { println!("{n:6}  {k}\n   {ex}"); }
}
