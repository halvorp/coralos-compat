//! `ppc-bench [--tiers 0,1] [--reps N] [--only SUBSTR] [--census] [--no-apps]`
//!
//! Runs every workload on every requested tier `--reps` times (default 3) and reports the MEDIAN elapsed time per
//! workload and tier: instructions retired, native callouts, elapsed, guest instructions/s, a `[PPC-PROF ...]` line,
//! the final-state digest, and (with two tiers) each tier's speedup over the first and the geometric mean. A digest
//! that differs from the first tier's is reported as a DIVERGENCE: that workload's speed number means nothing.
//! `--census` instead runs tier 0 once per workload with the opcode/block census and prints it; `--natives` runs
//! tier 0 once per workload timing every native callout and prints the share of elapsed time spent in them.
use heirloom_cpu_ppc::Tier;
use heirloom_cpu_ppc_bench::{apps, geomean, run_app, run_synth, synth, Measure, Opts};

fn usage() -> ! { eprintln!("usage: ppc-bench [--tiers 0,1] [--reps N] [--only SUBSTR] [--census] [--natives] [--no-apps]"); std::process::exit(2) }

fn census_report(name: &str, e: &heirloom_cpu_ppc::Exec) {
    let Some(c) = e.census() else { return };
    let total = c.total().max(1);
    println!("== census {name}: {} guest instructions, {} native calls", c.total(), c.natives);
    for (n, k) in c.top_ops(30) { println!("   {n:<10} {k:>12}  {:5.2}%", 100.0 * k as f64 / total as f64); }
    let (distinct, entries, insns) = c.block_summary();
    println!("   blocks: {distinct} distinct, {entries} dynamic entries, mean dynamic length {:.2} instructions", insns as f64 / entries.max(1) as f64);
    for (pc, e, i) in c.top_blocks(8) { println!("   block {pc:#010x}: {e} entries, {i} instructions ({:.1}%)", 100.0 * i as f64 / total as f64); }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut tiers, mut reps, mut only, mut census, mut natives, mut with_apps) = (vec![Tier::Interp], 3usize, None::<String>, false, false, true);
    while !args.is_empty() {
        let a = args.remove(0);
        match a.as_str() {
            "--tiers" => { if args.is_empty() { usage() } tiers = args.remove(0).split(',').map(|t| Tier::parse(t).unwrap_or_else(|| usage())).collect(); }
            "--reps" => { if args.is_empty() { usage() } reps = args.remove(0).parse().unwrap_or_else(|_| usage()); }
            "--only" => { if args.is_empty() { usage() } only = Some(args.remove(0)); }
            "--census" => census = true,
            "--natives" => natives = true,
            "--no-apps" => with_apps = false,
            _ => usage(),
        }
    }
    let wanted = |n: &str| only.as_deref().is_none_or(|o| n.contains(o));
    // (name, runner) for every workload that exists here
    type Runner = Box<dyn Fn(Tier, Opts) -> Result<(Measure, heirloom_cpu_ppc::Exec), String>>;
    let mut work: Vec<(String, Runner)> = Vec::new();
    for s in synth::all() {
        if !wanted(s.name) { continue; }
        let name = s.name.to_string();
        work.push((name, Box::new(move |t, c| Ok(run_synth(&s, t, c)))));
    }
    if with_apps {
        for a in apps::all() {
            if !wanted(&a.name) { continue; }
            if !apps::present(&a) { println!("SKIP {}: {} is absent (corpus root {})", a.name, a.path, apps::root().display()); continue; }
            let name = a.name.clone();
            work.push((name, Box::new(move |t, c| run_app(&a, t, c))));
        }
    }
    if census {
        for (name, run) in &work {
            match run(Tier::Interp, Opts { census: true, natives: false }) { Ok((_, e)) => census_report(name, &e), Err(e) => println!("FAIL {name}: {e}") }
        }
        return;
    }
    if natives {
        for (name, run) in &work {
            let (m, _) = match run(Tier::Interp, Opts { census: false, natives: true }) { Ok(r) => r, Err(e) => { println!("FAIL {name}: {e}"); continue } };
            let in_natives = m.prof.native_time as f64 / 1e3;
            println!("== natives {name}: {} calls, {:.0} us of {} us elapsed inside them ({:.1}%), {:.2} us per call; guest code alone {:.2} Mips",
                m.natives, in_natives, m.elapsed_us, 100.0 * in_natives / m.elapsed_us.max(1) as f64, in_natives / m.natives.max(1) as f64,
                (m.insns - m.natives) as f64 / (m.elapsed_us as f64 - in_natives).max(1.0));
            for (s, n, t) in m.native_report.iter().take(12) { println!("   {s:<48} {n:>9} calls {:>10.0} us  {:6.2} us/call", *t as f64 / 1e3, *t as f64 / 1e3 / *n as f64); }
        }
        return;
    }
    let mut speedups: Vec<Vec<f64>> = vec![Vec::new(); tiers.len()];
    println!("{:<24} {:>4} {:>12} {:>9} {:>11} {:>10}  {:<18} outcome", "workload", "tier", "insns", "natives", "elapsed_us", "Mips", "digest");
    for (name, run) in &work {
        let mut per_tier: Vec<Measure> = Vec::new();
        for &t in &tiers {
            let mut ms: Vec<Measure> = Vec::new();
            for _ in 0..reps.max(1) {
                match run(t, Opts::default()) { Ok((m, _)) => ms.push(m), Err(e) => { println!("FAIL {name} tier {}: {e}", t.number()); break; } }
            }
            if ms.is_empty() { continue; }
            ms.sort_by_key(|m| m.elapsed_us);
            let m = ms[ms.len() / 2].clone();
            println!("{:<24} {:>4} {:>12} {:>9} {:>11} {:>10.2}  {:016x}   {}", name, t.number(), m.insns, m.natives, m.elapsed_us, m.ips() / 1e6, m.digest, m.outcome);
            println!("    {}", m.prof.line(m.elapsed_us, t));
            if t != Tier::Interp { println!("    tier 1: {} blocks run ({:.2} instructions each), {:?}{}", m.prof.blocks, (m.insns - m.natives) as f64 / m.prof.blocks.max(1) as f64, m.cache, if t == Tier::Lockstep { format!(", lockstep {:?}", m.lockstep) } else { String::new() }); }
            per_tier.push(m);
        }
        if let Some(base) = per_tier.first().cloned() {
            for (i, m) in per_tier.iter().enumerate().skip(1) {
                if m.digest != base.digest || m.insns != base.insns {
                    println!("    DIVERGENCE: tier {} ended in a different state from tier {} (digest/insns): no speed is reported", m.tier.number(), base.tier.number());
                    continue;
                }
                let s = base.elapsed_us as f64 / m.elapsed_us.max(1) as f64;
                if base.insns < 500_000 { println!("    (not a CPU workload: {} instructions; left out of the geomean) {s:.2}x", base.insns); continue; }
                println!("    speedup tier {} over tier {}: {s:.2}x", m.tier.number(), base.tier.number());
                speedups[i].push(s);
            }
        }
    }
    for (i, t) in tiers.iter().enumerate().skip(1) {
        let v = &speedups[i];
        let worst = v.iter().cloned().fold(f64::INFINITY, f64::min);
        println!("GEOMEAN speedup tier {} over tier {}: {:.2}x over {} workloads (worst {:.2}x)", t.number(), tiers[0].number(), geomean(v), v.len(), worst);
    }
}
