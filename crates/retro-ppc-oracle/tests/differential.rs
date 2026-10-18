//! The differential fuzz as a test: fixed seeds, every instruction family, then short random sequences; plus the
//! corpus replay and negative controls for the comparator and the exclusions.
use retro_ppc_oracle::compare::{diff, Relax};
use retro_ppc_oracle::corpus;
use retro_ppc_oracle::exclude::{relax, trap_excuse};
use retro_ppc_oracle::gen;
use retro_ppc_oracle::rng::Rng;
use retro_ppc_oracle::run::{run_case, Verdict};
use retro_ppc_oracle::table::table;
use retro_ppc_oracle::world::{Pair, Tr};
use std::collections::BTreeMap;

const PER_FAMILY: u64 = 150;
const SEQUENCES: u64 = 3000;

#[test]
fn every_family_agrees_with_ppc_rs_apart_from_the_named_exclusions() {
    let t = table();
    let mut pair = Pair::new();
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
                 "ppcrs-frsp-deep-underflow", "ppcrs-fprf-single-denormal", "ppcrs-lfs-quiets-snan", "retro-rn-directed-rounding",
                 "retro-untracked-fpscr-xx-ux-fi-fr", "single-op-nonsingle-operand", "invalid-form-lsw-overlap", "mfspr-time-base"] {
        assert!(text.contains(name), "corpus has no case for exclusion {name}");
    }
}

fn word_pair(word: u32) -> (Pair, retro_ppc_oracle::world::Snap, retro_ppc_oracle::world::Snap) {
    let mut p = Pair::new();
    let mut r = Rng::new(7);
    let st = retro_ppc_oracle::state::State::random(&mut r);
    p.load(&[word], &st, false);
    let (a, b) = p.now();
    (p, a, b)
}

/// NEGATIVE CONTROL: the comparator sees a single flipped bit in every compared field, and a trap mismatch.
#[test]
fn the_comparator_detects_a_difference_in_each_field() {
    let (_, a, b) = word_pair(0x7c632214); // add r3,r3,r4
    assert!(diff(&a, &b, &Relax::full()).is_empty(), "identical states compare equal");
    let m = |f: &dyn Fn(&mut retro_ppc_oracle::world::Snap), name: &str| {
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
/// retro-ppc store that disagrees with the PEM model is a discrepancy, not an exclusion.
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
