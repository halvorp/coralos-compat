//! Finding the PowerPC fragment: bare PEF, MacBinary with both forks, AppleDouble header + data fork, and the `cfrg`
//! resource that names it; every way to get it wrong is a named error.
use macppc_engine::cfrg::{self, CfrgError};
use macppc_engine::fixture::{appledouble_app, cfrg_fork, macbinary_app, startup_app};
use macppc_engine::source::{fragment_bytes, SourceError};
use retro_resfork::writer::{macbinary, ForkBuilder};

#[test]
fn a_bare_pef_a_macbinary_and_an_appledouble_pair_yield_the_same_fragment() {
    let pef = startup_app();
    assert_eq!(fragment_bytes(&pef, None).unwrap().1, pef);
    let (name, bytes) = fragment_bytes(&macbinary_app(&pef), None).unwrap();
    assert_eq!((name.as_str(), &bytes), ("Startup", &pef));
    // AppleDouble keeps the data fork in another file: given, it works; absent, it is a named error
    assert_eq!(fragment_bytes(&appledouble_app(), Some(&pef)).unwrap().1, pef);
    assert_eq!(fragment_bytes(&appledouble_app(), None).unwrap_err(), SourceError::NeedDataFork);
}

#[test]
fn cfrg_member_offset_and_length_select_a_slice_of_the_data_fork() {
    let pef = startup_app();
    let mut fork = vec![0xEEu8; 64]; // a fat file: junk (a 68k part) before the PowerPC fragment
    fork.extend_from_slice(&pef);
    let rsrc = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", 1, 1, 64, pef.len() as u32, "Fat")).build();
    let (name, got) = fragment_bytes(&macbinary("Fat", b"APPL", b"FAT?", &fork, &rsrc), None).unwrap();
    assert_eq!((name.as_str(), got), ("Fat", pef.clone()));
    // negative controls: a slice that runs past the fork; a fragment in the resource fork; a 68k-only cfrg; no cfrg at all
    let bad = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", 1, 1, 64, 1 << 20, "x")).build();
    assert!(matches!(fragment_bytes(&macbinary("x", b"APPL", b"x   ", &fork, &bad), None), Err(SourceError::OutOfFork { .. })));
    let rf = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", 1, 2, 0, 0, "x")).build();
    assert_eq!(fragment_bytes(&macbinary("x", b"APPL", b"x   ", &fork, &rf), None).unwrap_err(), SourceError::UnsupportedLocation(2));
    let m68k = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"m68k", 1, 1, 0, 0, "x")).build();
    assert_eq!(fragment_bytes(&macbinary("x", b"APPL", b"x   ", &fork, &m68k), None).unwrap_err(), SourceError::NoPowerPcMember);
    let lib = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", 0, 1, 0, 0, "x")).build();
    assert_eq!(fragment_bytes(&macbinary("x", b"APPL", b"x   ", &fork, &lib), None).unwrap_err(), SourceError::NoPowerPcMember, "an import library is not an application");
    let none = ForkBuilder::new().add(b"TEXT", 128, None, b"hi").build();
    assert_eq!(fragment_bytes(&macbinary("x", b"APPL", b"x   ", &fork, &none), None).unwrap_err(), SourceError::NoCfrg);
}

#[test]
fn cfrg_parse_roundtrip_and_damage() {
    let r = cfrg::build(b"pwpc", 1, 1, 0x40, 0x1234, "My App");
    let m = cfrg::parse(&r).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!((&m[0].arch, m[0].usage, m[0].location, m[0].offset, m[0].length, m[0].name.as_str()), (b"pwpc", 1, 1, 0x40, 0x1234, "My App"));
    let mut bad = r.clone(); bad[11] = 2;
    assert_eq!(cfrg::parse(&bad).unwrap_err(), CfrgError::BadVersion(2));
    assert!(matches!(cfrg::parse(&r[..20]).unwrap_err(), CfrgError::Truncated(_)));
    assert!(matches!(cfrg::parse(&r[..40]).unwrap_err(), CfrgError::Truncated(_) | CfrgError::BadMemberSize { .. }));
    let mut bad = r.clone(); bad[32 + 41] = 41; // memberSize not a multiple of 4
    assert!(matches!(cfrg::parse(&bad).unwrap_err(), CfrgError::BadMemberSize { index: 0 }));
    let mut bad = r.clone(); bad[32 + 42] = 200; // name longer than the member
    assert!(matches!(cfrg::parse(&bad).unwrap_err(), CfrgError::BadMemberSize { index: 0 }));
    // the fork helper builds what the census reads
    assert!(cfrg_fork().len() > 32);
}

#[test]
fn a_file_that_is_not_pef_or_a_mac_container_is_refused() {
    // looks like neither: treated as a raw fork and rejected by the fork parser, never a panic
    assert!(matches!(fragment_bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], None), Err(SourceError::Fork(_))));
    assert!(fragment_bytes(&[], None).is_err());
}
