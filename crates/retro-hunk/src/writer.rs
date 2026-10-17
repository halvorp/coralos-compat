//! Builds Hunk files for fixtures and negative tests. Not a general assembler/linker.
use crate::*;

pub fn be32(v: &mut Vec<u8>, x: u32) { v.extend_from_slice(&x.to_be_bytes()); }
pub fn be16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }
pub fn pad4(v: &mut Vec<u8>) { while v.len() % 4 != 0 { v.push(0); } }

pub enum Kind { Code, Data, Bss }

pub struct Hunk {
    pub kind: Kind,
    /// CODE/DATA payload (padded to 4 on write). For BSS, `bss_bytes` is used.
    pub bytes: Vec<u8>,
    pub bss_bytes: u32,
    /// (target hunk index, offset of the 32-bit word in THIS hunk to relocate).
    pub relocs: Vec<(u32, u32)>,
}
impl Hunk {
    pub fn code(bytes: Vec<u8>, relocs: Vec<(u32, u32)>) -> Self { Hunk { kind: Kind::Code, bytes, bss_bytes: 0, relocs } }
    pub fn data(bytes: Vec<u8>, relocs: Vec<(u32, u32)>) -> Self { Hunk { kind: Kind::Data, bytes, bss_bytes: 0, relocs } }
    pub fn bss(bytes: u32) -> Self { Hunk { kind: Kind::Bss, bytes: vec![], bss_bytes: bytes, relocs: vec![] } }
    fn words(&self) -> u32 {
        match self.kind { Kind::Bss => self.bss_bytes.div_ceil(4), _ => (self.bytes.len() as u32).div_ceil(4) }
    }
}

/// Serialise `hunks` as a HUNK_HEADER executable. `declared_words` overrides the header's
/// size table (to build files that lie about their sizes).
pub fn build(hunks: &[Hunk], declared_words: Option<&[u32]>) -> Vec<u8> {
    let mut f = Vec::new();
    let n = hunks.len() as u32;
    for x in [HUNK_HEADER, 0, n, 0, n.saturating_sub(1)] { be32(&mut f, x); }
    for (i, h) in hunks.iter().enumerate() {
        be32(&mut f, declared_words.map_or_else(|| h.words(), |d| d[i]));
    }
    for h in hunks {
        match h.kind {
            Kind::Code | Kind::Data => {
                let mut b = h.bytes.clone();
                pad4(&mut b);
                be32(&mut f, if matches!(h.kind, Kind::Code) { HUNK_CODE } else { HUNK_DATA });
                be32(&mut f, (b.len() / 4) as u32);
                f.extend_from_slice(&b);
            }
            Kind::Bss => { be32(&mut f, HUNK_BSS); be32(&mut f, h.words()); }
        }
        let mut targets: Vec<u32> = h.relocs.iter().map(|r| r.0).collect();
        targets.sort_unstable();
        targets.dedup();
        if !targets.is_empty() {
            be32(&mut f, HUNK_RELOC32);
            for t in targets {
                let offs: Vec<u32> = h.relocs.iter().filter(|r| r.0 == t).map(|r| r.1).collect();
                be32(&mut f, offs.len() as u32);
                be32(&mut f, t);
                for o in offs { be32(&mut f, o); }
            }
            be32(&mut f, 0);
        }
        be32(&mut f, HUNK_END);
    }
    f
}
