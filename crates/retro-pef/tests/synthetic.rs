//! Container, pattern data and relocation behaviour on fragments built by our own writer, each with a negative control.
use retro_pef::loader::{ExportTarget, SymbolClass};
use retro_pef::pidata::unpack;
use retro_pef::reloc::{decode, apply, RelocOp};
use retro_pef::writer::{varint, PefBuilder};
use retro_pef::{Pef, PefError, SectionKind};

fn sample() -> Vec<u8> {
    let mut b = PefBuilder::new();
    let code = b.code(&[0x4E, 0x80, 0x00, 0x20, 0, 0, 0, 0]);
    let data = b.data(&[0, 0, 0, 8, 0, 0, 0, 0, 0xAA, 0xBB], 64);
    b.import_library("InterfaceLib", false, &[("NewHandle", SymbolClass::TVector, false), ("qd", SymbolClass::Data, false), ("SomeNewerCall", SymbolClass::TVector, true)]);
    b.import_library("MathLib", true, &[("sin", SymbolClass::TVector, false)]);
    b.export("main", SymbolClass::TVector, data as i16, 0).export("helper", SymbolClass::Code, code as i16, 4);
    b.main(data as i32, 0).init(-1, 0);
    b.reloc(data as u16, &[0x4A00 | 3]); // ImportRun, 4 words
    b.build()
}

#[test]
fn a_built_fragment_parses_back_with_everything_in_it() {
    let bytes = sample();
    let pef = Pef::parse(&bytes).unwrap();
    assert_eq!((pef.header.section_count, pef.header.inst_section_count), (3, 2));
    assert_eq!(pef.sections[2].kind, SectionKind::Loader);
    let l = pef.loader().unwrap();
    assert_eq!(l.libraries.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["InterfaceLib", "MathLib"]);
    assert!(l.libraries[1].weak && !l.libraries[0].weak);
    assert_eq!(l.imports.iter().map(|i| (i.name.as_str(), i.library, i.class, i.weak)).collect::<Vec<_>>(), vec![
        ("NewHandle", 0, SymbolClass::TVector, false), ("qd", 0, SymbolClass::Data, false), ("SomeNewerCall", 0, SymbolClass::TVector, true), ("sin", 1, SymbolClass::TVector, false)]);
    assert_eq!(l.main.map(|e| (e.section, e.offset)), Some((1, 0)));
    assert!(l.init.is_none() && l.term.is_none());
    let e = l.find_export("helper").unwrap();
    assert_eq!(e.target, ExportTarget::Section { index: 0, offset: 4 });
    assert!(l.find_export("absent").is_none());
    let img = pef.section_image(1).unwrap();
    assert_eq!((img.len(), img[9], img[10]), (64, 0xBB, 0));
}

#[test]
fn bad_magic_architecture_version_and_truncations_are_named() {
    let good = sample();
    let mut m = good.clone(); m[0] = b'X';
    assert_eq!(Pef::parse(&m).unwrap_err(), PefError::BadMagic);
    let mut m = good.clone(); m[8..12].copy_from_slice(b"m68k");
    assert_eq!(Pef::parse(&m).unwrap_err(), PefError::UnsupportedArchitecture(*b"m68k"));
    let mut m = good.clone(); m[15] = 2;
    assert_eq!(Pef::parse(&m).unwrap_err(), PefError::UnsupportedVersion(2));
    assert!(matches!(Pef::parse(&good[..20]).unwrap_err(), PefError::Truncated(_)));
    assert!(matches!(Pef::parse(&good[..50]).unwrap_err(), PefError::Truncated(_) | PefError::BadSection { .. }));
    // a section whose bytes run past the file
    let mut m = good.clone(); m[40 + 16..40 + 20].copy_from_slice(&0x7FFF_0000u32.to_be_bytes());
    assert!(matches!(Pef::parse(&m).unwrap_err(), PefError::BadSection { index: 0, .. }));
    // an unknown section kind
    let mut m = good.clone(); m[40 + 24] = 99;
    assert!(matches!(Pef::parse(&m).unwrap_err(), PefError::BadSection { index: 0, why: "unknown section kind" }));
    // negative control: the unmodified file is fine
    assert!(Pef::parse(&good).is_ok());
}

#[test]
fn a_damaged_loader_section_is_refused_by_name() {
    let good = sample();
    let pef = Pef::parse(&good).unwrap();
    let at = pef.sections[2].container_offset as usize;
    // totalImportedSymbolCount no longer covered by the libraries
    let mut m = good.clone(); m[at + 28..at + 32].copy_from_slice(&9u32.to_be_bytes());
    assert!(matches!(Pef::parse(&m).unwrap().loader().unwrap_err(), PefError::BadLoader(_)));
    // main entry names a section that does not exist
    let mut m = good.clone(); m[at..at + 4].copy_from_slice(&40u32.to_be_bytes());
    assert!(matches!(Pef::parse(&m).unwrap().loader().unwrap_err(), PefError::BadLoader(w) if w.contains("main")));
    // an imported symbol with an undefined class
    let sym0 = at + 56 + 2 * 24;
    let mut m = good.clone(); m[sym0] = (m[sym0] & 0xF0) | 7;
    assert_eq!(Pef::parse(&m).unwrap().loader().unwrap_err(), PefError::BadSymbolClass(7));
    // a loader section that is cut short
    assert!(Pef::parse(&good).unwrap().loader().is_ok());
    let mut b = PefBuilder::new();
    b.code(&[0; 4]);
    let mut nol = b.build();
    nol[32..34].copy_from_slice(&1u16.to_be_bytes()); // sectionCount 1: the loader section is gone
    assert!(matches!(Pef::parse(&nol).unwrap().loader().unwrap_err(), PefError::BadLoader(w) if w.contains("no loader")));
}

#[test]
fn pattern_data_every_opcode_and_its_refusals() {
    let mut s = Vec::new();
    s.push(0x00 | 0); s.extend(varint(3));                       // zero 3
    s.push(0x20 | 2); s.extend([0xAB, 0xCD]);                    // block of 2
    s.push(0x40 | 2); s.extend(varint(2)); s.extend([0x11, 0x22]); // repeat the 2-byte block 3 times
    s.push(0x60 | 1); s.extend(varint(1)); s.extend(varint(2)); s.push(0x7E); s.extend([0x01, 0x02]); // repeat-block common=7E custom=1 x2
    s.push(0x80 | 1); s.extend(varint(1)); s.extend(varint(1)); s.push(0x05); // repeat-zero common=1 zero, custom 1, x1
    let want = [0, 0, 0, 0xAB, 0xCD, 0x11, 0x22, 0x11, 0x22, 0x11, 0x22, 0x7E, 1, 0x7E, 2, 0x7E, 0, 5, 0];
    let mut out = vec![0xFF; want.len()];
    unpack(&s, &mut out).unwrap();
    assert_eq!(out, want);
    // a count of 0 in the instruction byte means "varint follows" (a 40-byte zero run)
    let mut big = vec![0xFF; 40];
    unpack(&[0x00, 40], &mut big).unwrap();
    assert!(big.iter().all(|&b| b == 0));
    // negative controls
    assert!(matches!(unpack(&s, &mut vec![0; want.len() - 1]), Err(PefError::BadPidata(w)) if w.contains("past")));
    assert!(matches!(unpack(&s, &mut vec![0; want.len() + 1]), Err(PefError::BadPidata(w)) if w.contains("ends before")));
    assert!(matches!(unpack(&[0x20 | 5, 1, 2], &mut [0; 5]), Err(PefError::BadPidata(w)) if w.contains("literal")));
    assert!(matches!(unpack(&[0xA0 | 1, 0], &mut [0; 1]), Err(PefError::BadPidata(w)) if w.contains("not defined")));
    assert!(matches!(unpack(&[0x00, 0x80], &mut [0; 1]), Err(PefError::BadPidata(w)) if w.contains("variable-length")));
    assert!(matches!(unpack(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F], &mut [0; 1]), Err(PefError::BadPidata(w)) if w.contains("overflows")));
}

#[test]
fn a_pattern_data_section_is_expanded_into_its_image() {
    let mut b = PefBuilder::new();
    b.code(&[0; 4]);
    b.pattern_data(&[0x20 | 3, 1, 2, 3, 0x00 | 2], 5, 12);
    let bytes = b.build();
    let pef = Pef::parse(&bytes).unwrap();
    assert_eq!(pef.section_image(1).unwrap(), [1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // negative control: unpacked size and stream disagree
    let mut b = PefBuilder::new();
    b.code(&[0; 4]);
    b.pattern_data(&[0x20 | 3, 1, 2, 3], 5, 12);
    assert!(matches!(Pef::parse(&b.build()).unwrap().section_image(1), Err(PefError::BadPidata(_))));
}

fn run(words: &[u16], bases: &[Option<u32>], imports: &[u32]) -> Result<Vec<(u16, u32, u32)>, PefError> {
    let ops = decode(words, 0, words.len())?;
    let mut seen = Vec::new();
    apply(&ops, 1, bases, imports, &mut |s, o, a| { seen.push((s, o, a)); Ok(()) })?;
    Ok(seen)
}
const BASES: [Option<u32>; 3] = [Some(0x1000), Some(0x2000), Some(0x3000)];

#[test]
fn every_relocation_opcode_does_what_the_format_says() {
    // BySectDWithSkip: skip 2 words, relocate 3 by D
    assert_eq!(run(&[(2 << 6) | 3], &BASES, &[]).unwrap(), [(1, 8, 0x2000), (1, 12, 0x2000), (1, 16, 0x2000)]);
    // BySectC run of 2; BySectD run of 1 after it
    assert_eq!(run(&[0x4000 | 1, 0x4200], &BASES, &[]).unwrap(), [(1, 0, 0x1000), (1, 4, 0x1000), (1, 8, 0x2000)]);
    // TVector12 x2: words 0 and 1 relocated, word 2 skipped
    assert_eq!(run(&[0x4400 | 1], &BASES, &[]).unwrap(), [(1, 0, 0x1000), (1, 4, 0x2000), (1, 12, 0x1000), (1, 16, 0x2000)]);
    // TVector8 x2
    assert_eq!(run(&[0x4600 | 1], &BASES, &[]).unwrap(), [(1, 0, 0x1000), (1, 4, 0x2000), (1, 8, 0x1000), (1, 12, 0x2000)]);
    // VTable8 x2: only word 0 (by D)
    assert_eq!(run(&[0x4800 | 1], &BASES, &[]).unwrap(), [(1, 0, 0x2000), (1, 8, 0x2000)]);
    // ImportRun of 3 starting at import 0
    assert_eq!(run(&[0x4A00 | 2], &BASES, &[7, 8, 9]).unwrap(), [(1, 0, 7), (1, 4, 8), (1, 8, 9)]);
    // SmByImport 2 then ImportRun continues at import 3
    assert_eq!(run(&[0x6000 | 2, 0x4A00], &BASES, &[0, 1, 22, 33]).unwrap(), [(1, 0, 22), (1, 4, 33)]);
    // SmSetSectC 2 / SmSetSectD 0 then BySectC / BySectD
    assert_eq!(run(&[0x6200 | 2, 0x6400, 0x4000, 0x4200], &BASES, &[]).unwrap(), [(1, 0, 0x3000), (1, 4, 0x1000)]);
    // SmBySection 1
    assert_eq!(run(&[0x6600 | 1], &BASES, &[]).unwrap(), [(1, 0, 0x2000)]);
    // IncrPosition 16 bytes (field 15), then BySectD
    assert_eq!(run(&[0x8000 | 15, 0x4200], &BASES, &[]).unwrap(), [(1, 16, 0x2000)]);
    // SetPosition 0x0001_0010 (two words) then BySectC
    assert_eq!(run(&[0xA000 | 1, 0x0010, 0x4000], &BASES, &[]).unwrap(), [(1, 0x1_0010, 0x1000)]);
    // LgByImport 3
    assert_eq!(run(&[0xA400, 3], &BASES, &[0, 0, 0, 5]).unwrap(), [(1, 0, 5)]);
    // LgSetOrBySection: BySection 2 / SetSectC 2 / SetSectD 2
    assert_eq!(run(&[0xB400, 2], &BASES, &[]).unwrap(), [(1, 0, 0x3000)]);
    assert_eq!(run(&[0xB400 | (1 << 6), 2, 0x4000], &BASES, &[]).unwrap(), [(1, 0, 0x3000)]);
    assert_eq!(run(&[0xB400 | (2 << 6), 2, 0x4200], &BASES, &[]).unwrap(), [(1, 0, 0x3000)]);
}

#[test]
fn repeat_instructions_expand_the_preceding_instructions() {
    // SmRepeat: blocks=2 (field 1), count=2 (field 1): the two preceding instructions run 2 more times
    let w = [0x4000 | 0, 0x4200 | 0, 0x9100 | 1];
    let got = run(&w, &BASES, &[]).unwrap();
    assert_eq!(got.len(), 6);
    assert_eq!(got.iter().map(|&(_, o, a)| (o, a)).collect::<Vec<_>>(), [(0, 0x1000), (4, 0x2000), (8, 0x1000), (12, 0x2000), (16, 0x1000), (20, 0x2000)]);
    // LgRepeat: 1 block, 3 more times (field 2 -> +1)
    let got = run(&[0x4A00, 0xB000, 2], &BASES, &[1, 2, 3, 4]).unwrap();
    assert_eq!(got.iter().map(|&(_, _, a)| a).collect::<Vec<_>>(), [1, 2, 3, 4]);
    // a two-word instruction counts as ONE instruction for the block size
    let got = run(&[0xA400, 1, 0x9000], &BASES, &[0, 6]).unwrap();
    assert_eq!(got.iter().map(|&(_, o, a)| (o, a)).collect::<Vec<_>>(), [(0, 6), (4, 6)]);
    // negative controls
    assert!(matches!(run(&[0x9000 | 0x100], &BASES, &[]), Err(PefError::BadReloc(w)) if w.contains("reaches back")));
    assert_eq!(run(&[0x4000, 0xB03F, 0xFFFF], &BASES, &[]).unwrap_err(), PefError::RelocTooLarge);
}

#[test]
fn undefined_truncated_and_out_of_range_relocations_are_refused_by_name() {
    // 0x4C00 is in the gap after ImportRun; 0xC000 is past LgSetOrBySection; 0xB800 is undefined
    for op in [0x4C00u16, 0x5000, 0x6800, 0xC000, 0xB800, 0xA800] {
        assert!(matches!(decode(&[op], 0, 1), Err(PefError::UnknownRelocOpcode { opcode, index: 0 }) if opcode == op), "{op:#06x}");
    }
    assert!(matches!(decode(&[0xB400 | (3 << 6), 0], 0, 2), Err(PefError::UnknownRelocOpcode { .. })));
    assert!(matches!(decode(&[0xA000], 0, 1), Err(PefError::BadReloc(w)) if w.contains("cut off")));
    assert!(matches!(decode(&[0x4000], 0, 4), Err(PefError::BadReloc(_))));
    // an import past the table; a section that is not instantiated; sectionC absent
    assert!(matches!(run(&[0x6000 | 5], &BASES, &[1]), Err(PefError::BadReloc(w)) if w.contains("import")));
    assert!(matches!(run(&[0x6600 | 2], &[Some(1), Some(2), None], &[]), Err(PefError::BadReloc(w)) if w.contains("not instantiated")));
    assert!(matches!(run(&[0x4000], &[None, Some(2)], &[]), Err(PefError::BadReloc(w)) if w.contains("sectionC")));
    // negative control for the decoder itself: the same opcodes in range decode
    assert_eq!(decode(&[0x4000], 0, 1).unwrap(), [RelocOp::BySectC(1)]);
}

#[test]
fn relocations_through_a_built_container_use_the_header_section_and_reset_state() {
    let bytes = sample();
    let pef = Pef::parse(&bytes).unwrap();
    let l = pef.loader().unwrap();
    assert_eq!(l.reloc_headers.len(), 1);
    let mut seen = Vec::new();
    l.relocate(&[Some(0x1000), Some(0x2000), None], &[10, 11, 12, 13], &mut |s, o, a| { seen.push((s, o, a)); Ok(()) }).unwrap();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0].0, 1, "the header's section index is reported");
    assert_eq!(seen.iter().map(|x| x.2).collect::<Vec<_>>(), [10, 11, 12, 13]);
}
