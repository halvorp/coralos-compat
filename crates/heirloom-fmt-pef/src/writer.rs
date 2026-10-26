//! The inverse of the parser: build a PEF container. Used by tests and tools (a fixture builder, the macppc engine's
//! tests); the output is a well-formed fragment, not an optimised one.
use crate::hash::{hash_word, slot, table_power};
use crate::header::{HEADER_SIZE, SECTION_HEADER_SIZE};
use crate::loader::SymbolClass;
use crate::SectionKind;
use alloc::string::String;
use alloc::vec::Vec;

struct Sec { kind: SectionKind, total: u32, unpacked: u32, data: Vec<u8>, align: u8 }
struct Lib { name: String, weak: bool, syms: Vec<(String, SymbolClass, bool)> }
struct Export { name: String, class: SymbolClass, section: i16, value: u32 }

#[derive(Default)]
pub struct PefBuilder {
    sections: Vec<Sec>,
    libs: Vec<Lib>,
    relocs: Vec<(u16, Vec<u16>)>,
    exports: Vec<Export>,
    main: Option<(i32, u32)>,
    init: Option<(i32, u32)>,
    term: Option<(i32, u32)>,
}

use heirloom_common::bytes::{put_be16 as push16, put_be32 as push32};

impl PefBuilder {
    pub fn new() -> Self { Self::default() }
    /// Add a code section; returns its section index.
    pub fn code(&mut self, bytes: &[u8]) -> usize { self.push(SectionKind::Code, bytes.len() as u32, bytes.to_vec(), bytes.len() as u32) }
    /// Add an unpacked data section of `total` bytes whose first bytes are `bytes`.
    pub fn data(&mut self, bytes: &[u8], total: u32) -> usize { self.push(SectionKind::UnpackedData, total, bytes.to_vec(), bytes.len() as u32) }
    /// Add a pattern-initialised data section: `pidata` expands to `unpacked` bytes inside `total`.
    pub fn pattern_data(&mut self, pidata: &[u8], unpacked: u32, total: u32) -> usize { self.push(SectionKind::PatternData, total, pidata.to_vec(), unpacked) }
    fn push(&mut self, kind: SectionKind, total: u32, data: Vec<u8>, unpacked: u32) -> usize {
        self.sections.push(Sec { kind, total, unpacked, data, align: 2 });
        self.sections.len() - 1
    }
    /// Set a section's alignment exponent (2^exp bytes).
    pub fn align(&mut self, section: usize, exp: u8) -> &mut Self { self.sections[section].align = exp; self }
    /// Add an imported library and its symbols `(name, class, weak)`.
    pub fn import_library(&mut self, name: &str, weak: bool, syms: &[(&str, SymbolClass, bool)]) -> &mut Self {
        self.libs.push(Lib { name: name.into(), weak, syms: syms.iter().map(|&(n, c, w)| (n.into(), c, w)).collect() });
        self
    }
    /// A relocation header for `section` with raw instruction words.
    pub fn reloc(&mut self, section: u16, words: &[u16]) -> &mut Self { self.relocs.push((section, words.to_vec())); self }
    pub fn export(&mut self, name: &str, class: SymbolClass, section: i16, value: u32) -> &mut Self {
        self.exports.push(Export { name: name.into(), class, section, value });
        self
    }
    pub fn main(&mut self, section: i32, offset: u32) -> &mut Self { self.main = Some((section, offset)); self }
    pub fn init(&mut self, section: i32, offset: u32) -> &mut Self { self.init = Some((section, offset)); self }
    pub fn term(&mut self, section: i32, offset: u32) -> &mut Self { self.term = Some((section, offset)); self }

    fn loader_section(&self) -> Vec<u8> {
        let mut strings: Vec<u8> = Vec::new();
        let mut name_at = |s: &str, nul: bool| { let at = strings.len() as u32; strings.extend_from_slice(s.as_bytes()); if nul { strings.push(0); } at };
        let nsym: usize = self.libs.iter().map(|l| l.syms.len()).sum();
        let (mut libs, mut syms, mut first) = (Vec::new(), Vec::new(), 0u32);
        for l in &self.libs {
            push32(&mut libs, name_at(&l.name, true));
            push32(&mut libs, 0); push32(&mut libs, 0);
            push32(&mut libs, l.syms.len() as u32); push32(&mut libs, first);
            libs.extend_from_slice(&[if l.weak { 0x40 } else { 0 }, 0, 0, 0]);
            first += l.syms.len() as u32;
            for (n, c, w) in &l.syms {
                let at = name_at(n, true);
                push32(&mut syms, (u32::from(if *w { 0x80u8 } else { 0 } | *c as u8) << 24) | at);
            }
        }
        let (mut rhs, mut instrs) = (Vec::new(), Vec::new());
        for (sec, words) in &self.relocs {
            push16(&mut rhs, *sec); push16(&mut rhs, 0); push32(&mut rhs, words.len() as u32); push32(&mut rhs, instrs.len() as u32);
            for w in words { push16(&mut instrs, *w); }
        }
        // exports: sorted by hash slot so every chain is contiguous
        let power = table_power(self.exports.len());
        let mut ex: Vec<(u32, u32, &Export)> = self.exports.iter().map(|e| { let w = hash_word(e.name.as_bytes()); (slot(w, power), w, e) }).collect();
        ex.sort_by_key(|&(s, _, _)| s);
        let mut slots = alloc::vec![0u32; 1usize << power];
        for (i, &(s, _, _)) in ex.iter().enumerate() {
            let e = &mut slots[s as usize];
            *e = if *e == 0 { (1 << 18) | i as u32 } else { *e + (1 << 18) };
        }
        let (mut keys, mut esyms) = (Vec::new(), Vec::new());
        for &(_, w, e) in &ex {
            push32(&mut keys, w);
            push32(&mut esyms, (u32::from(e.class as u8) << 24) | name_at(&e.name, false));
            push32(&mut esyms, e.value); push16(&mut esyms, e.section as u16);
        }
        let relocs_at = crate::loader::INFO_SIZE + libs.len() + syms.len() + rhs.len();
        let strings_at = relocs_at + instrs.len();
        let hash_at = strings_at + strings.len();
        let hash_at = hash_at.next_multiple_of(4);
        let ent = |e: Option<(i32, u32)>| e.unwrap_or((-1, 0));
        let mut out = Vec::new();
        for (s, o) in [ent(self.main), ent(self.init), ent(self.term)] { push32(&mut out, s as u32); push32(&mut out, o); }
        for v in [self.libs.len(), nsym, self.relocs.len(), relocs_at, strings_at, hash_at] { push32(&mut out, v as u32); }
        push32(&mut out, power); push32(&mut out, self.exports.len() as u32);
        out.extend_from_slice(&libs); out.extend_from_slice(&syms); out.extend_from_slice(&rhs); out.extend_from_slice(&instrs); out.extend_from_slice(&strings);
        out.resize(hash_at, 0);
        for s in slots { push32(&mut out, s); }
        out.extend_from_slice(&keys); out.extend_from_slice(&esyms);
        out
    }

    /// The container bytes. The loader section is appended last.
    pub fn build(&self) -> Vec<u8> {
        let loader = self.loader_section();
        let n = self.sections.len() + 1;
        let off = (HEADER_SIZE + n * SECTION_HEADER_SIZE).next_multiple_of(16);
        let mut out = Vec::new();
        out.extend_from_slice(b"Joy!peffpwpc");
        for v in [1u32, 0, 0, 0, 0] { push32(&mut out, v); }
        push16(&mut out, n as u16); push16(&mut out, self.sections.len() as u16); push32(&mut out, 0);
        let mut body: Vec<u8> = Vec::new();
        let all = self.sections.iter().map(|s| (s.kind, s.total, s.unpacked, &s.data[..], s.align))
            .chain(core::iter::once((SectionKind::Loader, loader.len() as u32, loader.len() as u32, &loader[..], 2u8)));
        for (kind, total, unpacked, data, align) in all {
            push32(&mut out, u32::MAX); push32(&mut out, 0); push32(&mut out, total); push32(&mut out, unpacked);
            push32(&mut out, data.len() as u32); push32(&mut out, (off + body.len()) as u32);
            out.extend_from_slice(&[kind.to_u8(), 4, align, 0]);
            body.extend_from_slice(data);
            body.resize(body.len().next_multiple_of(4), 0);
        }
        out.resize(off, 0);
        out.extend_from_slice(&body);
        out
    }
}

/// Encode a pattern-data variable-length count (7 bits per byte, high bit = more).
pub fn varint(mut v: u32) -> Vec<u8> {
    let mut parts = alloc::vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 { parts.push((v & 0x7F) as u8 | 0x80); v >>= 7; }
    parts.reverse();
    parts
}
