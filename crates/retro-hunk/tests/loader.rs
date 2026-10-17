use retro_guest_mem::GuestMem;
use retro_hunk::writer::{build, Hunk};
use retro_hunk::{load, HunkError};

fn good() -> Vec<u8> {
    // code: word0 holds an offset 8 into hunk 1 (reloc'd); data: 16 bytes; bss: 64 bytes
    let code = Hunk::code(vec![0, 0, 0, 8, 0x4E, 0x75, 0, 0], vec![(1, 0)]);
    build(&[code, Hunk::data(vec![1; 16], vec![]), Hunk::bss(64)], None)
}

#[test]
fn loads_relocates_and_zeroes_bss() {
    let mut m = GuestMem::new(1 << 20);
    m.write_bytes(0x10000 + 8 + 16, &[0xAA; 64]).unwrap(); // dirty RAM where BSS will land
    let l = load(&mut m, &good(), 0x10000).unwrap();
    assert_eq!(l.bases, vec![0x10000, 0x10008, 0x10018]);
    assert_eq!(m.read_u32(0x10000).unwrap(), 0x10008 + 8, "reloc adds hunk 1 base");
    assert_eq!(m.read_u8(0x10018).unwrap(), 0, "BSS zeroed");
    assert_eq!(l.entry, 0x10000);
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let f = good();
    for cut in 0..f.len() {
        let mut m = GuestMem::new(1 << 20);
        // a prefix may legitimately end on a hunk boundary and load; it must never panic
        let _ = load(&mut m, &f[..cut], 0x10000);
    }
    let mut m = GuestMem::new(1 << 20);
    assert_eq!(load(&mut m, &f[..3], 0x10000).unwrap_err(), HunkError::Truncated);
    assert_eq!(load(&mut m, &f[..30], 0x10000).unwrap_err(), HunkError::Truncated);
}

#[test]
fn single_byte_corruption_never_panics() {
    let f = good();
    for i in 0..f.len() {
        for v in [0x00u8, 0x7F, 0xFF] {
            let mut g = f.clone();
            g[i] = v;
            let mut m = GuestMem::new(1 << 20);
            let _ = load(&mut m, &g, 0x10000);
        }
    }
}

#[test]
fn bad_reloc_target_refused() {
    let code = Hunk::code(vec![0; 8], vec![(7, 0)]);
    let f = build(&[code], None);
    let e = load(&mut GuestMem::new(1 << 20), &f, 0x10000).unwrap_err();
    assert_eq!(e, HunkError::RelocToUnknownHunk { target: 7 });
}

#[test]
fn reloc_offset_outside_hunk_refused() {
    let code = Hunk::code(vec![0; 8], vec![(0, 6)]); // 6+4 > 8
    let e = load(&mut GuestMem::new(1 << 20), &build(&[code], None), 0x10000).unwrap_err();
    assert_eq!(e, HunkError::RelocOutsideRam { hunk: 0, offset: 6 });
}

#[test]
fn oversized_bss_refused_by_name() {
    let f = build(&[Hunk::code(vec![0; 4], vec![]), Hunk::bss(0x3000_0000)], None);
    let e = load(&mut GuestMem::new(1 << 20), &f, 0x10000).unwrap_err();
    assert!(matches!(e, HunkError::OversizedBss { hunk: 1, .. }), "{e:?}");
    assert!(e.to_string().contains("BSS"));
}

#[test]
fn bss_larger_than_declared_refused() {
    let f = build(&[Hunk::bss(64)], Some(&[4]));
    let e = load(&mut GuestMem::new(1 << 20), &f, 0x10000).unwrap_err();
    assert_eq!(e, HunkError::BssExceedsDeclared { hunk: 0 });
}

#[test]
fn section_larger_than_declared_refused() {
    let f = build(&[Hunk::code(vec![0; 32], vec![])], Some(&[2]));
    let e = load(&mut GuestMem::new(1 << 20), &f, 0x10000).unwrap_err();
    assert_eq!(e, HunkError::SectionExceedsDeclared { hunk: 0 });
}

#[test]
fn huge_hunk_table_and_bad_magic_refused() {
    let mut f = good();
    f[11] = 0xFF; // table size low byte -> huge
    assert!(load(&mut GuestMem::new(1 << 20), &f, 0x10000).is_err());
    let mut g = good();
    g[3] = 0x01;
    assert_eq!(load(&mut GuestMem::new(1 << 20), &g, 0x10000).unwrap_err(), HunkError::NotAHunkFile);
}

#[test]
fn unsupported_hunk_type_named() {
    let mut f = good();
    let pos = f.windows(4).position(|w| w == [0, 0, 0x03, 0xEA]).unwrap(); // the DATA id
    f[pos + 2] = 0x03;
    f[pos + 3] = 0xF4; // HUNK_OVERLAY-ish / DEBUG
    let e = load(&mut GuestMem::new(1 << 20), &f, 0x10000).unwrap_err();
    assert_eq!(e, HunkError::UnsupportedHunkType(0x3F4));
}

#[test]
fn image_outside_ram_refused() {
    let e = load(&mut GuestMem::new(0x10008), &good(), 0x10000).unwrap_err();
    assert!(matches!(e, HunkError::SectionOutsideRam { .. } | HunkError::OversizedBss { .. }), "{e:?}");
}
