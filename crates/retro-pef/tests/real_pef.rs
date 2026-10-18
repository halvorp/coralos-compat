//! A real, unmodified PEF: SheepShaver's `ethernet.ndrv` (a Mac OS network driver, built by CodeWarrior). It is NOT in
//! this repository; it lives in the macemu source tree. Set `PEF_CORPUS` to a directory holding it (default
//! `/data/refs/retro/macemu/SheepShaver/src/EthernetDriver`); a missing file SKIPS loudly.
use retro_pef::loader::SymbolClass;
use retro_pef::{Pef, SectionKind};

fn ndrv() -> Option<Vec<u8>> {
    let root = std::env::var("PEF_CORPUS").unwrap_or_else(|_| "/data/refs/retro/macemu/SheepShaver/src/EthernetDriver".into());
    match std::fs::read(format!("{root}/ethernet.ndrv")) {
        Ok(b) => Some(b),
        Err(e) => { eprintln!("SKIPPED: real PEF {root}/ethernet.ndrv not available ({e})"); None }
    }
}

#[test]
fn the_real_driver_parses_to_the_last_byte() {
    let Some(bytes) = ndrv() else { return };
    let pef = Pef::parse(&bytes).unwrap();
    assert_eq!(&pef.header.arch, b"pwpc");
    assert!(pef.sections.iter().any(|s| s.kind == SectionKind::Code));
    assert!(pef.sections.iter().any(|s| s.kind == SectionKind::Loader));
    for (i, s) in pef.sections.iter().enumerate() {
        if s.kind.instantiated() { assert_eq!(pef.section_image(i).unwrap().len(), s.total_size as usize, "section {i}"); }
    }
    let l = pef.loader().unwrap();
    assert!(!l.libraries.is_empty() && !l.imports.is_empty());
    // every import is attributed to a library, with a class the format defines
    for i in &l.imports { assert!(i.library < l.libraries.len() && !i.name.is_empty()); }
    eprintln!("ndrv imports: {:?}", l.imports.iter().map(|i| (&l.libraries[i.library].name, &i.name, i.class.name(), i.weak)).collect::<Vec<_>>());
    eprintln!("ndrv exports: {:?}", l.exports.iter().map(|e| (&e.name, e.class.name(), e.target)).collect::<Vec<_>>());
}

#[test]
fn every_export_of_the_real_driver_is_found_through_its_own_hash_table() {
    let Some(bytes) = ndrv() else { return };
    let l = Pef::parse(&bytes).unwrap().loader().unwrap();
    // This is the check of the hash function against a CodeWarrior-built table: a wrong function misses names.
    assert!(l.hash_mismatches().is_empty(), "stored hash words disagree with the hash function: {:?}", l.hash_mismatches().iter().map(|e| &e.name).collect::<Vec<_>>());
    for e in &l.exports {
        let found = l.find_export(&e.name).unwrap_or_else(|| panic!("{} not found through the hash table", e.name));
        assert_eq!(found, e);
    }
    // negative control: a name that is not exported is not found
    assert!(l.find_export("NoSuchSymbolInThisDriver").is_none());
}

#[test]
fn the_real_drivers_relocations_decode_and_apply_inside_their_sections() {
    let Some(bytes) = ndrv() else { return };
    let pef = Pef::parse(&bytes).unwrap();
    let l = pef.loader().unwrap();
    let bases: Vec<Option<u32>> = pef.sections.iter().enumerate().map(|(i, s)| s.kind.instantiated().then_some(0x10_0000 + 0x1_0000 * i as u32)).collect();
    let imports: Vec<u32> = (0..l.imports.len() as u32).map(|i| 0x9000_0000 + 16 * i).collect();
    let mut words = 0usize;
    l.relocate(&bases, &imports, &mut |sec, off, _| {
        let s = &pef.sections[usize::from(sec)];
        assert!(off + 4 <= s.total_size, "relocated word {off:#x} lies outside section {sec} ({} bytes)", s.total_size);
        words += 1;
        Ok(())
    }).unwrap();
    assert!(words > 0, "a real driver relocates something");
    eprintln!("ndrv: {words} relocated words, {} import symbols, TVector imports {}", l.imports.len(), l.imports.iter().filter(|i| i.class == SymbolClass::TVector).count());
}

#[test]
fn a_truncated_real_driver_is_refused_by_name_never_a_panic() {
    let Some(bytes) = ndrv() else { return };
    for cut in [0, 7, 39, 60, bytes.len() / 2, bytes.len() - 1] {
        let r = Pef::parse(&bytes[..cut]).and_then(|p| { let l = p.loader()?; for i in 0..p.sections.len() { if p.sections[i].kind.instantiated() { p.section_image(i)?; } } Ok(l) });
        assert!(r.is_err(), "cut at {cut} must be refused");
    }
}
