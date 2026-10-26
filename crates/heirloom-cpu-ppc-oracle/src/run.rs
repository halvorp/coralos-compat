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

/// lwarx/stwcx. carry reservation state the resync cannot copy, so a sequence stops at an excluded one.
fn has_reservation(w: u32) -> bool { w >> 26 == 31 && matches!(w >> 1 & 0x3FF, 20 | 150) }

pub fn run_case(p: &mut Pair, c: &Case) -> Verdict {
    p.load(&c.words, &c.st, strict_align(c));
    let end = CODE + 4 * c.words.len() as u32;
    let mut steps = 0;
    let mut excluded: Vec<&'static str> = Vec::new();
    for _ in 0..2 * c.words.len() + 8 {
        let (pre, _) = p.now();
        if pre.pc < CODE || pre.pc >= end || pre.pc & 3 != 0 { break; }
        let word = u32::from_be_bytes(pre.mem[pre.pc as usize..pre.pc as usize + 4].try_into().unwrap()); // as executed: a store may have patched the code
        let (a, b) = p.step();
        // An excluded step is accounted for by name; the oracle then adopts heirloom's state and the sequence goes on, so
        // the instructions after an inexact FP op are still compared.
        let mut skip: Option<Vec<&'static str>> = None;
        if crate::exclude::nonsingle_operand(word, &pre) {
            skip = Some(vec!["single-op-nonsingle-operand"]);
        } else if let Some(n) = crate::exclude::fp_mode_excuse(word, &pre, &a) {
            skip = Some(vec![n]);
        } else {
            let raw = diff(&a, &b, &Relax::full());
            if !raw.is_empty() {
                if a.trap != b.trap {
                    if let Some(n) = trap_excuse(word, &a, &b) { excluded.push(n); return Verdict::Excluded(excluded); }
                    return Verdict::Differ { step: steps, word, diffs: raw };
                }
                let rx = relax(word, &pre, &a, &b);
                let d = diff(&a, &b, &rx);
                if !d.is_empty() { return Verdict::Differ { step: steps, word, diffs: d }; }
                skip = Some(rx.names);
            }
        }
        if let Some(names) = skip {
            for n in names { if !excluded.contains(&n) { excluded.push(n); } }
            if a.trap != Tr::None || b.trap != Tr::None || a.pc != b.pc || has_reservation(word) { return Verdict::Excluded(excluded); }
            p.resync_oracle();
            steps += 1;
            continue;
        }
        if a.trap != Tr::None { break; }
        steps += 1;
    }
    if excluded.is_empty() { Verdict::Agree(steps) } else { Verdict::Excluded(excluded) }
}
