//! ppc-corpus: rebuild corpus/regressions.txt. Contains (a) hand-built cases for every retro-ppc defect the fuzz found,
//! with the verdict they have now, and (b) the first three cases the fuzz produced for each named exclusion.
use retro_ppc_oracle::corpus::{expect_of, write_line};
use retro_ppc_oracle::gen::{self, Case};
use retro_ppc_oracle::rng::Rng;
use retro_ppc_oracle::run::{run_case, Verdict};
use retro_ppc_oracle::state::State;
use retro_ppc_oracle::table::table;
use retro_ppc_oracle::world::Pair;
use std::collections::BTreeMap;

fn base(seed: u64) -> State {
    let mut r = Rng::new(seed);
    let mut s = State::random(&mut r);
    s.fpscr = 0;
    s
}
fn a_form(op: u32, xo: u32, frt: u32, fra: u32, frb: u32, frc: u32) -> u32 { op << 26 | frt << 21 | fra << 16 | frb << 11 | frc << 6 | xo << 1 }

fn main() {
    let mut pair = Pair::new();
    let mut out = vec!["# retro-ppc vs ppc-rs regression corpus. See ../README.md; regenerate with `cargo run --release --bin ppc-corpus > corpus/regressions.txt`.".to_string()];
    let mut line = |label: &str, c: &Case, pair: &mut Pair| {
        let v = run_case(pair, c);
        assert!(!matches!(v, Verdict::Differ { .. }), "{label}: {v:?}");
        out.push(write_line(label, &expect_of(&v), c));
    };
    // (a) the defects fixed in retro-ppc
    let mut st = base(1);
    st.gpr[3] = 0x1800;
    st.fpr[1] = (1.0f64 + 0.75 * 2f64.powi(-23)).to_bits();
    line("fixed:stfs-truncates-not-rounds", &Case { words: vec![52 << 26 | 1 << 21 | 3 << 16], st: st.clone() }, &mut pair);
    st.fpscr = 3; // RN = toward -inf: ppc-rs rounds per RN, PEM does not look at it
    line("fixed:stfs-ignores-rn", &Case { words: vec![52 << 26 | 1 << 21 | 3 << 16], st: st.clone() }, &mut pair);
    let mut st = base(2);
    st.gpr[3] = 0x1800;
    st.fpr[1] = (512.75 * 2f64.powi(-149)).to_bits();
    line("fixed:stfs-denormal-truncates", &Case { words: vec![52 << 26 | 1 << 21 | 3 << 16 | 4], st }, &mut pair);
    let mut st = base(3);
    st.fpr[2] = f64::INFINITY.to_bits();
    st.fpr[3] = 0x7FF8_0000_DEAD_BEEF;
    st.fpr[4] = 0;
    line("fixed:fmadd-vximz-with-qnan-addend", &Case { words: vec![a_form(63, 29, 1, 2, 3, 4)], st }, &mut pair);
    let mut st = base(4);
    st.fpscr = 1 << 4; // ZE
    st.fpr[1] = 7f64.to_bits();
    st.fpr[2] = 1f64.to_bits();
    st.fpr[3] = 0;
    line("fixed:fdiv-ze-leaves-target", &Case { words: vec![a_form(63, 18, 1, 2, 3, 0)], st }, &mut pair);
    let mut st = base(5);
    st.fpscr = 1 << 7; // VE
    st.fpr[2] = 0x7FF0_0000_0000_0001;
    st.fpr[3] = 1f64.to_bits();
    st.cr = 0;
    line("fixed:fadd-rc-cr1-when-ve-suppresses", &Case { words: vec![a_form(63, 21, 1, 2, 3, 0) | 1], st }, &mut pair);
    // (b) a few cases per named exclusion, and per family a plain agreeing case
    let t = table();
    let mut got: BTreeMap<String, u32> = BTreeMap::new();
    for fi in 0..t.len() {
        for i in 0..4000u64 {
            let mut r = Rng::new(0xC0DE ^ (fi as u64) << 32 ^ i);
            let c = gen::single(&mut r, &t[fi]);
            let v = run_case(&mut pair, &c);
            let key = match &v { Verdict::Excluded(n) => n.join(","), Verdict::Agree(_) => format!("agree:{}", t[fi].name), Verdict::Differ { .. } => panic!("{v:?}") };
            let n = got.entry(key.clone()).or_default();
            if *n < if key.starts_with("agree:") { 1 } else { 3 } {
                *n += 1;
                let label = if key.starts_with("agree:") { key.clone() } else { format!("excluded:{}:{}", key, t[fi].name) };
                out.push(write_line(&label, &expect_of(&v), &c));
            }
        }
    }
    println!("{}", out.join("\n"));
}
