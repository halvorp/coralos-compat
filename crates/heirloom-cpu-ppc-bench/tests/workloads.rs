//! Every synthetic workload runs to its exit and passes its own result check; each check can fail.
use heirloom_cpu_ppc::Tier;
use heirloom_cpu_ppc_bench::{exec_for, synth, Opts};

#[test]
fn every_synthetic_workload_exits_cleanly_and_passes_its_check() {
    for (mut s, tier) in synth::all().into_iter().flat_map(|s| [Tier::Interp, Tier::Blocks, Tier::Lockstep].map(|t| (s.clone(), t))) {
        s.n = 2_000; // even (the smc model's 3n/2), small enough for a debug build
        let (mut c, mut v) = synth::load(&s);
        let mut e = exec_for(tier, Opts::default());
        let st = synth::run(&mut e, &mut c, &mut v, 1_000);
        assert!(st.halted && st.trap.is_none() && e.divergence().is_none(), "{} on {tier:?}: {st:?} {:?}", s.name, e.divergence());
        (s.check)(&c, &v, s.n).unwrap_or_else(|e| panic!("{}: {e}", s.name));
        assert!(e.prof.insns > 2_000, "{}: it really looped ({} instructions)", s.name, e.prof.insns);
        // NEGATIVE CONTROL: the checks that model a result reject a wrong one
        if matches!(s.name, "synth-int" | "synth-branch" | "synth-callback" | "synth-smc") {
            c.gpr[5] ^= 1;
            assert!((s.check)(&c, &v, s.n).is_err(), "{}: the check cannot fail", s.name);
        }
    }
}
