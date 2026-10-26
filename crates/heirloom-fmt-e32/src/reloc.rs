//! A relocation section: `iSize` and `iNumberOfRelocs`, then blocks of a page offset, the block's byte size (its
//! 8-byte head included) and u16 entries - the kind in the top 4 bits, the offset in the page in the low 12. Kind 0 is
//! padding.
use alloc::vec::Vec;

use crate::{u32_at, E32Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    /// The word points into the code section.
    Text,
    /// The word points into the data section.
    Data,
    /// The loader decides from the word's value: inside the code section's link range is text, otherwise data.
    Inferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reloc {
    /// Offset of the 32-bit word in the relocated section.
    pub offset: u32,
    pub kind: RelocKind,
}

pub(crate) struct Deltas {
    pub code_base: u32,
    pub code_size: u32,
    pub code: u32,
    pub data: u32,
}

pub(crate) fn parse(file: &[u8], offset: u32, section_len: usize) -> Result<Vec<Reloc>, E32Error> {
    if offset == 0 {
        return Ok(Vec::new());
    }
    let base = offset as usize;
    let size = u32_at(file, base, "relocation section")? as usize;
    let declared = u32_at(file, base + 4, "relocation section")? as usize;
    let body = (base + 8)
        .checked_add(size)
        .and_then(|end| file.get(base + 8..end))
        .ok_or(E32Error::Truncated("relocation section"))?;
    let mut relocs = Vec::new();
    let mut at = 0usize;
    while at < body.len() {
        let page = u32_at(body, at, "relocation block")?;
        let block = u32_at(body, at + 4, "relocation block")? as usize;
        if block < 8 {
            return Err(E32Error::Malformed("relocation block smaller than its head"));
        }
        let entries = at
            .checked_add(block)
            .and_then(|end| body.get(at + 8..end))
            .ok_or(E32Error::Truncated("relocation block"))?;
        for e in entries.chunks_exact(2) {
            let e = u16::from_le_bytes([e[0], e[1]]);
            let kind = match e >> 12 {
                0 => continue,
                1 => RelocKind::Text,
                2 => RelocKind::Data,
                3 => RelocKind::Inferred,
                _ => return Err(E32Error::Malformed("relocation kind")),
            };
            let offset = page.checked_add(u32::from(e & 0xFFF)).ok_or(E32Error::Malformed("relocation offset"))?;
            if offset as usize + 4 > section_len {
                return Err(E32Error::Malformed("relocation outside its section"));
            }
            relocs.push(Reloc { offset, kind });
        }
        at += block;
    }
    if relocs.len() != declared {
        return Err(E32Error::Malformed("relocation count differs from iNumberOfRelocs"));
    }
    Ok(relocs)
}

pub(crate) fn apply(section: &mut [u8], relocs: &[Reloc], d: &Deltas) -> Result<(), E32Error> {
    for r in relocs {
        let at = r.offset as usize;
        let word = section.get_mut(at..at + 4).ok_or(E32Error::Malformed("relocation outside its section"))?;
        let value = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        let text = match r.kind {
            RelocKind::Text => true,
            RelocKind::Data => false,
            RelocKind::Inferred => value.wrapping_sub(d.code_base) < d.code_size,
        };
        let moved = value.wrapping_add(if text { d.code } else { d.data });
        word.copy_from_slice(&moved.to_le_bytes());
    }
    Ok(())
}
