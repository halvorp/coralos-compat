//! Drive a case through a [`Pair`] and judge it.
use crate::compare::{diff, Diff, Relax};
use crate::exclude::{relax, trap_excuse};
use crate::gen::Case;
use crate::state::CODE;
use crate::world::{Pair, Tr};

#[derive(Debug)]
pub enum Verdict {
    /// Every instruction agreed (steps executed).
    Agree(usize),
    /// Stopped at an instruction whose difference is covered by named exclusion(s).
    Excluded(Vec<&'static str>),
    /// A real discrepancy at instruction `step` (word `word`).
    Differ { step: usize, word: u32, diffs: Vec<Diff> },
}

fn strict_align(c: &Case) -> bool {
    c.words.len() == 1 && {
        let w = c.words[0];
        matches!(w >> 26, 46 | 47) || (w >> 26 == 31 && matches!(w >> 1 & 0x3FF, 20 | 150))
    }
}

pub fn run_case(p: &mut Pair, c: &Case) -> Verdict {
    p.load(&c.words, &c.st, strict_align(c));
    let end = CODE + 4 * c.words.len() as u32;
    let mut steps = 0;
    for _ in 0..2 * c.words.len() + 8 {
        let (pre, _) = p.now();
        if pre.pc < CODE || pre.pc >= end || pre.pc & 3 != 0 { break; }
        let word = u32::from_be_bytes(pre.mem[pre.pc as usize..pre.pc as usize + 4].try_into().unwrap()); // as executed: a store may have patched the code
        let (a, b) = p.step();
        if crate::exclude::nonsingle_operand(word, &pre) {
            return Verdict::Excluded(vec!["single-op-nonsingle-operand"]);
        }
        if let Some(n) = crate::exclude::fp_mode_excuse(word, &pre, &a) {
            return Verdict::Excluded(vec![n]);
        }
        let raw = diff(&a, &b, &Relax::full());
        if !raw.is_empty() {
            if a.trap != b.trap {
                if let Some(n) = trap_excuse(word, &a, &b) { return Verdict::Excluded(vec![n]); }
                return Verdict::Differ { step: steps, word, diffs: raw };
            }
            let rx = relax(word, &pre, &a, &b);
            let d = diff(&a, &b, &rx);
            if !d.is_empty() { return Verdict::Differ { step: steps, word, diffs: d }; }
            return Verdict::Excluded(rx.names);
        }
        if a.trap != Tr::None { return Verdict::Agree(steps); }
        steps += 1;
    }
    Verdict::Agree(steps)
}
