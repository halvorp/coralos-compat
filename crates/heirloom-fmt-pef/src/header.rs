//! Container header, section headers, section names and section images.
//!
//! ```text
//! container header (40): 'Joy!' 'peff' arch('pwpc') u32 formatVersion(1) u32 dateTimeStamp u32 oldDefVersion
//!                        u32 oldImpVersion u32 currentVersion u16 sectionCount u16 instSectionCount u32 reserved
//! section header  (28):  i32 nameOffset(-1 none) u32 defaultAddress u32 totalSize u32 unpackedSize u32 packedSize
//!                        u32 containerOffset u8 sectionKind u8 shareKind u8 alignment u8 reserved
//! ```
//! Section names follow the section headers. The first `instSectionCount` sections are the ones instantiated in
//! memory; the others (loader, debug, ...) stay in the container.
use crate::loader::Loader;
use crate::{be16, be32, pidata, PefError};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const HEADER_SIZE: usize = 40;
pub const SECTION_HEADER_SIZE: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind { Code, UnpackedData, PatternData, Constant, Loader, Debug, ExecutableData, Exception, Traceback }

impl SectionKind {
    pub fn from_u8(v: u8) -> Option<SectionKind> {
        Some(match v { 0 => Self::Code, 1 => Self::UnpackedData, 2 => Self::PatternData, 3 => Self::Constant, 4 => Self::Loader,
                       5 => Self::Debug, 6 => Self::ExecutableData, 7 => Self::Exception, 8 => Self::Traceback, _ => return None })
    }
    pub fn to_u8(self) -> u8 { self as u8 }
    /// True for the kinds that occupy memory when the fragment is instantiated.
    pub fn instantiated(self) -> bool { matches!(self, Self::Code | Self::UnpackedData | Self::PatternData | Self::Constant | Self::ExecutableData) }
    /// True when the instantiated section is writable (data sections).
    pub fn writable(self) -> bool { matches!(self, Self::UnpackedData | Self::PatternData) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub arch: [u8; 4],
    pub format_version: u32,
    pub timestamp: u32,
    pub old_def_version: u32,
    pub old_imp_version: u32,
    pub current_version: u32,
    pub section_count: u16,
    pub inst_section_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionHeader {
    pub name_offset: i32,
    pub default_address: u32,
    pub total_size: u32,
    pub unpacked_size: u32,
    pub packed_size: u32,
    pub container_offset: u32,
    pub kind: SectionKind,
    pub share_kind: u8,
    /// The alignment exponent: the section starts on a multiple of `1 << alignment` bytes.
    pub alignment: u8,
}

/// A validated container. Section extents are checked once, here.
#[derive(Debug, Clone)]
pub struct Pef<'a> {
    bytes: &'a [u8],
    pub header: Header,
    pub sections: Vec<SectionHeader>,
}

impl<'a> Pef<'a> {
    /// True when `bytes` starts with the PEF magic (a quick test before [`Pef::parse`]).
    pub fn has_magic(bytes: &[u8]) -> bool { bytes.get(..8) == Some(b"Joy!peff") }

    pub fn parse(bytes: &'a [u8]) -> Result<Pef<'a>, PefError> {
        if !Self::has_magic(bytes) { return Err(PefError::BadMagic); }
        let t = PefError::Truncated("container header");
        if bytes.len() < HEADER_SIZE { return Err(t); }
        let arch: [u8; 4] = bytes[8..12].try_into().unwrap();
        let header = Header {
            arch, format_version: be32(bytes, 12).unwrap(), timestamp: be32(bytes, 16).unwrap(),
            old_def_version: be32(bytes, 20).unwrap(), old_imp_version: be32(bytes, 24).unwrap(),
            current_version: be32(bytes, 28).unwrap(), section_count: be16(bytes, 32).unwrap(), inst_section_count: be16(bytes, 34).unwrap(),
        };
        if &arch != b"pwpc" { return Err(PefError::UnsupportedArchitecture(arch)); }
        if header.format_version != 1 { return Err(PefError::UnsupportedVersion(header.format_version)); }
        if header.inst_section_count > header.section_count { return Err(PefError::BadSection { index: usize::from(header.inst_section_count), why: "instSectionCount exceeds sectionCount" }); }
        let mut sections = Vec::with_capacity(usize::from(header.section_count));
        for i in 0..usize::from(header.section_count) {
            let at = HEADER_SIZE + i * SECTION_HEADER_SIZE;
            let s = bytes.get(at..at + SECTION_HEADER_SIZE).ok_or(PefError::Truncated("section header table"))?;
            let kind = SectionKind::from_u8(s[24]).ok_or(PefError::BadSection { index: i, why: "unknown section kind" })?;
            let sh = SectionHeader {
                name_offset: be32(s, 0).unwrap() as i32, default_address: be32(s, 4).unwrap(), total_size: be32(s, 8).unwrap(),
                unpacked_size: be32(s, 12).unwrap(), packed_size: be32(s, 16).unwrap(), container_offset: be32(s, 20).unwrap(),
                kind, share_kind: s[25], alignment: s[26],
            };
            let end = u64::from(sh.container_offset) + u64::from(sh.packed_size);
            if end > bytes.len() as u64 { return Err(PefError::BadSection { index: i, why: "its bytes run past the end of the file" }); }
            if sh.kind.instantiated() && sh.unpacked_size > sh.total_size { return Err(PefError::BadSection { index: i, why: "unpackedSize exceeds totalSize" }); }
            if sh.kind.instantiated() && sh.kind != SectionKind::PatternData && sh.packed_size != sh.unpacked_size {
                return Err(PefError::BadSection { index: i, why: "packedSize differs from unpackedSize in a section that is not pattern-initialised" });
            }
            if sh.alignment > 31 { return Err(PefError::BadSection { index: i, why: "alignment exponent past 31" }); }
            if i >= usize::from(header.inst_section_count) && sh.kind.instantiated() {
                return Err(PefError::BadSection { index: i, why: "an instantiated kind past instSectionCount" });
            }
            sections.push(sh);
        }
        let pef = Pef { bytes, header, sections };
        for i in 0..pef.sections.len() { pef.section_name(i)?; }
        Ok(pef)
    }

    pub fn bytes(&self) -> &'a [u8] { self.bytes }

    /// A section's bytes as stored in the container.
    pub fn section_bytes(&self, i: usize) -> Result<&'a [u8], PefError> {
        let s = self.sections.get(i).ok_or(PefError::BadSection { index: i, why: "no such section" })?;
        Ok(&self.bytes[s.container_offset as usize..s.container_offset as usize + s.packed_size as usize])
    }

    /// The section's name, when it has one.
    pub fn section_name(&self, i: usize) -> Result<Option<String>, PefError> {
        let s = self.sections.get(i).ok_or(PefError::BadSection { index: i, why: "no such section" })?;
        if s.name_offset < 0 { return Ok(None); }
        let table = HEADER_SIZE + self.sections.len() * SECTION_HEADER_SIZE;
        let start = table + s.name_offset as usize;
        let rest = self.bytes.get(start..).ok_or(PefError::BadSection { index: i, why: "name offset past the end of the file" })?;
        let end = rest.iter().position(|&c| c == 0).ok_or(PefError::BadSection { index: i, why: "section name is not terminated" })?;
        Ok(Some(rest[..end].iter().map(|&c| char::from(c)).collect()))
    }

    /// The section as it is in memory: `totalSize` bytes, pattern data expanded, the rest zero. Only instantiated kinds.
    pub fn section_image(&self, i: usize) -> Result<Vec<u8>, PefError> {
        let s = *self.sections.get(i).ok_or(PefError::BadSection { index: i, why: "no such section" })?;
        if !s.kind.instantiated() { return Err(PefError::BadSection { index: i, why: "not an instantiated section kind" }); }
        let src = self.section_bytes(i)?;
        let mut out = vec![0u8; s.total_size as usize];
        if s.kind == SectionKind::PatternData { pidata::unpack(src, &mut out[..s.unpacked_size as usize])?; }
        else { out[..src.len()].copy_from_slice(src); }
        Ok(out)
    }

    /// The loader section, parsed. A fragment has exactly one.
    pub fn loader(&self) -> Result<Loader, PefError> {
        let mut it = self.sections.iter().enumerate().filter(|(_, s)| s.kind == SectionKind::Loader);
        let (i, _) = it.next().ok_or(PefError::BadLoader("the container has no loader section"))?;
        if it.next().is_some() { return Err(PefError::BadLoader("more than one loader section")); }
        Loader::parse(self.section_bytes(i)?, self.sections.len())
    }
}
