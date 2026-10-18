//! Relocation: every PEF opcode decoded into [`RelocOp`] (repeat instructions expanded), and an interpreter that
//! reports each word to add to. The caller owns memory, so the interpreter is testable without any.
//!
//! Opcode map (16-bit big-endian instructions; a few use a second word):
//! ```text
//! 00 skip:8 count:6                RelocBySectDWithSkip   skip words, then add sectionD to count words
//! 0100000 run:9 (+1)               RelocBySectC           add sectionC to run words
//! 0100001 run:9 (+1)               RelocBySectD
//! 0100010 run:9 (+1)               RelocTVector12         per 12 bytes: word0 += C, word1 += D, word2 untouched
//! 0100011 run:9 (+1)               RelocTVector8          per 8 bytes: word0 += C, word1 += D
//! 0100100 run:9 (+1)               RelocVTable8           per 8 bytes: word0 += D, word1 untouched
//! 0100101 run:9 (+1)               RelocImportRun         the next run imports, in order, one word each
//! 0110000 index:9                  RelocSmByImport        one word += import[index]; next import = index + 1
//! 0110001 index:9                  RelocSmSetSectC        sectionC := base of section index
//! 0110010 index:9                  RelocSmSetSectD
//! 0110011 index:9                  RelocSmBySection       one word += base of section index
//! 1000 offset:12 (+1)              RelocIncrPosition      skip offset bytes
//! 1001 blocks:4 (+1) count:8 (+1)  RelocSmRepeat          repeat the preceding `blocks` instructions `count` more times
//! 101000 off:10 | off:16           RelocSetPosition       position := 26-bit byte offset into the section
//! 101001 idx:10 | idx:16           RelocLgByImport        one word += import[26-bit index]
//! 101100 blocks:4 (+1) cnt:6 | cnt:16 (+1)  RelocLgRepeat  as SmRepeat with a 22-bit count
//! 101101 sub:4 idx:6 | idx:16      RelocLgSetOrBySection  sub 0 = BySection, 1 = SetSectC, 2 = SetSectD (22-bit index)
//! ```
//! Initial state per relocation header: position 0 in the header's section, sectionC = section 0's base, sectionD =
//! section 1's base, next import 0. Every relocated word is an ADDITION to the word the section already holds.
use crate::loader::Loader;
use crate::PefError;
use alloc::vec::Vec;

/// A repeat that would expand beyond this many ops is refused (a corrupt count, not a real fragment).
pub const MAX_OPS: usize = 1 << 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocOp {
    Skip(u32),
    SetPosition(u32),
    BySectC(u32),
    BySectD(u32),
    TVector12(u32),
    TVector8(u32),
    VTable8(u32),
    ImportRun(u32),
    ByImport(u32),
    SetSectC(u32),
    SetSectD(u32),
    BySection(u32),
}

fn next_word(words: &[u16], at: &mut usize, end: usize) -> Result<u16, PefError> {
    if *at >= end { return Err(PefError::BadReloc("a two-word opcode is cut off by the end of its relocation run")); }
    let w = words[*at];
    *at += 1;
    Ok(w)
}

/// Decode `count` 16-bit relocation words starting at word `first` into ops.
pub fn decode(words: &[u16], first: usize, count: usize) -> Result<Vec<RelocOp>, PefError> {
    let end = first.checked_add(count).filter(|&e| e <= words.len()).ok_or(PefError::BadReloc("relocation run past the instruction area"))?;
    // one entry per INSTRUCTION, so a repeat's block count is in instructions, not words
    let mut instrs: Vec<Vec<RelocOp>> = Vec::new();
    let mut total = 0usize;
    let mut at = first;
    while at < end {
        let index = (at - first) as u32;
        let op = next_word(words, &mut at, end)?;
        let unknown = PefError::UnknownRelocOpcode { opcode: op, index };
        let ops: Vec<RelocOp> = match op >> 14 {
            0 => {
                let (skip, n) = (u32::from((op >> 6) & 0xFF), u32::from(op & 0x3F));
                alloc::vec![RelocOp::Skip(skip * 4), RelocOp::BySectD(n)]
            }
            _ => match op >> 9 {
                0x20 => alloc::vec![RelocOp::BySectC(u32::from(op & 0x1FF) + 1)],
                0x21 => alloc::vec![RelocOp::BySectD(u32::from(op & 0x1FF) + 1)],
                0x22 => alloc::vec![RelocOp::TVector12(u32::from(op & 0x1FF) + 1)],
                0x23 => alloc::vec![RelocOp::TVector8(u32::from(op & 0x1FF) + 1)],
                0x24 => alloc::vec![RelocOp::VTable8(u32::from(op & 0x1FF) + 1)],
                0x25 => alloc::vec![RelocOp::ImportRun(u32::from(op & 0x1FF) + 1)],
                0x30 => alloc::vec![RelocOp::ByImport(u32::from(op & 0x1FF))],
                0x31 => alloc::vec![RelocOp::SetSectC(u32::from(op & 0x1FF))],
                0x32 => alloc::vec![RelocOp::SetSectD(u32::from(op & 0x1FF))],
                0x33 => alloc::vec![RelocOp::BySection(u32::from(op & 0x1FF))],
                _ => match op >> 12 {
                    8 => alloc::vec![RelocOp::Skip(u32::from(op & 0xFFF) + 1)],
                    9 => repeat(&instrs, usize::from((op >> 8) & 0xF) + 1, u32::from(op & 0xFF) + 1, &mut total)?,
                    _ => match op >> 10 {
                        0x28 => { let lo = next_word(words, &mut at, end)?; alloc::vec![RelocOp::SetPosition((u32::from(op & 0x3FF) << 16) | u32::from(lo))] }
                        0x29 => { let lo = next_word(words, &mut at, end)?; alloc::vec![RelocOp::ByImport((u32::from(op & 0x3FF) << 16) | u32::from(lo))] }
                        0x2C => {
                            let lo = next_word(words, &mut at, end)?;
                            let n = ((u32::from(op & 0x3F) << 16) | u32::from(lo)) + 1;
                            repeat(&instrs, usize::from((op >> 6) & 0xF) + 1, n, &mut total)?
                        }
                        0x2D => {
                            let lo = next_word(words, &mut at, end)?;
                            let idx = (u32::from(op & 0x3F) << 16) | u32::from(lo);
                            match (op >> 6) & 0xF { 0 => alloc::vec![RelocOp::BySection(idx)], 1 => alloc::vec![RelocOp::SetSectC(idx)], 2 => alloc::vec![RelocOp::SetSectD(idx)], _ => return Err(unknown) }
                        }
                        _ => return Err(unknown),
                    },
                },
            },
        };
        total += ops.len();
        if total > MAX_OPS { return Err(PefError::RelocTooLarge); }
        instrs.push(ops);
    }
    Ok(instrs.into_iter().flatten().collect())
}

/// Expand a repeat: the ops of the last `blocks` instructions, `times` more times.
fn repeat(instrs: &[Vec<RelocOp>], blocks: usize, times: u32, total: &mut usize) -> Result<Vec<RelocOp>, PefError> {
    if blocks > instrs.len() { return Err(PefError::BadReloc("a repeat reaches back past the first instruction")); }
    let block: Vec<RelocOp> = instrs[instrs.len() - blocks..].iter().flatten().copied().collect();
    let n = block.len().checked_mul(times as usize).filter(|&n| total.saturating_add(n) <= MAX_OPS).ok_or(PefError::RelocTooLarge)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..times { out.extend_from_slice(&block); }
    Ok(out)
}

/// Run `ops` for one relocation header. `bases[i]` is the guest address of instantiated section `i` (None when it is
/// not instantiated); `imports[i]` is the resolved value of import `i`. `add(section, offset, addend)` must add `addend`
/// to the big-endian word at `offset` in `section`.
pub fn apply(ops: &[RelocOp], section: u16, bases: &[Option<u32>], imports: &[u32], add: &mut dyn FnMut(u16, u32, u32) -> Result<(), PefError>) -> Result<(), PefError> {
    let base = |i: u32| bases.get(i as usize).copied().flatten().ok_or(PefError::BadReloc("an opcode names a section that is not instantiated"));
    let (mut c, mut d) = (base(0).ok(), base(1).ok());
    let (mut pos, mut next_import) = (0u32, 0usize);
    let imp = |i: usize| imports.get(i).copied().ok_or(PefError::BadReloc("an opcode names an import past the import table"));
    let pos_add = |pos: u32, n: u32| pos.checked_add(n).ok_or(PefError::BadReloc("relocation position overflows"));
    let need = |v: Option<u32>| v.ok_or(PefError::BadReloc("an opcode relocates by sectionC/sectionD but that section is not instantiated"));
    for op in ops {
        match *op {
            RelocOp::Skip(n) => pos = pos_add(pos, n)?,
            RelocOp::SetPosition(p) => pos = p,
            RelocOp::BySectC(n) | RelocOp::BySectD(n) => {
                let v = need(if matches!(op, RelocOp::BySectC(_)) { c } else { d })?;
                for _ in 0..n { add(section, pos, v)?; pos = pos_add(pos, 4)?; }
            }
            RelocOp::TVector12(n) | RelocOp::TVector8(n) => {
                let step = if matches!(op, RelocOp::TVector12(_)) { 12 } else { 8 };
                let (cv, dv) = (need(c)?, need(d)?);
                for _ in 0..n { add(section, pos, cv)?; add(section, pos_add(pos, 4)?, dv)?; pos = pos_add(pos, step)?; }
            }
            RelocOp::VTable8(n) => { let dv = need(d)?; for _ in 0..n { add(section, pos, dv)?; pos = pos_add(pos, 8)?; } }
            RelocOp::ImportRun(n) => { for _ in 0..n { add(section, pos, imp(next_import)?)?; next_import += 1; pos = pos_add(pos, 4)?; } }
            RelocOp::ByImport(i) => { add(section, pos, imp(i as usize)?)?; next_import = i as usize + 1; pos = pos_add(pos, 4)?; }
            RelocOp::SetSectC(i) => c = Some(base(i)?),
            RelocOp::SetSectD(i) => d = Some(base(i)?),
            RelocOp::BySection(i) => { add(section, pos, base(i)?)?; pos = pos_add(pos, 4)?; }
        }
    }
    Ok(())
}

impl Loader {
    /// Decode relocation header `h` into ops.
    pub fn decode_header(&self, h: usize) -> Result<Vec<RelocOp>, PefError> {
        let rh = self.reloc_headers.get(h).ok_or(PefError::BadReloc("no such relocation header"))?;
        decode(&self.reloc_words, rh.first as usize / 2, rh.count as usize)
    }

    /// Run every relocation header (see [`apply`]).
    pub fn relocate(&self, bases: &[Option<u32>], imports: &[u32], add: &mut dyn FnMut(u16, u32, u32) -> Result<(), PefError>) -> Result<(), PefError> {
        for (i, rh) in self.reloc_headers.iter().enumerate() {
            let ops = self.decode_header(i)?;
            apply(&ops, rh.section, bases, imports, add)?;
        }
        Ok(())
    }
}
