#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

// ELF32 constants
const ELFCLASS32: u8 = 1;
const ELFDATA2LSB: u8 = 1;
const SHT_PROGBITS: u32 = 1;
const SHT_STRTAB: u32 = 3;
const SHT_DYNSYM: u32 = 11;
/// GNU symbol versioning: the version DEFINITIONS (one names the DLL) and the per-symbol version index table.
const SHT_GNU_VERDEF: u32 = 0x6fff_fffd;
const SHT_GNU_VERSYM: u32 = 0x6fff_ffff;
const SHF_ALLOC: u32 = 0x2;
const SHF_EXECINSTR: u32 = 0x4;
const STB_GLOBAL: u8 = 1;
const STT_FUNC: u8 = 2;
const STT_OBJECT: u8 = 1;
const ELF32_EHDR_SIZE: usize = 52;
const ELF32_SHDR_SIZE: usize = 40;
const ELF32_SYM_SIZE: usize = 16;

#[derive(Debug)]
pub struct DsoExport {
    pub dll: String,
    pub ordinal: u16,
    pub name: String,
}

#[derive(Debug)]
pub enum DsoError {
    NotElf32Le,
    Truncated(&'static str),
    NoSection(&'static str),
    BadOrdinal { name: String },
}

impl DsoError {
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotElf32Le => "not_elf32_le",
            Self::Truncated(_) => "truncated",
            Self::NoSection(_) => "no_section",
            Self::BadOrdinal { .. } => "bad_ordinal",
        }
    }
}

// Bounds-checked readers
fn rd_u16(b: &[u8], off: usize) -> Option<u16> {
    let end = off.checked_add(2)?;
    let s = b.get(off..end)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    let end = off.checked_add(4)?;
    let s = b.get(off..end)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn shdr_off(e_shoff: usize, e_shentsize: usize, i: usize) -> Option<usize> {
    i.checked_mul(e_shentsize)?.checked_add(e_shoff)
}

pub fn dso_exports(bytes: &[u8]) -> Result<Vec<DsoExport>, DsoError> {
    // 1. Validate ELF32 LE header
    if bytes.len() < ELF32_EHDR_SIZE {
        return Err(DsoError::Truncated("ehdr"));
    }
    if bytes.get(0..4) != Some(b"\x7fELF") {
        return Err(DsoError::NotElf32Le);
    }
    if bytes.get(4).copied() != Some(ELFCLASS32) || bytes.get(5).copied() != Some(ELFDATA2LSB) {
        return Err(DsoError::NotElf32Le);
    }

    let e_shoff = rd_u32(bytes, 32).ok_or(DsoError::Truncated("e_shoff"))? as usize;
    let e_shentsize = rd_u16(bytes, 46).ok_or(DsoError::Truncated("e_shentsize"))? as usize;
    let mut e_shnum = rd_u16(bytes, 48).ok_or(DsoError::Truncated("e_shnum"))? as usize;
    if e_shoff == 0 {
        return Err(DsoError::NoSection("section headers"));
    }

    // Extended numbering: if e_shnum == 0, real count is in section[0].sh_size
    if e_shnum == 0 {
        let size_off = e_shoff.checked_add(20).ok_or(DsoError::Truncated("sh0_size"))?;
        e_shnum = rd_u32(bytes, size_off).ok_or(DsoError::Truncated("sh0_size"))? as usize;
    }
    if e_shnum == 0 {
        return Err(DsoError::NoSection("section headers"));
    }
    if e_shentsize < ELF32_SHDR_SIZE {
        return Err(DsoError::Truncated("e_shentsize"));
    }
    let table_len = e_shnum.checked_mul(e_shentsize).ok_or(DsoError::Truncated("shdr"))?;
    let table_end = e_shoff.checked_add(table_len).ok_or(DsoError::Truncated("shdr"))?;
    if table_end > bytes.len() {
        return Err(DsoError::Truncated("shdr"));
    }

    // 2. Parse section headers, find ER_RO, .dynsym, strtab
    let mut er_ro_idx: Option<usize> = None;
    let mut dynsym_off: Option<usize> = None;
    let mut dynsym_size: Option<usize> = None;
    let mut dynsym_entsize: Option<usize> = None;
    let mut dynsym_link: Option<usize> = None;
    let mut er_ro_file_off: usize = 0;
    let mut verdef: Option<(usize, usize, usize)> = None; // (offset, size, string table section)
    let mut versym: Option<usize> = None; // offset of the u16-per-dynsym table

    for i in 0..e_shnum {
        let off = shdr_off(e_shoff, e_shentsize, i).ok_or(DsoError::Truncated("shdr"))?;
        let sh_type = rd_u32(bytes, off + 4).ok_or(DsoError::Truncated("sh_type"))?;
        let sh_flags = rd_u32(bytes, off + 8).ok_or(DsoError::Truncated("sh_flags"))?;
        let sh_offset = rd_u32(bytes, off + 16).ok_or(DsoError::Truncated("sh_offset"))? as usize;
        let sh_size = rd_u32(bytes, off + 20).ok_or(DsoError::Truncated("sh_size"))? as usize;
        let sh_link = rd_u32(bytes, off + 24).ok_or(DsoError::Truncated("sh_link"))? as usize;
        let sh_entsize = rd_u32(bytes, off + 36).ok_or(DsoError::Truncated("sh_entsize"))? as usize;

        // ER_RO: PROGBITS with ALLOC|EXECINSTR
        if sh_type == SHT_PROGBITS
            && (sh_flags & (SHF_ALLOC | SHF_EXECINSTR)) == (SHF_ALLOC | SHF_EXECINSTR)
            && er_ro_idx.is_none()
        {
            er_ro_idx = Some(i);
            er_ro_file_off = sh_offset;
        }
        if sh_type == SHT_GNU_VERDEF {
            verdef = Some((sh_offset, sh_size, sh_link));
        }
        if sh_type == SHT_GNU_VERSYM {
            versym = Some(sh_offset);
        }
        if sh_type == SHT_DYNSYM {
            dynsym_off = Some(sh_offset);
            dynsym_size = Some(sh_size);
            dynsym_entsize = Some(if sh_entsize == 0 { ELF32_SYM_SIZE } else { sh_entsize });
            dynsym_link = Some(sh_link);
        }
    }

    let er_ro_idx = er_ro_idx.ok_or(DsoError::NoSection("ER_RO"))?;
    let dynsym_off = dynsym_off.ok_or(DsoError::NoSection(".dynsym"))?;
    let dynsym_size = dynsym_size.ok_or(DsoError::NoSection(".dynsym"))?;
    let dynsym_entsize = dynsym_entsize.ok_or(DsoError::NoSection(".dynsym"))?;
    let dynsym_link = dynsym_link.ok_or(DsoError::NoSection("strtab"))?;

    // Resolve the link after the scan: a string table may precede .dynsym.
    let mut strtab_off: Option<usize> = None;
    let mut strtab_size: Option<usize> = None;
    if dynsym_link < e_shnum {
        let off = shdr_off(e_shoff, e_shentsize, dynsym_link).ok_or(DsoError::Truncated("shdr"))?;
        let sh_type = rd_u32(bytes, off + 4).ok_or(DsoError::Truncated("sh_type"))?;
        if sh_type == SHT_STRTAB {
            strtab_off = Some(rd_u32(bytes, off + 16).ok_or(DsoError::Truncated("sh_offset"))? as usize);
            strtab_size = Some(rd_u32(bytes, off + 20).ok_or(DsoError::Truncated("sh_size"))? as usize);
        }
    }
    let strtab_off = strtab_off.ok_or(DsoError::NoSection("strtab"))?;
    let strtab_size = strtab_size.ok_or(DsoError::NoSection("strtab"))?;
    let strtab_end = strtab_off.checked_add(strtab_size).ok_or(DsoError::Truncated("strtab name"))?;

    // The DLL a symbol belongs to is NOT in its name: it is the version definition its .version index names
    // (index 2 = `euser{000a0000}[100039e5].dll`, the exact name an E32 image imports by). Verdef entries:
    // vd_version u16, vd_flags u16, vd_ndx u16, vd_cnt u16, vd_hash u32, vd_aux u32, vd_next u32; the first aux
    // is vda_name u32 (an offset in the verdef's linked string table), vda_next u32.
    let (vd_off, vd_size, vd_link) = verdef.ok_or(DsoError::NoSection(".version_d"))?;
    let versym = versym.ok_or(DsoError::NoSection(".version"))?;
    let vd_str = {
        let off = shdr_off(e_shoff, e_shentsize, vd_link).ok_or(DsoError::Truncated("shdr"))?;
        rd_u32(bytes, off + 16).ok_or(DsoError::Truncated("sh_offset"))? as usize
    };
    let mut versions: Vec<(u16, String)> = Vec::new();
    let mut at = vd_off;
    let vd_end = vd_off.checked_add(vd_size).ok_or(DsoError::Truncated(".version_d"))?;
    while at < vd_end {
        let ndx = rd_u16(bytes, at + 4).ok_or(DsoError::Truncated(".version_d"))?;
        let aux = rd_u32(bytes, at + 12).ok_or(DsoError::Truncated(".version_d"))? as usize;
        let next = rd_u32(bytes, at + 16).ok_or(DsoError::Truncated(".version_d"))? as usize;
        let name_off = rd_u32(bytes, at.checked_add(aux).ok_or(DsoError::Truncated(".version_d"))?).ok_or(DsoError::Truncated(".version_d"))? as usize;
        let start = vd_str.checked_add(name_off).ok_or(DsoError::Truncated(".version_d name"))?;
        let len = bytes.get(start..).and_then(|t| t.iter().position(|&b| b == 0)).ok_or(DsoError::Truncated(".version_d name"))?;
        let name = core::str::from_utf8(&bytes[start..start + len]).map_err(|_| DsoError::Truncated(".version_d name"))?;
        versions.push((ndx, name.to_string()));
        if next == 0 { break; }
        at = at.checked_add(next).ok_or(DsoError::Truncated(".version_d"))?;
    }

    // 3. Iterate .dynsym symbols
    let mut exports = Vec::new();
    let n_syms = dynsym_size / dynsym_entsize;
    for i in 0..n_syms {
        let sym_off = i
            .checked_mul(dynsym_entsize)
            .and_then(|n| dynsym_off.checked_add(n))
            .ok_or(DsoError::Truncated("sym"))?;
        let st_name = rd_u32(bytes, sym_off).ok_or(DsoError::Truncated("st_name"))? as usize;
        let st_value = rd_u32(bytes, sym_off + 4).ok_or(DsoError::Truncated("st_value"))?;
        let st_info = bytes.get(sym_off + 12).copied().ok_or(DsoError::Truncated("st_info"))?;
        let st_shndx = rd_u16(bytes, sym_off + 14).ok_or(DsoError::Truncated("st_shndx"))? as usize;

        let st_bind = st_info >> 4;
        let st_type = st_info & 0xf;

        // Keep GLOBAL FUNC or OBJECT defined in ER_RO
        if st_bind != STB_GLOBAL {
            continue;
        }
        if st_type != STT_FUNC && st_type != STT_OBJECT {
            continue;
        }
        if st_shndx != er_ro_idx {
            continue;
        }

        // Read symbol name from strtab
        if st_name >= strtab_size {
            return Err(DsoError::Truncated("strtab name"));
        }
        let name_start = strtab_off.checked_add(st_name).ok_or(DsoError::Truncated("strtab name"))?;
        let mut name_end = name_start;
        while name_end < strtab_end && bytes.get(name_end).copied() != Some(0) {
            name_end += 1;
        }
        let name_bytes = bytes.get(name_start..name_end).ok_or(DsoError::Truncated("strtab name"))?;
        let name = core::str::from_utf8(name_bytes)
            .map_err(|_| DsoError::Truncated("strtab utf8"))?
            .to_string();

        let vix = rd_u16(bytes, versym.checked_add(i * 2).ok_or(DsoError::Truncated(".version"))?).ok_or(DsoError::Truncated(".version"))? & 0x7fff;
        let Some((_, dll)) = versions.iter().find(|(n, _)| *n == vix) else { continue };
        let (sym_name, dll) = (name, dll.clone());

        // The ordinal is the u32 in ER_RO's bytes at st_value (ER_RO's FILE offset + st_value).
        let er_ro_off = er_ro_file_off.checked_add(st_value as usize).ok_or(DsoError::Truncated("ER_RO ordinal"))?;
        let ordinal_u32 = rd_u32(bytes, er_ro_off).ok_or(DsoError::Truncated("ER_RO ordinal"))?;
        if ordinal_u32 == 0 || ordinal_u32 > 0xFFFF {
            return Err(DsoError::BadOrdinal { name: sym_name.clone() });
        }
        let ordinal = ordinal_u32 as u16;

        exports.push(DsoExport { dll, ordinal, name: sym_name });
    }

    Ok(exports)
}

#[derive(Debug)]
pub struct OrdinalTable {
    map: BTreeMap<(String, u16), String>,
}

fn ascii_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

impl OrdinalTable {
    pub fn parse(tsv: &str) -> Result<OrdinalTable, (usize, &'static str)> {
        let mut map = BTreeMap::new();
        for (i, line) in tsv.lines().enumerate() {
            let lineno = i + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 3 {
                return Err((lineno, "expected 3 fields"));
            }
            let dll = fields[0].to_string();
            let ordinal: u16 = fields[1].parse().map_err(|_| (lineno, "bad ordinal"))?;
            let name = fields[2].to_string();
            let key = (ascii_lower(&dll), ordinal);
            map.insert(key, name);
        }
        Ok(OrdinalTable { map })
    }

    pub fn name_of(&self, dll: &str, ordinal: u16) -> Option<&str> {
        let key = (ascii_lower(dll), ordinal);
        self.map.get(&key).map(|s| s.as_str())
    }
}