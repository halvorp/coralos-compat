//! Symbian resource-file (`.rsc`) reader: the header, the resource index, the "compressed unicode" run layer, and a
//! [`Reader`] that walks the fields of one resource (`LONG`, `WORD`, `BYTE`, `LTEXT`).
//!
//! FORMAT FACTS (read from EKA2L1 `src/emu/loader/src/rsc.cpp`, GPL: facts only, this code is written from them, and checked
//! against four real S60 5th edition files):
//! * UID1 `0x101F4A6B` = the plain layout this reader handles; `0x101F5010` (dictionary-compressed) and the "Calypso" layout
//!   (rsc.cpp:`read_header_and_resource_index`) are refused by name, not guessed.
//! * Plain layout: 12 bytes of UIDs, a 4-byte checksum, a flags byte (16), a 2-byte largest-resource size (17), then at 19 a
//!   bit array (one bit per resource, LSB first) saying which resources carry compressed unicode, then the resources; the
//!   file ends with the index (one little-endian u16 absolute file offset per resource) and a final u16 = the index's own offset
//!   (rsc.cpp:330-352).
//! * Resource ids are 1-based; the high 20 bits are an offset that must be 0 or the file's. The offset comes from the FIRST
//!   resource when it is the 8-byte signature record `{ 4, id }` (rsc.cpp:`confirm_signature`, `own_res_id`).
//! * A compressed-unicode resource is a sequence of runs, alternately "unicode" (first) and "raw": a run header is one byte
//!   (or two when bit 7 is set: `(b & 0x7f) << 8 | next`), then that many bytes. A unicode run is SCSU-compressed text that
//!   expands to UTF-16LE, padded with `0xAB` first when the output so far is odd, so 16-bit text is 2-aligned (rsc.cpp:402-455).
//!
//! The SCSU expander here is the SUBSET real files use: ASCII bytes and the default Latin-1 window (bytes >= 0x80). Any SCSU
//! tag is a named [`RscError::Scsu`], never a guess.
#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Why a file or resource cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RscError {
    Truncated(&'static str),
    /// A layout this reader does not handle (named).
    Unsupported(&'static str),
    /// The id's offset is not this file's, or its number is out of range.
    NotOwned(u32),
    /// A malformed run layer.
    BadRuns(&'static str),
    /// An SCSU tag outside the supported subset.
    Scsu(u8),
    /// A field read ran past the end of the resource.
    Field(&'static str),
}

const UID1_PLAIN: u32 = 0x101F_4A6B;
const UID1_DICTIONARY: u32 = 0x101F_5010;
const FLAG_FIRST_IS_BITS: u8 = 0x20;
const FLAG_SIGNATURE_GENERATED: u8 = 0x40;
const FLAG_UID3_OFFSET: u8 = 0x80;
const BITS_AT: usize = 19;

pub struct RscFile<'a> {
    bytes: &'a [u8],
    pub uid1: u32,
    pub uid2: u32,
    pub uid3: u32,
    /// Absolute offset of each resource, plus the end of the last one (the index offset): `count() + 1` entries.
    bounds: Vec<u16>,
    bits: &'a [u8],
    /// The id offset (`id & 0xFFFFF000`) from the signature record; 0 when the file has none.
    pub id_offset: u32,
}

fn u16_at(b: &[u8], at: usize, what: &'static str) -> Result<u16, RscError> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or(RscError::Truncated(what))
}
fn u32_at(b: &[u8], at: usize, what: &'static str) -> Result<u32, RscError> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or(RscError::Truncated(what))
}

impl<'a> RscFile<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, RscError> {
        let uid1 = u32_at(bytes, 0, "UID1")?;
        let uid2 = u32_at(bytes, 4, "UID2")?;
        let uid3 = u32_at(bytes, 8, "UID3")?;
        if uid1 == UID1_DICTIONARY {
            return Err(RscError::Unsupported("dictionary-compressed resource file (UID1 0x101F5010)"));
        }
        if uid1 as u16 == 4 {
            return Err(RscError::Unsupported("Calypso resource file"));
        }
        if uid1 != UID1_PLAIN {
            return Err(RscError::Unsupported("UID1 is not 0x101F4A6B (not a plain Symbian resource file)"));
        }
        let flags = *bytes.get(16).ok_or(RscError::Truncated("flags byte"))?;
        if flags & (FLAG_FIRST_IS_BITS | FLAG_SIGNATURE_GENERATED | FLAG_UID3_OFFSET) != 0 {
            return Err(RscError::Unsupported("plain resource file with flag 0x20/0x40/0x80 set (first-resource bit array, generated signature, UID3 offset)"));
        }
        let tail = bytes.len().checked_sub(2).ok_or(RscError::Truncated("index offset"))?;
        let index_at = u16_at(bytes, tail, "index offset")? as usize;
        if index_at < BITS_AT || index_at > tail || (tail - index_at) % 2 != 0 {
            return Err(RscError::BadRuns("index offset is outside the file or odd"));
        }
        // The index is `n` entries then the final u16 (= its own offset, which is also the end of the last resource).
        let n = (tail - index_at) / 2;
        let mut bounds = Vec::with_capacity(n + 1);
        for i in 0..n {
            bounds.push(u16_at(bytes, index_at + 2 * i, "index entry")?);
        }
        bounds.push(index_at as u16);
        let bits_len = (n + 7) / 8;
        let data_start = BITS_AT + bits_len;
        let bits = bytes.get(BITS_AT..data_start).ok_or(RscError::Truncated("unicode bit array"))?;
        let mut prev = data_start;
        for &b in &bounds {
            let b = b as usize;
            if b < prev || b > index_at {
                return Err(RscError::BadRuns("resource offsets are not ascending inside the data"));
            }
            prev = b;
        }
        let mut file = RscFile { bytes, uid1, uid2, uid3, bounds, bits, id_offset: 0 };
        if n > 0 {
            if let Ok(first) = file.resource_at(0) {
                if first.len() == 8 && u32::from_le_bytes([first[0], first[1], first[2], first[3]]) == 4 {
                    file.id_offset = u32::from_le_bytes([first[4], first[5], first[6], first[7]]) & 0xFFFF_F000;
                }
            }
        }
        Ok(file)
    }

    pub fn count(&self) -> usize {
        self.bounds.len() - 1
    }

    /// Resource `id` (1-based; high 20 bits 0 or this file's offset), decoded: the unicode run layer is expanded.
    pub fn resource(&self, id: u32) -> Result<Vec<u8>, RscError> {
        let off = id & 0xFFFF_F000;
        let n = (id & 0xFFF) as usize;
        if (off != 0 && off != self.id_offset) || n == 0 || n > self.count() {
            return Err(RscError::NotOwned(id));
        }
        self.resource_at(n - 1)
    }

    /// The raw bytes of resource `index` (0-based), exactly as stored.
    pub fn raw(&self, index: usize) -> Result<&'a [u8], RscError> {
        if index >= self.count() {
            return Err(RscError::NotOwned(index as u32 + 1));
        }
        self.bytes.get(self.bounds[index] as usize..self.bounds[index + 1] as usize).ok_or(RscError::Truncated("resource"))
    }

    fn resource_at(&self, index: usize) -> Result<Vec<u8>, RscError> {
        let raw = self.raw(index)?;
        let unicode = self.bits.get(index / 8).map_or(false, |b| b & (1 << (index % 8)) != 0);
        if unicode { expand_runs(raw) } else { Ok(raw.to_vec()) }
    }
}

/// The run layer: alternating unicode / raw runs, unicode first.
fn expand_runs(data: &[u8]) -> Result<Vec<u8>, RscError> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut at = 0;
    let mut unicode = true;
    while at < data.len() {
        let mut len = data[at] as usize;
        at += 1;
        if len & 0x80 != 0 {
            len = (len & 0x7F) << 8 | *data.get(at).ok_or(RscError::BadRuns("run header cut short"))? as usize;
            at += 1;
        }
        let run = data.get(at..at + len).ok_or(RscError::BadRuns("run is longer than the resource"))?;
        at += len;
        if len == 0 {
            unicode = !unicode;
            continue;
        }
        if unicode {
            if out.len() % 2 == 1 {
                out.push(0xAB);
            }
            scsu_expand(run, &mut out)?;
        } else {
            out.extend_from_slice(run);
        }
        unicode = !unicode;
    }
    Ok(out)
}

/// SCSU subset: single-byte mode, the default dynamic window 0 (base 0x0080), static ASCII.
fn scsu_expand(run: &[u8], out: &mut Vec<u8>) -> Result<(), RscError> {
    for &b in run {
        let unit: u16 = match b {
            0x00 | 0x09 | 0x0A | 0x0D | 0x20..=0x7F => b as u16,
            0x80..=0xFF => 0x0080 + (b as u16 - 0x80),
            other => return Err(RscError::Scsu(other)),
        };
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

/// A cursor over one decoded resource. Integers are little-endian; `ltext` is the length-prefixed 16-bit string
/// (`LTEXT`: a length byte, or two bytes with bit 7 set as in the run header, then 2-aligned UTF-16LE).
pub struct Reader<'a> {
    data: &'a [u8],
    pub at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], RscError> {
        let s = self.data.get(self.at..self.at + n).ok_or(RscError::Field(what))?;
        self.at += n;
        Ok(s)
    }
    pub fn byte(&mut self) -> Result<u8, RscError> {
        Ok(self.take(1, "BYTE")?[0])
    }
    pub fn word(&mut self) -> Result<u16, RscError> {
        let s = self.take(2, "WORD")?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn long(&mut self) -> Result<u32, RscError> {
        let s = self.take(4, "LONG")?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn ltext(&mut self) -> Result<String, RscError> {
        let mut len = self.byte()? as usize;
        if len & 0x80 != 0 {
            len = (len & 0x7F) << 8 | self.byte()? as usize;
        }
        if len == 0 {
            return Ok(String::new());
        }
        if self.at % 2 == 1 {
            self.at += 1; // the 0xAB pad
        }
        let s = self.take(len * 2, "LTEXT body")?;
        let units = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
        Ok(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect())
    }
    /// The whole resource as UTF-16LE text with no length prefix (`TBUF`/`LBUF`-style bodies).
    pub fn rest_utf16(&mut self) -> Result<String, RscError> {
        let s = self.take(self.data.len() - self.at.min(self.data.len()), "text")?;
        let units = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
        Ok(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect())
    }
}

#[cfg(test)]
mod tests;
