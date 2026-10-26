//! The differential fuzz as a test: fixed seeds, every instruction family, then short random sequences; plus the
//! corpus replay and negative controls for the comparator and the exclusions.
use heirloom_cpu_ppc_oracle::compare::{diff, Relax};
use heirloom_cpu_ppc_oracle::corpus;
use heirloom_cpu_ppc_oracle::exclude::{relax, trap_excuse};
use heirloom_cpu_ppc_oracle::gen;
use heirloom_cpu_ppc_oracle::rng::Rng;
use heirloom_cpu_ppc_oracle::run::{run_case, Verdict};
use heirloom_cpu_ppc_oracle::table::table;
use heirloom_cpu_ppc_oracle::world::{Pair, Tr};
use heirloom_cpu_ppc::Tier;
use std::collections::BTreeMap;

const PER_FAMILY: u64 = 150;
const SEQUENCES: u64 = 3000;

#[test]
fn every_family_agrees_with_ppc_rs_apart_from_the_named_exclusions() { sweep(Pair::new()) }

/// The same sweep on each heirloom-cpu-ppc tier explicitly, whatever the default and HEIRLOOM_PPC_TIER say: tier 0 is the
/// reference, tier 1 must agree with ppc-rs exactly as it does, and lockstep checks tier 1 against tier 0 on top.
#[test]
fn every_family_agrees_on_tier_0_tier_1_and_in_lockstep() {
    for t in [Tier::Interp, Tier::Blocks, Tier::Lockstep] {
        if t == heirloom_cpu_ppc_oracle::world::tier_from_env() { continue; } // the test above ran it
        let p = Pair::with_tier(t);
        assert_eq!(p.tier(), t);
        sweep(p);
    }
}

/// NEGATIVE CONTROL: tier 1 that never invalidates decoded code goes RED within a few cases (each case loads new code
/// where the last one's was, and sequences patch their own next instruction).
#[test]
fn the_stale_code_mutant_is_caught() {
    let t = table();
    let mut pair = Pair::with_tier(Tier::Blocks);
    pair.plant_stale_code_mutant();
    let mut caught = None;
    for i in 0..2_000u64 {
        let mut r = Rng::new(0x5EED ^ i);
        let c = gen::single(&mut r, &t[(i as usize * 7) % t.len()]);
        if let Verdict::Differ { .. } = run_case(&mut pair, &c) { caught = Some(i); break; }
    }
    let i = caught.expect("the stale-code mutant survived 2000 cases: the oracle cannot see stale decoded code");
    eprintln!("stale-code mutant caught at case {i}");
    assert!(i < 50, "caught only at case {i}");
}

fn sweep(mut pair: Pair) {
    let t = table();
    let mut agree: BTreeMap<&str, u64> = BTreeMap::new();
    let mut excl: BTreeMap<&str, u64> = BTreeMap::new();
    let mut compared: BTreeMap<&str, u64> = BTreeMap::new();
    let mut total = 0;
    for (fi, ins) in t.iter().enumerate() {
        for i in 0..PER_FAMILY {
            let mut r = Rng::new(0x5EED ^ (fi as u64) << 32 ^ i);
            let c = gen::single(&mut r, ins);
            total += 1;
            match run_case(&mut pair, &c) {
                Verdict::Agree(_) => { *agree.entry(ins.name).or_default() += 1; *compared.entry(ins.name).or_default() += 1 }
                Verdict::Excluded(ns) => { *compared.entry(ins.name).or_default() += 1; for n in ns { *excl.entry(n).or_default() += 1 } }
                Verdict::Differ { step, word, diffs } => panic!("{}: step {step} word {word:08x} words {:08x?}: {diffs:?}", ins.name, c.words),
            }
        }
    }
    for i in 0..SEQUENCES {
        let mut r = Rng::new(0xBEEF ^ i << 8);
        let c = if i % 8 == 0 { gen::atomic(&mut r) } else { gen::sequence(&mut r, &t) };
        total += 1;
        if let Verdict::Differ { step, word, diffs } = run_case(&mut pair, &c) {
            panic!("sequence {i}: step {step} word {word:08x} words {:08x?}: {diffs:?}", c.words);
        }
    }
    // Coverage: every family must have been compared at least once. An excluded case is still compared under the
    // relaxation (the rest of the state must match); only a case that never reached the comparison would be missing.
    for ins in &t {
        assert!(compared.get(ins.name).copied().unwrap_or(0) > 0, "{} was never compared", ins.name);
    }
    // And the families whose every case needs a relaxation are the ones with an undefined result field.
    for ins in &t {
        if agree.get(ins.name).copied().unwrap_or(0) == 0 {
            assert!(matches!(ins.name, "mffs"), "{} never agreed unrelaxed; add it to this list only with a reason", ins.name);
        }
    }
    assert!(t.len() >= 170, "{} families", t.len());
    eprintln!("{} families, {total} cases; exclusions applied: {excl:?}", t.len());
}

#[test]
fn the_regression_corpus_keeps_its_verdicts() {
    let entries = corpus::parse(include_str!("../corpus/regressions.txt"));
    assert!(entries.len() >= 200, "{} corpus cases", entries.len());
    let mut pair = Pair::new();
    for e in &entries {
        let v = run_case(&mut pair, &e.case);
        assert_eq!(corpus::expect_of(&v), e.expect, "corpus case {}", e.label);
    }
    // Every named exclusion that can fire is witnessed by the corpus (the fuzz asserts they stay narrow).
    let text = include_str!("../corpus/regressions.txt");
    for name in ["div-undefined-result", "fp-int-result-high-word-undefined", "ppcrs-nan-selection", "ppcrs-stfs-rounds", "ppcrs-fctiw-nan",
                 "ppcrs-frsp-deep-underflow", "ppcrs-fprf-single-denormal", "ppcrs-lfs-quiets-snan", "heirloom-rn-directed-rounding",
                 "heirloom-untracked-fpscr-xx-ux-fi-fr", "single-op-nonsingle-operand", "invalid-form-lsw-overlap", "mfspr-time-base"] {
        assert!(text.contains(name), "corpus has no case for exclusion {name}");
    }
}

fn word_pair(word: u32) -> (Pair, heirloom_cpu_ppc_oracle::world::Snap, heirloom_cpu_ppc_oracle::world::Snap) {
    let mut p = Pair::new();
    let mut r = Rng::new(7);
    let st = heirloom_cpu_ppc_oracle::state::State::random(&mut r);
    p.load(&[word], &st, false);
    let (a, b) = p.now();
    (p, a, b)
}

/// NEGATIVE CONTROL: the comparator sees a single flipped bit in every compared field, and a trap mismatch.
#[test]
fn the_comparator_detects_a_difference_in_each_field() {
    let (_, a, b) = word_pair(0x7c632214); // add r3,r3,r4
    assert!(diff(&a, &b, &Relax::full()).is_empty(), "identical states compare equal");
    let m = |f: &dyn Fn(&mut heirloom_cpu_ppc_oracle::world::Snap), name: &str| {
        let mut c = b.clone();
        f(&mut c);
        let d = diff(&a, &c, &Relax::full());
        assert!(d.iter().any(|x| x.field.starts_with(name)), "{name}: {d:?}");
    };
    m(&|s| s.gpr[17] ^= 1, "r17");
    m(&|s| s.fpr[3] ^= 1 << 63, "f3");
    m(&|s| s.cr ^= 1, "cr");
    m(&|s| s.xer ^= 0x2000_0000, "xer");
    m(&|s| s.lr ^= 4, "lr");
    m(&|s| s.ctr ^= 1, "ctr");
    m(&|s| s.fpscr ^= 1 << 12, "fpscr");
    m(&|s| s.pc += 4, "pc");
    m(&|s| s.mem[0x1234] ^= 1, "mem");
    m(&|s| s.trap = Tr::Sys, "trap");
}

/// NEGATIVE CONTROL: a relaxation masks only what it names; unrelated instructions get the full comparison, and a
/// heirloom-cpu-ppc store that disagrees with the PEM model is a discrepancy, not an exclusion.
#[test]
fn exclusions_are_narrow() {
    let (_, a, b) = word_pair(0x7c632214); // add
    let r = relax(0x7c632214, &a, &a, &b);
    assert!(r.names.is_empty() && r.cr == !0 && r.fpscr == !0 && r.gpr.iter().all(|m| *m == !0), "add is fully compared");
    // divw by zero relaxes exactly rD and CR0[LT,GT,EQ].
    let mut pre = a.clone();
    pre.gpr[4] = 0;
    let r = relax(0x7c6323d6, &pre, &pre, &pre); // divw r3,r3,r4
    assert_eq!(r.names, vec!["div-undefined-result"]);
    assert_eq!((r.gpr[3], r.gpr[4], r.cr), (0, !0, !0xE000_0000));
    // stfs: a stored word that is not the PEM truncation is reported.
    let mut pre = a.clone();
    pre.gpr[3] = 0x1800;
    pre.fpr[1] = (1.0f64 + 0.75 * 2f64.powi(-23)).to_bits();
    let mut post = pre.clone();
    post.mem[0x1800..0x1804].copy_from_slice(&0x3F80_0001u32.to_be_bytes()); // rounded, not truncated
    let r = relax(52 << 26 | 1 << 21 | 3 << 16, &pre, &post, &post);
    assert_eq!(r.model_fail.len(), 1, "{:?}", r.model_fail);
    post.mem[0x1800..0x1804].copy_from_slice(&0x3F80_0000u32.to_be_bytes());
    assert!(relax(52 << 26 | 1 << 21 | 3 << 16, &pre, &post, &post).model_fail.is_empty(), "CONTROL: the PEM word passes");
    // trap excuses: only the named kinds. A bus fault on a plain lwz is never excused.
    let mut x = a.clone();
    x.trap = Tr::Bus { write: false };
    assert_eq!(trap_excuse(32 << 26, &x, &a), None);
    let mut y = a.clone();
    y.trap = Tr::Align;
    assert_eq!(trap_excuse(46 << 26, &y, &a), Some("misaligned-multiple-or-reserved"));
    assert_eq!(trap_excuse(32 << 26, &y, &a), None, "an alignment trap on lwz is not excused");
}

/// lfs: the relaxation applies only to a loaded signalling NaN that heirloom left as the PEM load conversion says; a
/// heirloom that corrupts bit 51 of an ordinary loaded single is a discrepancy (mutation witnessed 2026-10-18).
#[test]
fn lfs_exclusion_is_narrow_and_model_checked() {
    use heirloom_cpu_ppc_oracle::exclude::fp_mode_excuse;
    let (_, a, _) = word_pair(0x7c632214);
    let lfs = 48 << 26 | 1 << 21 | 3 << 16; // lfs f1,0(r3)
    let setup = |word: u32, heirloom_f1: u64, oracle_f1: u64| {
        let mut pre = a.clone();
        pre.gpr[3] = 0x1800;
        pre.mem[0x1800..0x1804].copy_from_slice(&word.to_be_bytes());
        let (mut rt, mut or) = (pre.clone(), pre.clone());
        rt.fpr[1] = heirloom_f1;
        or.fpr[1] = oracle_f1;
        relax(lfs, &pre, &rt, &or)
    };
    let snan = 0x7F80_0001u32;
    let m = 0x7FF0_0000_2000_0000u64; // PEM: fraction copied unchanged
    let r = setup(snan, m, m | 1 << 51);
    assert!(r.model_fail.is_empty() && r.names == ["ppcrs-lfs-quiets-snan"] && r.fpr[1] == !(1 << 51), "{r:?}");
    // CONTROL: heirloom quieted the SNaN itself -> not the model -> a discrepancy.
    assert_eq!(setup(snan, m | 1 << 51, m | 1 << 51).model_fail.len(), 1);
    // CONTROL: an ordinary single with bit 51 flipped by heirloom is a discrepancy, and nothing is relaxed.
    let one = 0x3F80_0000u32;
    let good = 1.0f64.to_bits();
    let r = setup(one, good ^ 1 << 51, good);
    assert_eq!(r.model_fail.len(), 1, "{r:?}");
    let r = setup(one, good, good);
    assert!(r.model_fail.is_empty() && r.names.is_empty() && r.fpr[1] == !0, "an agreeing ordinary lfs is fully compared");
    let _ = fp_mode_excuse; // (see fp_mode_exclusions_are_narrow)
}

/// The OX/UX whole-case exclusion fires only for a real overflow / underflow under an enabled exception.
#[test]
fn fp_mode_exclusions_are_narrow() {
    use heirloom_cpu_ppc_oracle::exclude::fp_mode_excuse;
    let (_, a, _) = word_pair(0x7c632214);
    let fo = |op: u32, xo: u32, frt: u32, fra: u32, frb: u32, frc: u32| op << 26 | frt << 21 | fra << 16 | frb << 11 | frc << 6 | xo << 1;
    let case = |w: u32, fpscr: u32, ops: [(usize, f64); 3], res: f64| {
        let mut pre = a.clone();
        pre.fpscr = fpscr;
        for (r, v) in ops { pre.fpr[r] = v.to_bits(); }
        let mut post = pre.clone();
        post.fpr[1] = res.to_bits();
        fp_mode_excuse(w, &pre, &post)
    };
    const OE: u32 = 1 << 6;
    const UE: u32 = 1 << 5;
    let fadd = fo(63, 21, 1, 2, 3, 0);
    let fdiv = fo(63, 18, 1, 2, 3, 0);
    let fmul = fo(63, 25, 1, 2, 0, 3);
    let n = "heirloom-unwrapped-result-ox-ux-enabled";
    // overflow: fadd MAX+MAX = inf under OE is excused; the control without OE is not; fdiv x/0 is ZX, not OX.
    assert_eq!(case(fadd, OE, [(2, f64::MAX), (3, f64::MAX), (0, 0.0)], f64::INFINITY), Some(n));
    assert_eq!(case(fadd, 0, [(2, f64::MAX), (3, f64::MAX), (0, 0.0)], f64::INFINITY), None, "CONTROL: OE clear");
    assert_eq!(case(fdiv, OE, [(2, 1.0), (3, 0.0), (0, 0.0)], f64::INFINITY), None, "x/0 is ZX");
    assert_eq!(case(fdiv, OE, [(2, f64::MAX), (3, 0.5), (0, 0.0)], f64::INFINITY), Some(n), "CONTROL: a real overflowing quotient");
    // underflow: a tiny product is excused; an exact zero (0 + -0, 0 * x, a cancelling sum) is not.
    assert_eq!(case(fmul, UE, [(2, 1e-200), (0, 0.0), (3, 1e-200)], 0.0), Some(n), "product flushed to zero");
    assert_eq!(case(fmul, UE, [(2, 0.0), (0, 0.0), (3, 1e-200)], 0.0), None, "0 * x is exact");
    assert_eq!(case(fadd, UE, [(2, 0.0), (3, -0.0), (0, 0.0)], 0.0), None, "0 + -0 is exact");
    assert_eq!(case(fadd, UE, [(2, 1.5), (3, -1.5), (0, 0.0)], 0.0), None, "cancellation is exact");
    assert_eq!(case(fadd, UE, [(2, 5e-324), (3, 1e-310), (0, 0.0)], 1e-310 + 5e-324), Some(n), "tiny nonzero result");
    // a trapped (refused) instruction is never excused here
    let mut pre = a.clone();
    pre.fpscr = OE | UE;
    let mut post = pre.clone();
    post.trap = Tr::Illegal;
    assert_eq!(fp_mode_excuse(fadd, &pre, &post), None);
}

/// The reference models are arithmetic, so check them against hand-computed words.
#[test]
fn the_reference_models_match_hand_computed_words() {
    use heirloom_cpu_ppc_oracle::models::{fprf_denormal, lfs_double, stfs_word};
    assert_eq!(stfs_word((1.0f64 + 0.75 * 2f64.powi(-23)).to_bits()), Some(0x3F80_0000), "truncated, not rounded");
    assert_eq!(stfs_word((-1.0f64 - 0.75 * 2f64.powi(-23)).to_bits()), Some(0xBF80_0000));
    assert_eq!(stfs_word(2f64.powi(-127).to_bits()), Some(0x0040_0000), "exp 896: denormal single");
    assert_eq!(stfs_word((1.9 * 2f64.powi(-149)).to_bits()), Some(1), "denormal truncates toward zero");
    assert_eq!(stfs_word((-(2f64.powi(-149))).to_bits()), Some(0x8000_0001));
    assert_eq!(stfs_word(2f64.powi(-150).to_bits()), None, "below the smallest denormal is undefined");
    assert_eq!(stfs_word((-0.0f64).to_bits()), Some(0x8000_0000));
    assert_eq!(lfs_double(0x7F80_0001), 0x7FF0_0000_2000_0000, "SNaN fraction kept");
    assert_eq!(lfs_double(0xFF80_0000), 0xFFF0_0000_0000_0000, "-inf");
    assert_eq!(lfs_double(1), 2f64.powi(-149).to_bits(), "smallest denormal widens exactly");
    assert_eq!(fprf_denormal(2f64.powi(-130).to_bits()), 0x1_4000);
    assert_eq!(fprf_denormal((-(2f64.powi(-130))).to_bits()), 0x1_8000);
}
