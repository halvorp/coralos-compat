//! `E32ImageHeaderV`: the fixed part this reader uses, by offset.
use crate::{u32_at, E32Error};

/// UID1 of a DLL image (KDynamicLibraryUid) and of an EXE image (KExecutableImageUid).
const UID_DLL: u32 = 0x1000_0079;
const UID_EXE: u32 = 0x1000_007A;
const SIGNATURE: &[u8; 4] = b"EPOC";
const UID_DEFLATE: u32 = 0x101F_7AFC;
const UID_BYTE_PAIR: u32 = 0x1028_22AA;
/// `iFlags` bits 24..27: the header format; 2 = `E32ImageHeaderV`.
const HDR_FMT_SHIFT: u32 = 24;
const HDR_FMT_V: u32 = 2;
/// `iFlags` bits 28..31: the import format; 1 = ELF-derived (import entries are code offsets).
const IMP_FMT_SHIFT: u32 = 28;
const IMP_FMT_ELF: u32 = 1;
/// `iFlags` bit 0: the image is a DLL.
const FLAG_DLL: u32 = 1;
/// The fixed part of `E32ImageHeaderV` read here ends with the security info at 0x80..0x90.
const FIXED_LEN: usize = 0x90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Deflate,
    BytePair,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E32Header {
    pub uid1: u32,
    pub uid2: u32,
    pub uid3: u32,
    pub compression: Compression,
    pub flags: u32,
    pub is_dll: bool,
    pub code_size: u32,
    pub data_size: u32,
    pub heap_min: u32,
    pub heap_max: u32,
    pub stack_size: u32,
    pub bss_size: u32,
    /// Entry point, an offset into the code section.
    pub entry_point: u32,
    /// Link-time addresses of the code and data sections.
    pub code_base: u32,
    pub data_base: u32,
    pub dll_ref_count: u32,
    pub export_dir_offset: u32,
    pub export_dir_count: u32,
    pub text_size: u32,
    pub code_offset: u32,
    pub data_offset: u32,
    pub import_offset: u32,
    pub code_reloc_offset: u32,
    pub data_reloc_offset: u32,
    pub secure_id: u32,
    pub vendor_id: u32,
    /// Capability bits (`SCapabilitySet`, two words).
    pub capabilities: u64,
}

impl E32Header {
    pub fn parse(file: &[u8]) -> Result<Self, E32Error> {
        if file.len() < FIXED_LEN {
            return Err(E32Error::Truncated("E32 header"));
        }
        let w = |o: usize| u32_at(file, o, "E32 header");
        if !matches!(w(0x00)?, UID_DLL | UID_EXE) || &file[0x10..0x14] != SIGNATURE {
            return Err(E32Error::NotE32);
        }
        if uid_checksum(&file[..12]) != w(0x0C)? {
            return Err(E32Error::UidChecksum);
        }
        let flags = w(0x2C)?;
        if (flags >> HDR_FMT_SHIFT) & 0xF != HDR_FMT_V {
            return Err(E32Error::Unsupported("header format other than E32ImageHeaderV"));
        }
        if flags >> IMP_FMT_SHIFT != IMP_FMT_ELF {
            return Err(E32Error::Unsupported("import format other than ELF-derived"));
        }
        let compression = match w(0x1C)? {
            0 => Compression::None,
            UID_DEFLATE => Compression::Deflate,
            UID_BYTE_PAIR => Compression::BytePair,
            _ => return Err(E32Error::Unsupported("unknown compression")),
        };
        Ok(Self {
            uid1: w(0x00)?,
            uid2: w(0x04)?,
            uid3: w(0x08)?,
            compression,
            flags,
            is_dll: flags & FLAG_DLL != 0,
            code_size: w(0x30)?,
            data_size: w(0x34)?,
            heap_min: w(0x38)?,
            heap_max: w(0x3C)?,
            stack_size: w(0x40)?,
            bss_size: w(0x44)?,
            entry_point: w(0x48)?,
            code_base: w(0x4C)?,
            data_base: w(0x50)?,
            dll_ref_count: w(0x54)?,
            export_dir_offset: w(0x58)?,
            export_dir_count: w(0x5C)?,
            text_size: w(0x60)?,
            code_offset: w(0x64)?,
            data_offset: w(0x68)?,
            import_offset: w(0x6C)?,
            code_reloc_offset: w(0x70)?,
            data_reloc_offset: w(0x74)?,
            secure_id: w(0x80)?,
            vendor_id: w(0x84)?,
            capabilities: u64::from(w(0x88)?) | u64::from(w(0x8C)?) << 32,
        })
    }
}

/// `TCheckedUid`: CRC-16/CCITT of UID bytes 1,3,5.. in the high half, of bytes 0,2,4.. in the low half.
pub fn uid_checksum(uids: &[u8]) -> u32 {
    let crc = |start: usize| {
        uids.iter().skip(start).step_by(2).fold(0u16, |mut crc, &b| {
            crc ^= u16::from(b) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
            crc
        })
    };
    u32::from(crc(1)) << 16 | u32::from(crc(0))
}
