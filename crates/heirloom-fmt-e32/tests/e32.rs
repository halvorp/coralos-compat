use heirloom_fmt_e32::{uid_checksum, Compression, E32Error, E32Image, Import, RelocKind};

const CODE_BASE: u32 = 0x8000;
const DATA_BASE: u32 = 0x40_0000;

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// A DLL in the `E32ImageHeaderV` layout real S60 images use: a 0x9C header, 0x20 bytes of code (an import word at
/// 0x10, an export directory of one entry at 0x18), 8 bytes of data, an import section naming euser with one import,
/// a code relocation of the word at 0x04 (text) and a data relocation of the word at 0x00 (inferred, a data address).
fn image() -> Vec<u8> {
    let hdr = 0x9C;
    let mut code = vec![0u8; 0x20];
    put(&mut code, 0x04, CODE_BASE + 0x0C);
    put(&mut code, 0x10, 0x0008_0025); // ordinal 0x25, addend 8
    put(&mut code, 0x18, CODE_BASE + 0x08);
    let mut data = vec![0u8; 8];
    put(&mut data, 0x00, DATA_BASE + 4);

    let name = b"euser{000a0000}[100039e5].dll\0";
    let mut imports = Vec::new();
    imports.extend_from_slice(&0u32.to_le_bytes()); // iSize, set below
    imports.extend_from_slice(&16u32.to_le_bytes()); // DLL name at section offset 16
    imports.extend_from_slice(&1u32.to_le_bytes());
    imports.extend_from_slice(&0x10u32.to_le_bytes());
    imports.extend_from_slice(name);
    while imports.len() % 4 != 0 {
        imports.push(0);
    }
    let isize = imports.len() as u32;
    put(&mut imports, 0, isize);

    let reloc = |off: u16, kind: u16| {
        let mut r = Vec::new();
        r.extend_from_slice(&12u32.to_le_bytes()); // iSize: one 12-byte block
        r.extend_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes()); // page 0
        r.extend_from_slice(&12u32.to_le_bytes()); // block size incl. head
        r.extend_from_slice(&(kind << 12 | off).to_le_bytes());
        r.extend_from_slice(&0u16.to_le_bytes()); // padding entry
        r
    };
    let crel = reloc(0x04, 1);
    let drel = reloc(0x00, 3);

    let code_off = hdr;
    let data_off = code_off + code.len();
    let imp_off = data_off + data.len();
    let crel_off = imp_off + imports.len();
    let drel_off = crel_off + crel.len();
    let mut f = vec![0u8; hdr];
    put(&mut f, 0x00, 0x1000_0079);
    put(&mut f, 0x04, 0x1000_008D);
    put(&mut f, 0x08, 0x2000_1234);
    let ck = uid_checksum(&f[..12]);
    put(&mut f, 0x0C, ck);
    f[0x10..0x14].copy_from_slice(b"EPOC");
    put(&mut f, 0x2C, 0x1200_0001); // ELF imports, header V, DLL
    put(&mut f, 0x30, code.len() as u32);
    put(&mut f, 0x34, data.len() as u32);
    put(&mut f, 0x4C, CODE_BASE);
    put(&mut f, 0x50, DATA_BASE);
    put(&mut f, 0x54, 1);
    put(&mut f, 0x58, (code_off + 0x18) as u32);
    put(&mut f, 0x5C, 1);
    put(&mut f, 0x60, 0x10);
    put(&mut f, 0x64, code_off as u32);
    put(&mut f, 0x68, data_off as u32);
    put(&mut f, 0x6C, imp_off as u32);
    put(&mut f, 0x70, crel_off as u32);
    put(&mut f, 0x74, drel_off as u32);
    put(&mut f, 0x80, 0x2000_1234);
    for part in [&code, &data, &imports, &crel, &drel] {
        f.extend_from_slice(part);
    }
    f
}

#[test]
fn reads_sections_imports_exports_and_relocations() {
    let img = E32Image::parse(&image()).unwrap();
    assert!(img.header.is_dll);
    assert_eq!(img.header.compression, Compression::None);
    assert_eq!(img.header.secure_id, 0x2000_1234);
    assert_eq!(img.code.len(), 0x20);
    assert_eq!(img.data.len(), 8);
    assert_eq!(img.exports, [CODE_BASE + 0x08]);
    assert_eq!(img.imports.len(), 1);
    assert_eq!(img.imports[0].name, "euser{000a0000}[100039e5].dll");
    assert_eq!(img.imports[0].imports, [Import { site: 0x10, ordinal: 0x25, addend: 8 }]);
    assert_eq!(img.code_relocs.len(), 1);
    assert_eq!(img.code_relocs[0].kind, RelocKind::Text);
    assert_eq!(img.data_relocs[0].kind, RelocKind::Inferred);
}

#[test]
fn relocation_moves_each_word_by_its_target_sections_delta() {
    let mut img = E32Image::parse(&image()).unwrap();
    img.relocate(0x7000_0000, 0x0060_0000).unwrap();
    assert_eq!(u32::from_le_bytes(img.code[4..8].try_into().unwrap()), 0x7000_000C);
    assert_eq!(u32::from_le_bytes(img.data[0..4].try_into().unwrap()), 0x0060_0004, "inferred: a data address");
}

#[test]
fn every_proper_prefix_is_refused_never_read_past() {
    let f = image();
    for len in 0..f.len() {
        assert!(E32Image::parse(&f[..len]).is_err(), "prefix {len} was accepted");
    }
}

#[test]
fn a_wrong_uid_checksum_and_a_missing_signature_are_refused_by_name() {
    let mut f = image();
    f[0x0C] ^= 1;
    assert_eq!(E32Image::parse(&f), Err(E32Error::UidChecksum));
    let mut f = image();
    f[0x10] = b'X';
    assert_eq!(E32Image::parse(&f), Err(E32Error::NotE32));
}

#[test]
fn a_relocation_count_that_disagrees_with_the_blocks_is_refused() {
    let mut f = image();
    let crel = u32::from_le_bytes(f[0x70..0x74].try_into().unwrap()) as usize;
    put(&mut f, crel + 4, 2);
    assert_eq!(E32Image::parse(&f), Err(E32Error::Malformed("relocation count differs from iNumberOfRelocs")));
}

#[test]
fn an_import_site_outside_the_code_is_refused() {
    let mut f = image();
    let imp = u32::from_le_bytes(f[0x6C..0x70].try_into().unwrap()) as usize;
    put(&mut f, imp + 12, 0x1E);
    assert_eq!(E32Image::parse(&f), Err(E32Error::Truncated("import site in code")));
}

#[test]
fn byte_pair_images_are_refused_by_name_and_a_bad_deflate_stream_is_not_read() {
    let mut f = image();
    put(&mut f, 0x1C, 0x1028_22AA);
    assert_eq!(E32Image::parse(&f), Err(E32Error::Unsupported("byte-pair compression")));
    let mut f = image();
    put(&mut f, 0x1C, 0x101F_7AFC);
    put(&mut f, 0x7C, 0x100);
    assert!(E32Image::parse(&f).is_err(), "an uncompressed body is not a deflate stream");
}

/// HEIRLOOM_E32_CORPUS names a directory of E32 images taken from the owner's test packages (never part of the
/// sources). Unset = skipped, said so.
fn corpus(name: &str) -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("HEIRLOOM_E32_CORPUS") else {
        eprintln!("SKIP: HEIRLOOM_E32_CORPUS is not set; no real E32 image read");
        return None;
    };
    Some(std::fs::read(std::path::Path::new(&dir).join(name)).unwrap())
}

#[test]
fn real_file_browser_images_read_whole() {
    for (name, dlls, imports, relocs, exports) in [
        ("FileBrowser.exe", 38, 1789, 475, 0),
        ("FileBrowserFileOpClient.dll", 3, 42, 37, 18),
        ("FileBrowserFileOpServer.exe", 5, 124, 68, 0),
    ] {
        let Some(f) = corpus(name) else { return };
        let img = E32Image::parse(&f).unwrap();
        assert_eq!(img.imports.len(), dlls, "{name} DLLs");
        assert_eq!(img.imports.iter().map(|d| d.imports.len()).sum::<usize>(), imports, "{name} imports");
        assert_eq!(img.code_relocs.len(), relocs, "{name} code relocations");
        assert_eq!(img.exports.len(), exports, "{name} exports");
        assert!(img.imports.iter().any(|d| d.name.starts_with("euser{")), "{name} imports euser");
    }
}

#[test]
fn real_hero_of_sparta_decompresses_and_reads_whole() {
    let Some(f) = corpus("HeroOfSparta.exe") else { return };
    let img = E32Image::parse(&f).unwrap();
    assert_eq!(img.header.compression, Compression::Deflate);
    assert_eq!(img.code.len(), 0xBF47C);
    assert_eq!(img.data.len(), 0x23C);
    assert_eq!(img.imports.len(), 23);
    assert!(img.imports.iter().any(|d| d.name.starts_with("euser{")));
    assert!(!img.code_relocs.is_empty() && !img.data_relocs.is_empty());
}
