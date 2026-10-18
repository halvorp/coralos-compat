//! AmigaOS Hunk loader: HUNK_HEADER / CODE / DATA / BSS / RELOC32 / RELOC32SHORT / DREL32 /
//! SYMBOL / DEBUG / NAME / END.
//!
//! Every read of the file and every write to guest RAM is bounds-checked; every refusal is
//! a named [`HunkError`]. Not supported (refused by name, not skipped): overlays, resident
//! libraries, HUNK_EXT.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::vec::Vec;
use retro_guest_mem::{GuestMem, MemError};
use core::fmt;

pub mod writer;

pub const HUNK_HEADER: u32 = 0x3F3;
pub const HUNK_CODE: u32 = 0x3E9;
pub const HUNK_DATA: u32 = 0x3EA;
pub const HUNK_BSS: u32 = 0x3EB;
pub const HUNK_RELOC32: u32 = 0x3EC;
pub const HUNK_NAME: u32 = 0x3E8;
pub const HUNK_SYMBOL: u32 = 0x3F0;
pub const HUNK_DEBUG: u32 = 0x3F1;
pub const HUNK_DREL32: u32 = 0x3F7;
pub const HUNK_RELOC32SHORT: u32 = 0x3FC;
pub const HUNK_END: u32 = 0x3F2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkError {
    NotAHunkFile,
    ResidentLibrariesUnsupported,
    InconsistentTable,
    Truncated,
    AddressOverflow,
    SectionExceedsDeclared { hunk: usize },
    SectionOutsideRam { hunk: usize },
    /// A BSS hunk whose declared size does not fit in guest RAM at its load address.
    OversizedBss { hunk: usize, bytes: u32 },
    BssExceedsDeclared { hunk: usize },
    RelocToUnknownHunk { target: u32 },
    RelocOutsideRam { hunk: usize, offset: u32 },
    UnsupportedHunkType(u32),
    NoHunks,
    Mem(MemError),
}
impl fmt::Display for HunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use HunkError::*;
        match self {
            NotAHunkFile => write!(f, "hunk: not a HUNK_HEADER file"),
            ResidentLibrariesUnsupported => write!(f, "hunk: resident library names unsupported"),
            InconsistentTable => write!(f, "hunk: inconsistent hunk table"),
            Truncated => write!(f, "hunk: truncated file"),
            AddressOverflow => write!(f, "hunk: address overflow"),
            SectionExceedsDeclared { hunk } => write!(f, "hunk {hunk}: section exceeds declared size"),
            SectionOutsideRam { hunk } => write!(f, "hunk {hunk}: section outside guest RAM"),
            OversizedBss { hunk, bytes } => write!(f, "hunk {hunk}: BSS of {bytes} bytes does not fit guest RAM"),
            BssExceedsDeclared { hunk } => write!(f, "hunk {hunk}: BSS exceeds declared size"),
            RelocToUnknownHunk { target } => write!(f, "hunk: reloc to unknown hunk {target}"),
            RelocOutsideRam { hunk, offset } => write!(f, "hunk {hunk}: reloc at offset {offset:#x} outside guest RAM"),
            UnsupportedHunkType(t) => write!(f, "hunk: unsupported hunk type {t:#x}"),
            NoHunks => write!(f, "hunk: file declares no hunks"),
            Mem(e) => write!(f, "hunk: {e}"),
        }
    }
}
impl core::error::Error for HunkError {}
impl From<MemError> for HunkError { fn from(e: MemError) -> Self { HunkError::Mem(e) } }

#[derive(Debug)]
pub struct Loaded {
    /// Guest load address of every hunk, in file order.
    pub bases: Vec<u32>,
    /// Declared size in bytes of every hunk.
    pub sizes: Vec<u32>,
    /// First hunk's address.
    pub entry: u32,
    /// First address after the image.
    pub end: u32,
}

struct Rd<'a> { b: &'a [u8], p: usize }
impl Rd<'_> {
    fn u32(&mut self) -> Result<u32, HunkError> {
        let s = self.b.get(self.p..self.p.checked_add(4).ok_or(HunkError::Truncated)?).ok_or(HunkError::Truncated)?;
        self.p += 4;
        Ok(u32::from_be_bytes(s.try_into().unwrap()))
    }
    fn u16(&mut self) -> Result<u16, HunkError> {
        let s = self.b.get(self.p..self.p.checked_add(2).ok_or(HunkError::Truncated)?).ok_or(HunkError::Truncated)?;
        self.p += 2;
        Ok(u16::from_be_bytes(s.try_into().unwrap()))
    }
    fn at_end(&self) -> bool { self.p >= self.b.len() }
}

fn process_reloc(
    rd: &mut Rd,
    mem: &mut GuestMem,
    idx: usize,
    hunk_base: u32,
    hunk_size: u32,
    num_hunks: usize,
    bases: &[u32],
    short: bool,
) -> Result<(), HunkError> {
    let mut words: u32 = 0;
    loop {
        let count = if short {
            let v = rd.u16()? as u32;
            words += 1;
            v
        } else {
            rd.u32()?
        };
        if count == 0 {
            if short && words % 2 == 1 {
                let _ = rd.u16()?;
            }
            break;
        }
        let target = if short {
            let v = rd.u16()? as u32;
            words += 1;
            v
        } else {
            rd.u32()?
        };
        // Preserve existing check order verbatim:
        if idx >= num_hunks {
            return Err(HunkError::SectionExceedsDeclared { hunk: idx });
        }
        if target as usize >= num_hunks {
            return Err(HunkError::RelocToUnknownHunk { target });
        }
        let target_base = bases[target as usize];
        for _ in 0..count {
            let offset = if short {
                let v = rd.u16()? as u32;
                words += 1;
                v
            } else {
                rd.u32()?
            };
            let bad = HunkError::RelocOutsideRam { hunk: idx, offset };
            if offset.checked_add(4).map_or(true, |e| e > hunk_size) {
                return Err(bad);
            }
            let addr = hunk_base.wrapping_add(offset);
            let val = mem.read_u32(addr).map_err(|_| bad.clone())?;
            mem.write_u32(addr, val.wrapping_add(target_base)).map_err(|_| bad)?;
        }
    }
    Ok(())
}

/// Load a Hunk executable into guest RAM starting at `base`, relocations applied.
pub fn load(mem: &mut GuestMem, file: &[u8], base: u32) -> Result<Loaded, HunkError> {
    let mut r = Rd { b: file, p: 0 };
    if r.u32()? != HUNK_HEADER { return Err(HunkError::NotAHunkFile); }
    if r.u32()? != 0 { return Err(HunkError::ResidentLibrariesUnsupported); }
    let table = r.u32()? as usize;
    let (first, last) = (r.u32()? as usize, r.u32()? as usize);
    if table == 0 { return Err(HunkError::NoHunks); }
    if last.checked_add(1).and_then(|l| l.checked_sub(first)) != Some(table) { return Err(HunkError::InconsistentTable); }
    // a table larger than the file could possibly hold is truncation, not an allocation request
    if table > file.len() / 4 { return Err(HunkError::Truncated); }
    let (mut bases, mut sizes, mut cur) = (Vec::with_capacity(table), Vec::with_capacity(table), base);
    for _ in 0..table {
        let sz = (r.u32()? & 0x3FFF_FFFF).checked_mul(4).ok_or(HunkError::AddressOverflow)?; // top 2 bits = memory flags
        bases.push(cur);
        sizes.push(sz);
        cur = cur.checked_add(sz).ok_or(HunkError::AddressOverflow)?;
    }
    let mut idx = 0usize;
    while !r.at_end() {
        let id = r.u32()? & 0x3FFF_FFFF;
        match id {
            HUNK_CODE | HUNK_DATA => {
                let n = (r.u32()? as usize).checked_mul(4).ok_or(HunkError::Truncated)?;
                if idx >= table || n > sizes[idx] as usize { return Err(HunkError::SectionExceedsDeclared { hunk: idx }); }
                let src = r.b.get(r.p..r.p.checked_add(n).ok_or(HunkError::Truncated)?).ok_or(HunkError::Truncated)?;
                mem.zero(bases[idx], sizes[idx]).map_err(|_| HunkError::SectionOutsideRam { hunk: idx })?;
                mem.write_bytes(bases[idx], src).map_err(|_| HunkError::SectionOutsideRam { hunk: idx })?;
                r.p += n;
            }
            HUNK_BSS => {
                let words = r.u32()?;
                if idx >= table { return Err(HunkError::SectionExceedsDeclared { hunk: idx }); }
                let bytes = words.checked_mul(4).ok_or(HunkError::OversizedBss { hunk: idx, bytes: u32::MAX })?;
                if bytes > sizes[idx] { return Err(HunkError::BssExceedsDeclared { hunk: idx }); }
                mem.zero(bases[idx], sizes[idx]).map_err(|_| HunkError::OversizedBss { hunk: idx, bytes: sizes[idx] })?;
            }
            HUNK_RELOC32 | HUNK_RELOC32SHORT | HUNK_DREL32 => {
                let short = id != HUNK_RELOC32;
                if idx >= table { return Err(HunkError::SectionExceedsDeclared { hunk: idx }); }
                process_reloc(&mut r, mem, idx, bases[idx], sizes[idx], table, &bases, short)?;
            }
            HUNK_SYMBOL => {
                loop {
                    let n = r.u32()?;
                    if n == 0 { break; }
                    for _ in 0..n { r.u32()?; }
                    r.u32()?;
                }
            }
            HUNK_DEBUG | HUNK_NAME => {
                let n = r.u32()?;
                for _ in 0..n { r.u32()?; }
            }
            HUNK_END => idx += 1,
            other => return Err(HunkError::UnsupportedHunkType(other)),
        }
    }
    // A file that ends before every declared hunk reached its HUNK_END was cut short, even when the cut falls exactly
    // on a block boundary (the case a byte-level check cannot see).
    if idx < table { return Err(HunkError::Truncated); }
    Ok(Loaded { entry: bases[0], bases, sizes, end: cur })
}
