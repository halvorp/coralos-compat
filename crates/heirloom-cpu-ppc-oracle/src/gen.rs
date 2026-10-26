//! Case generation: instruction words from the family table, state set-up per category, short sequences.
use crate::rng::Rng;
use crate::state::{fp_single_val, State, CODE, DATA, RAM};
use crate::table::{Cat, Ins};

#[derive(Clone, Debug)]
pub struct Case {
    pub words: Vec<u32>,
    pub st: State,
}

fn put_spr(w: u32, n: u32) -> u32 { (w & !0x001F_F800) | (n & 31) << 16 | (n >> 5) << 11 }

fn small_disp(r: &mut Rng) -> u32 { (r.below(0x600) as i32 - 0x300) as u16 as u32 }

/// Make the base registers of a memory instruction land in the data window (most of the time).
fn aim_memory(r: &mut Rng, st: &mut State, w: &mut u32) {
    let op = *w >> 26;
    let ra = (*w >> 16 & 31) as usize;
    let rb = (*w >> 11 & 31) as usize;
    if op != 31 {
        *w = (*w & 0xFFFF_0000) | small_disp(r);
    }
    match r.below(20) {
        0 => {} // wild: whatever the random state holds (bus faults, alignment)
        1 => { // straddle the end of RAM
            if ra != 0 { st.gpr[ra] = RAM as u32 - 1 - r.below(8); }
            if op == 31 && rb != ra { st.gpr[rb] = 0; }
        }
        2 => { // low memory (code / zero page), including address 0 wrap
            if ra != 0 { st.gpr[ra] = r.below(0x100); }
            if op == 31 && rb != ra { st.gpr[rb] = 0u32.wrapping_sub(r.below(8)); }
        }
        _ => {
            let base = DATA + 0x800 + r.below(0x400);
            if ra != 0 { st.gpr[ra] = base; }
            if op == 31 && rb != ra {
                st.gpr[rb] = if r.pct(60) { r.below(0x40) } else { (r.below(0x400) as i32 - 0x200) as u32 };
            }
        }
    }
    if r.pct(50) && op == 31 { // keep word accesses aligned half the time, so lwarx/stwcx./lmw/lsw get past the check
        if ra != 0 { st.gpr[ra] &= !3; }
        if rb != ra { st.gpr[rb] &= !3; }
    }
}

fn fix_branch(r: &mut Rng, w: &mut u32, st: &mut State) {
    let op = *w >> 26;
    let near = r.pct(60);
    match op {
        18 if near => *w = (*w & !0x03FF_FFFC) | ((r.below(24) as i32 - 6) * 4) as u32 & 0x03FF_FFFC,
        16 if near => *w = (*w & !0xFFFC) | ((r.below(24) as i32 - 6) * 4) as u32 & 0xFFFC,
        19 => {
            st.lr = CODE + 4 * r.below(10);
            st.ctr = if r.pct(40) { CODE + 4 * r.below(10) } else { st.ctr };
            if r.pct(30) { st.ctr = r.below(4); }
        }
        _ => {}
    }
    if r.pct(20) { st.ctr = r.below(3); }
}

/// Instruction word for table entry `ins`, with `st` adjusted so the case is meaningful.
pub fn make(r: &mut Rng, ins: &Ins, st: &mut State) -> u32 {
    let mut w = ins.word(r.u32());
    match ins.cat {
        Cat::Mem | Cat::FpMem => aim_memory(r, st, &mut w),
        Cat::Br => fix_branch(r, &mut w, st),
        Cat::FpSingle => {
            if r.pct(92) {
                for f in [w >> 16 & 31, w >> 11 & 31, w >> 6 & 31] { st.fpr[f as usize] = fp_single_val(r); }
            }
        }
        Cat::Spr => {
            let op31 = w >> 1 & 0x3FF;
            if op31 == 339 || op31 == 467 {
                let n = if r.pct(80) { r.pick(&[1, 8, 9, 268, 269]) } else { r.pick(&[0, 3, 4, 5, 6, 18, 19, 22, 25, 26, 256, 272, 287, 1008, 1022, 0x3FF]) };
                w = put_spr(w, if op31 == 467 && (n == 268 || n == 269) { 8 } else { n });
            } else if op31 == 371 {
                w = put_spr(w, if r.pct(90) { 268 + r.below(2) } else { r.pick(&[0, 1, 8, 9, 267, 270, 284]) });
            }
        }
        _ => {}
    }
    w
}

fn state_for(r: &mut Rng) -> State { State::random(r) }

/// One instruction from `ins`.
pub fn single(r: &mut Rng, ins: &Ins) -> Case {
    let mut st = state_for(r);
    let w = make(r, ins, &mut st);
    Case { words: vec![w], st }
}

/// A short random sequence of 2..=8 instructions from the whole table (privileged / reserved system ops thinned).
pub fn sequence(r: &mut Rng, t: &[Ins]) -> Case {
    let mut st = state_for(r);
    let n = 2 + r.below(7) as usize;
    let mut words = Vec::new();
    // Keep the data pointer stable across the sequence: aim every memory instruction at the same base.
    for _ in 0..n {
        let ins = loop {
            let i = &t[r.below(t.len() as u32) as usize];
            if !matches!(i.name, "mfmsr" | "mtmsr" | "rfi" | "sc") || r.pct(5) { break i; }
        };
        words.push(make(r, ins, &mut st));
    }
    Case { words, st }
}

/// lwarx ... stwcx. pairs, with an intervening store sometimes, and a different EA sometimes.
pub fn atomic(r: &mut Rng) -> Case {
    let mut st = state_for(r);
    let (ra, rb) = (1 + r.below(30), 1 + r.below(30));
    let (rt, rs) = (r.below(32), r.below(32));
    st.gpr[ra as usize] = DATA + 0x100 * r.below(8) + 4 * r.below(8);
    if rb != ra { st.gpr[rb as usize] = 4 * r.below(4); }
    let lwarx = 31 << 26 | rt << 21 | ra << 16 | rb << 11 | 20 << 1;
    let stwcx = 31 << 26 | rs << 21 | ra << 16 | rb << 11 | 150 << 1 | 1;
    let mut words = vec![lwarx];
    if r.pct(25) { words.push(36 << 26 | rs << 21 | ra << 16 | 0); } // stw to the reserved word
    if r.pct(20) { words.push(36 << 26 | rs << 21 | ra << 16 | 4); }
    if r.pct(25) { // second lwarx on another address, then stwcx. on the first
        words.push(31 << 26 | rt << 21 | ra << 16 | rb << 11 | 20 << 1);
    }
    if r.pct(20) { words.push(31 << 26 | rs << 21 | ra << 16 | rb << 11 | 150 << 1 | 1); } // stwcx. twice
    words.push(stwcx);
    if r.pct(20) { st.gpr[rb as usize] = st.gpr[rb as usize].wrapping_add(4); }
    Case { words, st }
}
