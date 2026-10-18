//! The loader section (Mac OS Runtime Architectures, "Loader Section").
//!
//! ```text
//! loader info header (56): i32 mainSection u32 mainOffset i32 initSection u32 initOffset i32 termSection u32 termOffset
//!   u32 importedLibraryCount u32 totalImportedSymbolCount u32 relocSectionCount u32 relocInstrOffset
//!   u32 loaderStringsOffset u32 exportHashOffset u32 exportHashTablePower u32 exportedSymbolCount
//! imported library (24):  u32 nameOffset u32 oldImpVersion u32 currentVersion u32 importedSymbolCount
//!                         u32 firstImportedSymbol u8 options(0x40 weak, 0x80 init-before) u8 reserved u16 reserved
//! imported symbol (4):    u8 flags:4|class:4 (flag 0x80 = weak), u24 nameOffset
//! relocation header (12): u16 sectionIndex u16 reserved u32 relocCount u32 firstRelocOffset   (offset in bytes into
//!                         the relocation instructions)
//! ```
//! Order in the section: header, libraries, symbols, relocation headers, relocation instructions (u16 each), strings,
//! then the export hash table = `1 << power` slot words (chainCount:14 | firstKeyIndex:18), `exportedSymbolCount`
//! key words (hash words), `exportedSymbolCount` 10-byte symbols (u32 class:8|nameOffset:24, u32 value, i16 section).
//! All offsets in the info header are from the start of the loader section.
use crate::hash::{hash_word, slot};
use crate::{be16, be32, PefError};
use alloc::string::String;
use alloc::vec::Vec;

pub const INFO_SIZE: usize = 56;
pub const LIBRARY_SIZE: usize = 24;
pub const SYMBOL_SIZE: usize = 4;
pub const RELOC_HEADER_SIZE: usize = 12;
pub const EXPORT_SYMBOL_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolClass { Code, Data, TVector, Toc, Glue }

impl SymbolClass {
    pub fn from_u8(v: u8) -> Result<SymbolClass, PefError> {
        Ok(match v { 0 => Self::Code, 1 => Self::Data, 2 => Self::TVector, 3 => Self::Toc, 4 => Self::Glue, c => return Err(PefError::BadSymbolClass(c)) })
    }
    pub fn name(self) -> &'static str { match self { Self::Code => "code", Self::Data => "data", Self::TVector => "tvector", Self::Toc => "toc", Self::Glue => "glue" } }
}

/// A main / init / term entry: the TVector lives at `offset` in `section`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryPoint { pub section: u32, pub offset: u32 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedLibrary {
    pub name: String,
    pub old_imp_version: u32,
    pub current_version: u32,
    pub first_symbol: u32,
    pub symbol_count: u32,
    /// The whole library may be absent (kPEFWeakImportLibMask, 0x40).
    pub weak: bool,
    pub init_before: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSymbol {
    pub name: String,
    pub class: SymbolClass,
    /// A weak symbol may be unresolved: it relocates to zero instead of failing the load.
    pub weak: bool,
    /// Index into [`Loader::libraries`].
    pub library: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportTarget { Section { index: u16, offset: u32 }, Absolute(u32), Reexport { import: u32 } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedSymbol { pub name: String, pub class: SymbolClass, pub target: ExportTarget, pub hash_word: u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocHeader { pub section: u16, pub count: u32, pub first: u32 }

#[derive(Debug, Clone)]
pub struct Loader {
    pub main: Option<EntryPoint>,
    pub init: Option<EntryPoint>,
    pub term: Option<EntryPoint>,
    pub libraries: Vec<ImportedLibrary>,
    pub imports: Vec<ImportedSymbol>,
    pub reloc_headers: Vec<RelocHeader>,
    /// The relocation instruction words, big-endian already decoded.
    pub reloc_words: Vec<u16>,
    pub hash_power: u32,
    slots: Vec<u32>,
    pub exports: Vec<ExportedSymbol>,
}

fn bad(w: &'static str) -> PefError { PefError::BadLoader(w) }

/// A NUL-terminated name at `at` in the string table.
fn cstr(strings: &[u8], at: usize, what: &'static str) -> Result<String, PefError> {
    let rest = strings.get(at..).ok_or(bad(what))?;
    let end = rest.iter().position(|&c| c == 0).ok_or(bad(what))?;
    Ok(rest[..end].iter().map(|&c| char::from(c)).collect())
}

fn entry(section: i32, offset: u32, sections: usize, what: &'static str) -> Result<Option<EntryPoint>, PefError> {
    if section < 0 { return Ok(None); }
    if section as usize >= sections { return Err(bad(what)); }
    Ok(Some(EntryPoint { section: section as u32, offset }))
}

impl Loader {
    /// Parse the loader section `d`; `sections` is the container's section count (indices must be below it).
    pub fn parse(d: &[u8], sections: usize) -> Result<Loader, PefError> {
        let w = |i: usize| be32(d, i * 4).ok_or(PefError::Truncated("loader info header"));
        if d.len() < INFO_SIZE { return Err(PefError::Truncated("loader info header")); }
        let (main_s, main_o, init_s, init_o, term_s, term_o) = (w(0)? as i32, w(1)?, w(2)? as i32, w(3)?, w(4)? as i32, w(5)?);
        let (nlib, nsym, nrel) = (w(6)? as usize, w(7)? as usize, w(8)? as usize);
        let (reloc_off, str_off, hash_off, power, nexp) = (w(9)? as usize, w(10)? as usize, w(11)? as usize, w(12)?, w(13)? as usize);
        let main = entry(main_s, main_o, sections, "main entry names a section that does not exist")?;
        let init = entry(init_s, init_o, sections, "init entry names a section that does not exist")?;
        let term = entry(term_s, term_o, sections, "term entry names a section that does not exist")?;
        if str_off > d.len() { return Err(bad("loaderStringsOffset is past the end of the section")); }
        let strings = &d[str_off..];
        let sym_at = nlib.checked_mul(LIBRARY_SIZE).and_then(|n| n.checked_add(INFO_SIZE)).ok_or(bad("library count overflows"))?;
        let rh_at = nsym.checked_mul(SYMBOL_SIZE).and_then(|n| n.checked_add(sym_at)).ok_or(bad("symbol count overflows"))?;
        let rh_end = nrel.checked_mul(RELOC_HEADER_SIZE).and_then(|n| n.checked_add(rh_at)).ok_or(bad("relocation header count overflows"))?;
        if rh_end > reloc_off || reloc_off > str_off || str_off > d.len() { return Err(bad("tables overlap or run past the section: library, symbol and relocation-header tables must precede the relocation instructions, then the strings")); }

        let mut libraries = Vec::with_capacity(nlib);
        let mut next_first = 0u32;
        for i in 0..nlib {
            let b = &d[INFO_SIZE + i * LIBRARY_SIZE..INFO_SIZE + (i + 1) * LIBRARY_SIZE];
            let lib = ImportedLibrary {
                name: cstr(strings, be32(b, 0).unwrap() as usize, "library name offset is outside the string table or unterminated")?,
                old_imp_version: be32(b, 4).unwrap(), current_version: be32(b, 8).unwrap(), symbol_count: be32(b, 12).unwrap(),
                first_symbol: be32(b, 16).unwrap(), weak: b[20] & 0x40 != 0, init_before: b[20] & 0x80 != 0,
            };
            if lib.first_symbol != next_first || u64::from(lib.first_symbol) + u64::from(lib.symbol_count) > nsym as u64 {
                return Err(bad("an imported library's symbol range is not the next run of the symbol table"));
            }
            next_first = lib.first_symbol + lib.symbol_count;
            libraries.push(lib);
        }
        if next_first as usize != nsym { return Err(bad("totalImportedSymbolCount is not covered by the libraries")); }
        let mut imports = Vec::with_capacity(nsym);
        for (li, lib) in libraries.iter().enumerate() {
            for s in lib.first_symbol..lib.first_symbol + lib.symbol_count {
                let at = sym_at + s as usize * SYMBOL_SIZE;
                let v = be32(d, at).unwrap();
                imports.push(ImportedSymbol {
                    name: cstr(strings, (v & 0x00FF_FFFF) as usize, "imported symbol name is outside the string table or unterminated")?,
                    class: SymbolClass::from_u8(((v >> 24) & 0x0F) as u8)?, weak: v & 0x8000_0000 != 0, library: li,
                });
            }
        }
        let mut reloc_headers = Vec::with_capacity(nrel);
        for i in 0..nrel {
            let at = rh_at + i * RELOC_HEADER_SIZE;
            let h = RelocHeader { section: be16(d, at).unwrap(), count: be32(d, at + 4).unwrap(), first: be32(d, at + 8).unwrap() };
            if usize::from(h.section) >= sections { return Err(bad("a relocation header names a section that does not exist")); }
            reloc_headers.push(h);
        }
        let reloc_words: Vec<u16> = d[reloc_off..str_off].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        for h in &reloc_headers {
            let first = h.first as usize;
            if first % 2 != 0 || first / 2 + h.count as usize > reloc_words.len() { return Err(bad("a relocation header's instructions run past the relocation area")); }
        }

        // export hash table
        let nslots = 1usize.checked_shl(power).filter(|&n| n <= 1 << 24).ok_or(bad("exportHashTablePower is unreasonably large"))?;
        let has_exports = hash_off != 0 || nexp > 0;
        if has_exports {
            let end = nslots.checked_mul(4).and_then(|n| n.checked_add(nexp.checked_mul(4 + EXPORT_SYMBOL_SIZE)?)).and_then(|n| n.checked_add(hash_off));
            if end.map_or(true, |e| e > d.len()) { return Err(bad("the export hash table runs past the section")); }
        }
        let (mut slots, mut exports) = (Vec::new(), Vec::new());
        if has_exports {
            for i in 0..nslots { slots.push(be32(d, hash_off + i * 4).unwrap()); }
            let keys_at = hash_off + nslots * 4;
            let syms_at = keys_at + nexp * 4;
            for i in 0..nexp {
                let key = be32(d, keys_at + i * 4).unwrap();
                let s = &d[syms_at + i * EXPORT_SYMBOL_SIZE..syms_at + (i + 1) * EXPORT_SYMBOL_SIZE];
                let cn = be32(s, 0).unwrap();
                let (class, off, len) = (SymbolClass::from_u8(((cn >> 24) & 0x0F) as u8)?, (cn & 0x00FF_FFFF) as usize, (key >> 16) as usize);
                let nb = strings.get(off..off + len).ok_or(bad("an exported symbol's name is outside the string table"))?;
                let (value, section) = (be32(s, 4).unwrap(), be16(s, 8).unwrap() as i16);
                let target = match section {
                    -1 => ExportTarget::Absolute(value),
                    -2 => ExportTarget::Reexport { import: value },
                    n if n >= 0 && (n as usize) < sections => ExportTarget::Section { index: n as u16, offset: value },
                    _ => return Err(bad("an exported symbol names a section that does not exist")),
                };
                exports.push(ExportedSymbol { name: nb.iter().map(|&c| char::from(c)).collect(), class, target, hash_word: key });
            }
            for &sl in &slots {
                let (count, first) = ((sl >> 18) as usize, (sl & 0x3_FFFF) as usize);
                if first + count > nexp { return Err(bad("an export hash chain runs past the export key table")); }
            }
        }
        Ok(Loader { main, init, term, libraries, imports, reloc_headers, reloc_words, hash_power: power, slots, exports })
    }

    /// Look an exported symbol up THROUGH the hash table, as the Code Fragment Manager does.
    pub fn find_export(&self, name: &str) -> Option<&ExportedSymbol> {
        if self.slots.is_empty() { return None; }
        let word = hash_word(name.as_bytes());
        let sl = *self.slots.get(slot(word, self.hash_power) as usize)?;
        let (count, first) = ((sl >> 18) as usize, (sl & 0x3_FFFF) as usize);
        self.exports.get(first..first + count)?.iter().find(|e| e.hash_word == word && e.name == name)
    }

    /// Exported symbols whose stored hash word disagrees with the name (a corrupt table; a census can report it).
    pub fn hash_mismatches(&self) -> Vec<&ExportedSymbol> {
        self.exports.iter().filter(|e| e.hash_word != hash_word(e.name.as_bytes())).collect()
    }
}
