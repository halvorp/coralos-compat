//! Symbian EKA2 E32 image reader: the header, the code and data sections, the (dll, ordinal) imports with the code
//! words that bind them, the export directory and the relocation tables, plus relocation to a load address.
//!
//! Layout facts (`E32ImageHeaderV`, the format EKA2 toolchains emit) were checked against real S60 5th edition images;
//! every offset and count is range-checked against the file, and anything this reader does not handle is refused by
//! name. A deflate-compressed image is the header followed by everything else compressed; its header offsets refer to
//! the decompressed image, so it is decompressed (`inflate`) and then read like any other. Byte-pair compression is
//! refused by name.
#![no_std]
extern crate alloc;

mod header;
mod imports;
mod inflate;
mod ordinals;
mod reloc;

use alloc::vec::Vec;

pub use header::{uid_checksum, Compression, E32Header};
pub use imports::{Import, ImportedDll};
pub use ordinals::{dso_exports, DsoError, DsoExport, OrdinalTable};
pub use reloc::{Reloc, RelocKind};

/// Why an image cannot be read. Each names the field or rule it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E32Error {
    /// The file ends inside the named structure.
    Truncated(&'static str),
    /// UID1 is neither a DLL's nor an EXE's, or the "EPOC" signature is missing.
    NotE32,
    /// The UID checksum does not match UID1..UID3.
    UidChecksum,
    /// The image uses a feature this reader refuses by name.
    Unsupported(&'static str),
    /// A count or offset is inconsistent with the rest of the image.
    Malformed(&'static str),
}

/// One E32 image, sections copied out of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E32Image {
    pub header: E32Header,
    /// `iCodeSize` bytes: text, then the export directory and the import address words.
    pub code: Vec<u8>,
    /// `iDataSize` bytes of initialised data (`bss` follows it at load and is not in the file).
    pub data: Vec<u8>,
    pub imports: Vec<ImportedDll>,
    /// Export directory: each entry the export's address at the image's link-time code base.
    pub exports: Vec<u32>,
    /// Relocations of words in `code` and in `data`.
    pub code_relocs: Vec<Reloc>,
    pub data_relocs: Vec<Reloc>,
}

impl E32Image {
    pub fn parse(file: &[u8]) -> Result<Self, E32Error> {
        let header = E32Header::parse(file)?;
        match header.compression {
            Compression::None => Self::read(file, header),
            Compression::Deflate => {
                let head = slice(file, 0, header.code_offset, "E32 header")?;
                let rest = file.get(head.len()..).ok_or(E32Error::Truncated("compressed image"))?;
                let mut image = head.to_vec();
                image.extend(inflate::inflate(rest, header.uncompressed_size as usize)?);
                Self::read(&image, header)
            }
            Compression::BytePair => Err(E32Error::Unsupported("byte-pair compression")),
        }
    }

    /// Sections, imports, exports and relocations of an uncompressed image `file` described by `header`.
    fn read(file: &[u8], header: E32Header) -> Result<Self, E32Error> {
        let code = slice(file, header.code_offset, header.code_size, "code section")?.to_vec();
        let data = slice(file, header.data_offset, header.data_size, "data section")?.to_vec();
        let imports = imports::parse(file, &header, &code)?;
        let exports = exports(file, &header)?;
        let code_relocs = reloc::parse(file, header.code_reloc_offset, code.len())?;
        let data_relocs = reloc::parse(file, header.data_reloc_offset, data.len())?;
        Ok(Self { header, code, data, imports, exports, code_relocs, data_relocs })
    }

    /// Rebase both sections to where they are loaded: every relocated word moves by the delta of the section its
    /// value points into.
    pub fn relocate(&mut self, code_at: u32, data_at: u32) -> Result<(), E32Error> {
        let h = &self.header;
        let deltas = reloc::Deltas {
            code_base: h.code_base,
            code_size: h.code_size,
            code: code_at.wrapping_sub(h.code_base),
            data: data_at.wrapping_sub(h.data_base),
        };
        reloc::apply(&mut self.code, &self.code_relocs, &deltas)?;
        reloc::apply(&mut self.data, &self.data_relocs, &deltas)
    }
}

fn exports(file: &[u8], h: &E32Header) -> Result<Vec<u32>, E32Error> {
    if h.export_dir_count == 0 {
        return Ok(Vec::new());
    }
    let len = h.export_dir_count.checked_mul(4).ok_or(E32Error::Malformed("export count"))?;
    let dir = slice(file, h.export_dir_offset, len, "export directory")?;
    (0..dir.len()).step_by(4).map(|at| u32_at(dir, at, "export directory")).collect()
}

/// `len` bytes at `offset` of `bytes`, or the named truncation.
pub(crate) fn slice<'a>(bytes: &'a [u8], offset: u32, len: u32, what: &'static str) -> Result<&'a [u8], E32Error> {
    let start = offset as usize;
    let end = start.checked_add(len as usize).ok_or(E32Error::Truncated(what))?;
    bytes.get(start..end).ok_or(E32Error::Truncated(what))
}

pub(crate) fn u32_at(bytes: &[u8], offset: usize, what: &'static str) -> Result<u32, E32Error> {
    let end = offset.checked_add(4).ok_or(E32Error::Truncated(what))?;
    let b = bytes.get(offset..end).ok_or(E32Error::Truncated(what))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}