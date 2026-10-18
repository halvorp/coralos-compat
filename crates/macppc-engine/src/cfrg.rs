//! The `cfrg` (code fragment) resource: tells the CFM where an application's fragment lives.
//!
//! ```text
//! header (32): u32 res u32 res u16 res u16 version(1) u32*4 res u16 res u16 memberCount
//! member (42 + name): 'pwpc'|'m68k' u16 res u8 res u8 updateLevel u32 currentVersion u32 oldDefVersion
//!   u32 appStackSize u16 appSubFolder u8 usage u8 where u32 offset u32 length u32 uWhere1 u16 uWhere2
//!   u16 extensionCount u16 memberSize  Str31 name, padded so the member is a multiple of 4 bytes
//! ```
//! usage: 0 import library, 1 application, 2 drop-in addition, 3 stub library, 4 weak stub library.
//! where: 0 memory, 1 data fork, 2 resource fork. length 0 means "to the end of the fork".
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

pub const USAGE_IMPORT_LIBRARY: u8 = 0;
pub const USAGE_APPLICATION: u8 = 1;
pub const WHERE_DATA_FORK: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub arch: [u8; 4],
    pub usage: u8,
    pub location: u8,
    pub offset: u32,
    pub length: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfrgError {
    Truncated(&'static str),
    BadVersion(u16),
    BadMemberSize { index: usize },
}
impl fmt::Display for CfrgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CfrgError::Truncated(w) => write!(f, "cfrg resource truncated: {w}"),
            CfrgError::BadVersion(v) => write!(f, "cfrg resource version {v} is not 1"),
            CfrgError::BadMemberSize { index } => write!(f, "cfrg member {index} has a size that is not a multiple of 4 or overruns the resource"),
        }
    }
}
impl core::error::Error for CfrgError {}

use retro_common::bytes::{be16, be32};

pub fn parse(b: &[u8]) -> Result<Vec<Member>, CfrgError> {
    let t = CfrgError::Truncated("header");
    let version = be16(b, 10).ok_or(t.clone())?;
    if version != 1 { return Err(CfrgError::BadVersion(version)); }
    let count = be16(b, 30).ok_or(t)? as usize;
    let (mut at, mut out) = (32usize, Vec::new());
    for index in 0..count {
        let t = CfrgError::Truncated("member");
        let size = be16(b, at + 40).ok_or(t.clone())? as usize;
        if size < 42 || size % 4 != 0 || at + size > b.len() { return Err(CfrgError::BadMemberSize { index }); }
        let nlen = *b.get(at + 42).ok_or(t.clone())? as usize;
        let name = b.get(at + 43..at + 43 + nlen).filter(|_| 43 + nlen <= size).ok_or(CfrgError::BadMemberSize { index })?;
        out.push(Member {
            arch: b[at..at + 4].try_into().unwrap(), usage: b[at + 22], location: b[at + 23], offset: be32(b, at + 24).ok_or(t.clone())?,
            length: be32(b, at + 28).ok_or(t)?, name: name.iter().map(|&c| char::from(c)).collect(),
        });
        at += size;
    }
    Ok(out)
}

/// A cfrg resource with one member, for tests and tools.
pub fn build(arch: &[u8; 4], usage: u8, location: u8, offset: u32, length: u32, name: &str) -> Vec<u8> {
    let mut out = alloc::vec![0u8; 32];
    out[10..12].copy_from_slice(&1u16.to_be_bytes());
    out[30..32].copy_from_slice(&1u16.to_be_bytes());
    let size = (43 + name.len()).next_multiple_of(4);
    let mut m = alloc::vec![0u8; size];
    m[..4].copy_from_slice(arch);
    m[22] = usage; m[23] = location;
    m[24..28].copy_from_slice(&offset.to_be_bytes()); m[28..32].copy_from_slice(&length.to_be_bytes());
    m[40..42].copy_from_slice(&(size as u16).to_be_bytes());
    m[42] = name.len() as u8; m[43..43 + name.len()].copy_from_slice(name.as_bytes());
    out.extend_from_slice(&m);
    out
}
