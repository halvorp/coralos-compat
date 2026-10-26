//! ppc-gaps: sweep the whole opcode space with random operand fields and list the encodings ppc-rs executes but
//! heirloom-cpu-ppc refuses as Unimplemented / Privileged (coverage gaps), and the reverse. Not a correctness check.
use heirloom_cpu_ppc_oracle::gen;
use heirloom_cpu_ppc_oracle::rng::Rng;
use heirloom_cpu_ppc_oracle::state::State;
use heirloom_cpu_ppc_oracle::world::{Pair, Tr};
use std::collections::BTreeMap;

fn main() {
    let mut pair = Pair::new();
    let mut r = Rng::new(99);
    let mut only_oracle: BTreeMap<(u32, u32), String> = BTreeMap::new();
    let mut only_heirloom: BTreeMap<(u32, u32), String> = BTreeMap::new();
    for op in 0..64u32 {
        let xos: Vec<u32> = if matches!(op, 19 | 31 | 59 | 63) { (0..1024).collect() } else { vec![0] };
        for xo in xos {
            for _ in 0..6 {
                let mut w = r.u32();
                if xos_len(op) { w = (w & !0x7FE) | xo << 1; }
                w = (op << 26) | (w & 0x03FF_FFFF);
                let mut st = State::random(&mut r);
                st.gpr[(w >> 16 & 31) as usize] = 0x1800;
                let c = gen::Case { words: vec![w], st };
                pair.load(&c.words, &c.st, false);
                let (a, b) = pair.step();
                let key = (op, if xos_len(op) { xo } else { 0 });
                // A-form FP instructions carry frC in the low bits of the 10-bit XO field; RN != 0 is a named refusal.
                if a.note.contains("directed rounding") || (matches!(op, 59 | 63) && matches!(xo & 0x1F, 18 | 20..=26 | 28..=31)) { continue; }
                if a.trap == Tr::Illegal && b.trap == Tr::None && a.invalid_why.is_none() { only_oracle.entry(key).or_insert(format!("{w:08x}: {}", a.note)); }
                if a.trap == Tr::None && b.trap == Tr::Illegal { only_heirloom.entry(key).or_insert(format!("{w:08x}: ppc-rs {}", b.note)); }
            }
        }
    }
    println!("executed by ppc-rs, refused by heirloom-cpu-ppc (op, xo): {}", only_oracle.len());
    for ((op, xo), n) in &only_oracle { println!("  op {op:2} xo {xo:4}  {n}"); }
    println!("executed by heirloom-cpu-ppc, refused by ppc-rs (op, xo): {}", only_heirloom.len());
    for ((op, xo), n) in &only_heirloom { println!("  op {op:2} xo {xo:4}  {n}"); }
}
fn xos_len(op: u32) -> bool { matches!(op, 19 | 31 | 59 | 63) }
