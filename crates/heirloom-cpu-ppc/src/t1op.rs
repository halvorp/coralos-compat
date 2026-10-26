//! Tier 1's pre-decoded instruction: a handler id with the register numbers and immediates already extracted.
//!
//! Only the forms the census shows hot get a handler of their own, and only their VALID forms (an update load with
//! RA=0 or RA=RT, `bcctr` decrementing CTR, an `mfspr` of anything but LR/CTR, ...): everything else is
//! [`K::Generic`], which runs tier 0's own `execute` on the original word, so its semantics (and its traps) are tier
//! 0's by construction. Each specialised handler restates one tier-0 instruction; the differential oracle and the
//! lockstep mode compare the two.
use crate::fields::{mask, W};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum K {
    Nop,
    /// rD = imm (addi/addis with RA=0: li/lis).
    Li,
    /// rD = rA + imm (addi, and addis with imm already shifted).
    Addi,
    /// addic / addic. (CA; CR0) and subfic (CA), mulli.
    Addic, AddicRc, Subfic, Mulli,
    /// rA = rS op uimm (ori/oris/xori/xoris; the immediate already shifted).
    Ori, Xori,
    /// rA = rS & uimm, CR0 (andi./andis.).
    AndiRc,
    /// rA = rotl(rS, b) & imm, `Rc` variant sets CR0.
    Rlwinm, RlwinmRc,
    /// CR[d] = compare(rA, imm) signed / unsigned.
    Cmpi, Cmpli,
    /// CR[d] = compare(rA, rB) signed / unsigned.
    Cmp, Cmpl,
    /// X-form logical rA = rS op rB (Rc variant: CR0).
    Or, OrRc, And, AndRc, Xor, Andc, Nor,
    /// XO-form with OE=0: rD = rA op rB (Rc variant: CR0).
    Add, AddRc, Subf, SubfRc, Neg, Mullw,
    /// rA = sign-extend(rS) / sra(rS, b) with CA; Rc variants.
    Extsh, Extsb, Srawi, SrawiRc,
    Slw, Srw,
    Mflr, Mtlr, Mfctr, Mtctr,
    /// D-form loads with RA != 0: EA = rA + imm.
    Lwz, Lbz, Lhz, Lha,
    /// Valid update-form loads (RA != 0, RA != RT): rA = EA after.
    Lwzu, Lbzu,
    /// X-form indexed loads with RA != 0: EA = rA + rB.
    Lwzx, Lbzx,
    /// FP loads (RA != 0; the update forms are Generic).
    Lfs, Lfd,
    /// The FP arithmetic opcodes, straight to tier 0's handler for them (fpu::exec_59 / exec_63).
    Fp59, Fp63,
    // ---- everything from here on may write guest memory (or the code watch) ----
    Stw, Stb, Sth,
    /// Valid update-form stores (RA != 0).
    Stwu, Stbu,
    Stwx, Stbx,
    Stfs, Stfd,
    /// Run tier 0's `execute` on `w` (with pc/next/tb exactly as tier 0 has them).
    Generic,
    /// [`K::Generic`] for an instruction that ends a block (an invalid-form branch: it traps).
    GenericEnd,
    /// The one instruction that calls the host (`sc`): always a block of its own.
    Sc,
    // ---- terminal: branches. `imm` = the absolute target for b/bc; d = BO, a = BI for the conditional forms ----
    B, Bl, Bc, Bcl, Bclr, Bclrl, Bcctr, Bcctrl,
}

/// One decoded instruction. `w` is the original word (Generic runs it; traps quote it).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Op { pub k: K, pub d: u8, pub a: u8, pub b: u8, pub imm: u32, pub w: u32 }

impl Op {
    fn new(k: K, w: u32, d: usize, a: usize, b: usize, imm: u32) -> Op { Op { k, d: d as u8, a: a as u8, b: b as u8, imm, w } }
    /// Ends a block: a branch, `sc`, or a Generic that is a branch.
    #[inline(always)]
    pub(crate) fn terminal(&self) -> bool { self.k >= K::GenericEnd }
    /// May write guest memory (or the code watch): the block re-checks its page after it.
    #[inline(always)]
    pub(crate) fn may_write(&self) -> bool { self.k >= K::Stw }
}

/// Decode `word` found at `pc`.
pub(crate) fn decode(pc: u32, word: u32) -> Op {
    let w = W(word);
    let (t, a, b) = (w.rt(), w.ra(), w.rb());
    let g = Op::new(if crate::census::ends_block(word) { K::GenericEnd } else { K::Generic }, word, 0, 0, 0, 0);
    let o = |k: K, d: usize, a: usize, b: usize, imm: u32| Op::new(k, word, d, a, b, imm);
    match w.op() {
        14 => if a == 0 { o(K::Li, t, 0, 0, w.simm()) } else { o(K::Addi, t, a, 0, w.simm()) },
        15 => if a == 0 { o(K::Li, t, 0, 0, w.uimm() << 16) } else { o(K::Addi, t, a, 0, w.uimm() << 16) },
        12 => o(K::Addic, t, a, 0, w.simm()),
        13 => o(K::AddicRc, t, a, 0, w.simm()),
        8 => o(K::Subfic, t, a, 0, w.simm()),
        7 => o(K::Mulli, t, a, 0, w.simm()),
        48 | 50 | 52 | 54 if a != 0 => {
            let k = match w.op() { 48 => K::Lfs, 50 => K::Lfd, 52 => K::Stfs, _ => K::Stfd };
            o(k, t, a, 0, w.simm())
        }
        59 => o(K::Fp59, 0, 0, 0, 0),
        63 => o(K::Fp63, 0, 0, 0, 0),
        24 if t == a && w.uimm() == 0 => o(K::Nop, 0, 0, 0, 0),
        24 => o(K::Ori, a, t, 0, w.uimm()),
        25 => o(K::Ori, a, t, 0, w.uimm() << 16),
        26 => o(K::Xori, a, t, 0, w.uimm()),
        27 => o(K::Xori, a, t, 0, w.uimm() << 16),
        28 => o(K::AndiRc, a, t, 0, w.uimm()),
        29 => o(K::AndiRc, a, t, 0, w.uimm() << 16),
        21 => o(if w.rc() { K::RlwinmRc } else { K::Rlwinm }, a, t, w.sh() as usize, mask(w.mb(), w.me())),
        11 => o(K::Cmpi, w.bf() as usize, a, 0, w.simm()),
        10 => o(K::Cmpli, w.bf() as usize, a, 0, w.uimm()),
        32 | 34 | 40 | 42 | 36 | 38 | 44 if a != 0 => {
            let k = match w.op() { 32 => K::Lwz, 34 => K::Lbz, 40 => K::Lhz, 42 => K::Lha, 36 => K::Stw, 38 => K::Stb, _ => K::Sth };
            o(k, t, a, 0, w.simm())
        }
        33 | 35 if a != 0 && a != t => o(if w.op() == 33 { K::Lwzu } else { K::Lbzu }, t, a, 0, w.simm()),
        37 | 39 if a != 0 => o(if w.op() == 37 { K::Stwu } else { K::Stbu }, t, a, 0, w.simm()),
        18 => {
            let li = (((word & 0x03FF_FFFC) << 6) as i32 >> 6) as u32;
            let target = if w.aa() { li } else { pc.wrapping_add(li) };
            o(if w.lk() { K::Bl } else { K::B }, 0, 0, 0, target)
        }
        16 => {
            let bd = (word & 0xFFFC) as u16 as i16 as i32 as u32;
            let target = if w.aa() { bd } else { pc.wrapping_add(bd) };
            o(if w.lk() { K::Bcl } else { K::Bc }, t, a, 0, target)
        }
        17 => Op::new(K::Sc, word, 0, 0, 0, 0),
        19 => match w.xo() {
            16 => o(if w.lk() { K::Bclrl } else { K::Bclr }, t, a, 0, 0),
            528 if t & 0x04 != 0 => o(if w.lk() { K::Bcctrl } else { K::Bcctr }, t, a, 0, 0),
            _ => g,
        },
        31 => x31(w, t, a, b).unwrap_or(g),
        _ => g,
    }
}

/// The opcode-31 forms with a handler. Exact matches only: tier 0 resolves an XO number by trying the X-form tables
/// first, and none of these numbers is in another table.
fn x31(w: W, t: usize, a: usize, b: usize) -> Option<Op> {
    let word = w.0;
    let o = |k: K, d: usize, a: usize, b: usize| Some(Op::new(k, word, d, a, b, 0));
    let rc = w.rc();
    match w.xo() {
        444 => o(if rc { K::OrRc } else { K::Or }, a, t, b),
        28 => o(if rc { K::AndRc } else { K::And }, a, t, b),
        316 if !rc => o(K::Xor, a, t, b),
        60 if !rc => o(K::Andc, a, t, b),
        124 if !rc => o(K::Nor, a, t, b),
        0 => o(K::Cmp, w.bf() as usize, a, b),
        32 => o(K::Cmpl, w.bf() as usize, a, b),
        266 => o(if rc { K::AddRc } else { K::Add }, t, a, b),
        40 => o(if rc { K::SubfRc } else { K::Subf }, t, a, b),
        104 if !rc => o(K::Neg, t, a, 0),
        235 if !rc => o(K::Mullw, t, a, b),
        922 if !rc => o(K::Extsh, a, t, 0),
        954 if !rc => o(K::Extsb, a, t, 0),
        824 => o(if rc { K::SrawiRc } else { K::Srawi }, a, t, w.sh() as usize),
        24 if !rc => o(K::Slw, a, t, b),
        536 if !rc => o(K::Srw, a, t, b),
        23 if a != 0 => o(K::Lwzx, t, a, b),
        87 if a != 0 => o(K::Lbzx, t, a, b),
        151 if a != 0 => o(K::Stwx, t, a, b),
        215 if a != 0 => o(K::Stbx, t, a, b),
        339 => match w.spr() { 8 => o(K::Mflr, t, 0, 0), 9 => o(K::Mfctr, t, 0, 0), _ => None },
        467 => match w.spr() { 8 => o(K::Mtlr, t, 0, 0), 9 => o(K::Mtctr, t, 0, 0), _ => None },
        _ => None,
    }
}
