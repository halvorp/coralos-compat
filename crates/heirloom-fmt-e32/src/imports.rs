//! The import section (ELF-derived format): `iSize`, then one block per referenced DLL - the DLL name's offset in
//! the section, the import count, and that many code-section offsets. The code word at each offset holds the
//! export ordinal in its low half and an addend in its high half; the loader replaces it with the export's address
//! plus the addend.
use alloc::{string::String, vec::Vec};

use crate::{u32_at, E32Error, E32Header};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedDll {
    /// As the image names it, e.g. `euser{000a0000}[100039e5].dll`: base name, `{version}`, `[uid3]`.
    pub name: String,
    pub imports: Vec<Import>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Import {
    /// Offset of the bound word in the code section.
    pub site: u32,
    pub ordinal: u16,
    pub addend: u16,
}

pub(crate) fn parse(file: &[u8], h: &E32Header, code: &[u8]) -> Result<Vec<ImportedDll>, E32Error> {
    if h.dll_ref_count == 0 {
        return Ok(Vec::new());
    }
    let base = h.import_offset as usize;
    let size = u32_at(file, base, "import section")? as usize;
    let section = base
        .checked_add(size)
        .and_then(|end| file.get(base..end))
        .ok_or(E32Error::Truncated("import section"))?;
    let mut at = 4usize;
    let mut dlls = Vec::new();
    for _ in 0..h.dll_ref_count {
        let name_off = u32_at(section, at, "import block")? as usize;
        let count = u32_at(section, at + 4, "import block")? as usize;
        at += 8;
        let tail = section.get(name_off..).ok_or(E32Error::Truncated("import DLL name"))?;
        let len = tail.iter().position(|&b| b == 0).ok_or(E32Error::Truncated("import DLL name"))?;
        let name =
            core::str::from_utf8(&tail[..len]).map_err(|_| E32Error::Malformed("import DLL name is not UTF-8"))?;
        let mut imports = Vec::new();
        for _ in 0..count {
            let site = u32_at(section, at, "import entries")?;
            at += 4;
            let word = u32_at(code, site as usize, "import site in code")?;
            imports.push(Import { site, ordinal: word as u16, addend: (word >> 16) as u16 });
        }
        dlls.push(ImportedDll { name: String::from(name), imports });
    }
    Ok(dlls)
}
