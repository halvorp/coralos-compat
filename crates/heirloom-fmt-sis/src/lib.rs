//! Symbian SIS/SISX install package reader.
#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

mod controller;
mod data;
mod field;

/// A parsed SISX installation package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub uids: (u32, u32, u32),
    pub package_name: String,
    pub vendor: String,
    pub version: (u32, u32, u32),
    pub supported_languages: Vec<u32>,
    pub files: Vec<SisFile>,
}

/// A file described by the controller's install block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SisFile {
    pub target: String,
    pub mime: String,
    pub operation: u32,
    pub data_index: u32,
    pub bytes: Vec<u8>,
    pub is_conditional: bool,
}

/// A malformed, truncated, or unsupported package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SisError {
    Truncated(&'static str),
    Malformed(&'static str),
    Unsupported(&'static str),
    InvalidUtf16,
    Decompression,
    SizeMismatch { expected: u64, actual: u64 },
    Missing(&'static str),
    DataIndex(u32),
}

impl Package {
    /// Parse a SISX package, including its compressed controller and file data.
    pub fn parse(bytes: &[u8]) -> Result<Package, SisError> {
        use field::{Field, Reader};

        if bytes.len() < 16 {
            return Err(SisError::Truncated("SIS header"));
        }

        let mut head = Reader::new(&bytes[..12]);
        let (uid1, uid2, uid3) = (head.read_u32()?, head.read_u32()?, head.read_u32()?);

        // These are pre-SISX executable/image UIDs, not installable SISX files.
        if matches!(uid1, 0x1000_007a | 0x1000_007b | 0x1000_007c) {
            return Err(SisError::Unsupported("legacy SIS or E32 image"));
        }

        let mut root = Reader::new(&bytes[16..]);
        let contents = root.read_field()?;
        root.finish()?;
        if contents.kind != field::SIS_CONTENTS {
            return Err(SisError::Malformed("missing SISContents"));
        }

        let mut compressed_controller = None;
        let mut sis_data = None;
        let mut fields = Reader::new(contents.data);
        while !fields.is_empty() {
            let Field { kind, data } = fields.read_field()?;
            match kind {
                field::SIS_COMPRESSED => {
                    if compressed_controller.replace(data).is_some() {
                        return Err(SisError::Malformed("duplicate compressed controller"));
                    }
                }
                field::SIS_DATA => {
                    if sis_data.replace(data).is_some() {
                        return Err(SisError::Malformed("duplicate SISData"));
                    }
                }
                // Checksums and future contents fields do not affect extraction.
                _ => {}
            }
        }

        let compressed_controller =
            compressed_controller.ok_or(SisError::Missing("compressed controller"))?;
        let sis_data = sis_data.ok_or(SisError::Missing("SISData"))?;

        let controller_bytes = data::decompress_field(compressed_controller)?;
        let description = controller::parse(&controller_bytes)?;
        let file_data = data::parse(sis_data)?;

        let mut files = Vec::with_capacity(description.files.len());
        for file in description.files {
            let bytes = file_data.file_bytes(file.data_index)?.to_vec();
            files.push(SisFile {
                target: file.target,
                mime: file.mime,
                operation: file.operation,
                data_index: file.data_index,
                bytes,
                is_conditional: file.is_conditional,
            });
        }

        Ok(Package {
            uids: (uid1, uid2, uid3),
            package_name: description.package_name,
            vendor: description.vendor,
            version: description.version,
            supported_languages: description.supported_languages,
            files,
        })
    }
}