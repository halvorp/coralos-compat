//! The fixed regression corpus (corpus/regressions.txt): one case per line, a label, the verdict it must keep, the
//! instruction words and the complete starting state. `tools: cargo run --release --bin ppc-corpus` regenerates it.
//!
//! `label | expect | words | gpr | fpr | cr xer lr ctr fpscr tb memseed`
//! `expect` is `agree` or `excluded:<name>[,<name>]`; a corpus line whose verdict changes fails the test.
use crate::gen::Case;
use crate::state::State;

fn hex<T: std::fmt::LowerHex>(xs: &[T], w: usize) -> String { xs.iter().map(|x| format!("{x:0w$x}")).collect::<Vec<_>>().join(",") }
fn unhex(s: &str) -> Vec<u64> { s.split(',').filter(|x| !x.is_empty()).map(|x| u64::from_str_radix(x, 16).expect("corpus hex")).collect() }

pub fn write_line(label: &str, expect: &str, c: &Case) -> String {
    let s = &c.st;
    format!(
        "{label} | {expect} | {} | {} | {} | {:08x} {:08x} {:08x} {:08x} {:08x} {:016x} {:016x}",
        hex(&c.words, 8), hex(&s.gpr, 8), hex(&s.fpr, 16), s.cr, s.xer, s.lr, s.ctr, s.fpscr, s.tb, s.memseed
    )
}

pub struct Entry {
    pub label: String,
    pub expect: String,
    pub case: Case,
}

pub fn parse(text: &str) -> Vec<Entry> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|l| {
        let f: Vec<&str> = l.split(" | ").collect();
        assert_eq!(f.len(), 6, "corpus line: {l}");
        let words = unhex(f[2]).into_iter().map(|w| w as u32).collect();
        let mut gpr = [0u32; 32];
        for (i, v) in unhex(f[3]).into_iter().enumerate() { gpr[i] = v as u32; }
        let mut fpr = [0u64; 32];
        for (i, v) in unhex(f[4]).into_iter().enumerate() { fpr[i] = v; }
        let r: Vec<u64> = f[5].split(' ').map(|x| u64::from_str_radix(x, 16).unwrap()).collect();
        let st = State { gpr, fpr, cr: r[0] as u32, xer: r[1] as u32, lr: r[2] as u32, ctr: r[3] as u32, fpscr: r[4] as u32, tb: r[5], memseed: r[6], reservation: None };
        Entry { label: f[0].into(), expect: f[1].into(), case: Case { words, st } }
    }).collect()
}

/// How a verdict is written in the corpus.
pub fn expect_of(v: &crate::run::Verdict) -> String {
    match v {
        crate::run::Verdict::Agree(_) => "agree".into(),
        crate::run::Verdict::Excluded(n) => format!("excluded:{}", n.join(",")),
        crate::run::Verdict::Differ { .. } => "DIFFER".into(),
    }
}
