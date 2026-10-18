//! Where a resource fork comes from: a MacBinary II file, an AppleDouble header file, or the raw fork itself.
//!
//! MacBinary II: a 128-byte header (byte 0 = 0, byte 1.. = Pascal file name, 65 file type, 69 creator, byte 74 = 0,
//! 83 data fork length, 87 resource fork length, 124 u16 CRC of bytes 0..124 = CRC-16/XMODEM, polynomial 0x1021,
//! initial value 0), then the data fork, padded to a multiple of 128, then the resource fork. (Facts from the format,
//! cross-read against metamage_1's macbinary + macbinary-crc: header offsets and the 124-byte CRC span.)
//! AppleDouble: magic 0x00051607, version 0x00020000, 16 filler bytes, u16 entry count, then 12-byte descriptors
//! (u32 id, u32 offset, u32 length); id 2 is the resource fork, 1 the data fork (which an AppleDouble pair keeps in
//! the *other* file, so it is never found here), 3 the real name.
use crate::{be16, be32, OsType};
use alloc::string::String;
use core::fmt;
use core::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { MacBinary, AppleDouble, AppleSingle, Raw }

/// What [`open`] found: which container, the file's identity when it carries one, and the byte ranges of the forks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub kind: Kind,
    pub name: Option<String>,
    pub file_type: Option<OsType>,
    pub creator: Option<OsType>,
    /// The data fork, when this container holds it (MacBinary, AppleSingle).
    pub data: Option<Range<usize>>,
    /// The resource fork bytes.
    pub rsrc: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerError {
    /// MacBinary header whose CRC does not match its first 124 bytes.
    BadCrc { stored: u16, computed: u16 },
    /// MacBinary header that fails a structural rule (name length, the zero bytes, a fork past the end of the file).
    BadMacBinary(&'static str),
    /// AppleDouble/AppleSingle structure error.
    BadAppleDouble(&'static str),
    /// The container is well formed but has no resource fork (an empty one or no descriptor 2).
    NoResourceFork,
}
impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::BadCrc { stored, computed } => write!(f, "MacBinary CRC {stored:#06x} does not match the header ({computed:#06x})"),
            ContainerError::BadMacBinary(w) => write!(f, "bad MacBinary header: {w}"),
            ContainerError::BadAppleDouble(w) => write!(f, "bad AppleDouble/AppleSingle file: {w}"),
            ContainerError::NoResourceFork => write!(f, "the file has no resource fork"),
        }
    }
}
impl core::error::Error for ContainerError {}

/// CRC-16/XMODEM (poly 0x1021, init 0), MacBinary's header checksum.
pub fn crc16(b: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in b {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 { crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 }; }
    }
    crc
}

pub const AD_MAGIC: u32 = 0x0005_1607;
pub const AS_MAGIC: u32 = 0x0005_1600;
pub const AD_VERSION: u32 = 0x0002_0000;

/// Identify `bytes` and locate the resource fork. A file that is neither MacBinary nor AppleDouble/AppleSingle is
/// taken as a raw fork (the caller's [`crate::ResFork::parse`] then validates it).
pub fn open(bytes: &[u8]) -> Result<Opened, ContainerError> {
    match be32(bytes, 0) {
        Some(AD_MAGIC) => return apple(bytes, Kind::AppleDouble),
        Some(AS_MAGIC) => return apple(bytes, Kind::AppleSingle),
        _ => {}
    }
    if looks_like_macbinary(bytes) { return macbinary(bytes); }
    Ok(Opened { kind: Kind::Raw, name: None, file_type: None, creator: None, data: None, rsrc: 0..bytes.len() })
}

/// The cheap signature test: a full header whose zero bytes and name length are right. The CRC is checked by
/// [`macbinary`] so a header that merely *looks* right but is corrupt is reported, not mistaken for a raw fork.
fn looks_like_macbinary(b: &[u8]) -> bool {
    b.len() >= 128 && b[0] == 0 && b[74] == 0 && b[82] == 0 && (1..=63).contains(&b[1])
        && (be16(b, 124) == Some(crc16(&b[..124])) || b[122] >= 129)
}

fn macbinary(b: &[u8]) -> Result<Opened, ContainerError> {
    let bad = ContainerError::BadMacBinary;
    let stored = be16(b, 124).unwrap();
    let computed = crc16(&b[..124]);
    if stored != computed { return Err(ContainerError::BadCrc { stored, computed }); }
    let (dlen, rlen) = (be32(b, 83).unwrap() as u64, be32(b, 87).unwrap() as u64);
    let data_at = 128u64;
    let rsrc_at = data_at + dlen.div_ceil(128) * 128;
    if rsrc_at + rlen > b.len() as u64 { return Err(bad("a fork runs past the end of the file")); }
    if rlen == 0 { return Err(ContainerError::NoResourceFork); }
    let name = b[2..2 + b[1] as usize].iter().map(|&c| char::from(c)).collect();
    Ok(Opened {
        kind: Kind::MacBinary, name: Some(name), file_type: Some(b[65..69].try_into().unwrap()),
        creator: Some(b[69..73].try_into().unwrap()),
        data: Some(data_at as usize..(data_at + dlen) as usize), rsrc: rsrc_at as usize..(rsrc_at + rlen) as usize,
    })
}

fn apple(b: &[u8], kind: Kind) -> Result<Opened, ContainerError> {
    let bad = ContainerError::BadAppleDouble;
    if be32(b, 4) != Some(AD_VERSION) && be32(b, 4) != Some(0x0001_0000) { return Err(bad("unknown version")); }
    let n = be16(b, 24).ok_or(bad("truncated header"))? as usize;
    let (mut rsrc, mut data, mut name) = (None, None, None);
    for i in 0..n {
        let at = 26 + i * 12;
        let (id, off, len) = (be32(b, at).ok_or(bad("truncated descriptor table"))?, be32(b, at + 4).unwrap() as usize, be32(b, at + 8).unwrap() as usize);
        if off.checked_add(len).is_none_or(|e| e > b.len()) { return Err(bad("an entry runs past the end of the file")); }
        match id {
            1 => data = Some(off..off + len),
            2 => rsrc = Some(off..off + len),
            3 => name = Some(b[off..off + len].iter().map(|&c| char::from(c)).collect()),
            _ => {}
        }
    }
    match rsrc {
        Some(r) if !r.is_empty() => Ok(Opened { kind, name, file_type: None, creator: None, data, rsrc: r }),
        _ => Err(ContainerError::NoResourceFork),
    }
}
