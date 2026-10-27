use super::*;
use alloc::vec;

const DRIVE: &str = "/data/build/corpus-symbian/drive";
extern crate std;

fn real(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(std::format!("{DRIVE}/{rel}")).ok()
}

/// A synthetic plain file: `res` = (bytes, has_unicode_runs); signature offset id `0x0abc_d000` as resource 1.
fn build(res: &[(&[u8], bool)]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&UID1_PLAIN.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&0x000a_bcdu32.to_le_bytes());
    f.extend_from_slice(&[0; 4]); // checksum
    f.push(0); // flags
    f.extend_from_slice(&[0, 0]); // largest
    let mut all: Vec<(Vec<u8>, bool)> = vec![];
    let mut sig = 4u32.to_le_bytes().to_vec();
    sig.extend_from_slice(&0x0abc_d001u32.to_le_bytes());
    all.push((sig, false));
    for (b, u) in res {
        all.push((b.to_vec(), *u));
    }
    let mut bits = vec![0u8; (all.len() + 7) / 8];
    for (i, (_, u)) in all.iter().enumerate() {
        if *u {
            bits[i / 8] |= 1 << (i % 8);
        }
    }
    f.extend_from_slice(&bits);
    let mut offs = vec![];
    for (b, _) in &all {
        offs.push(f.len() as u16);
        f.extend_from_slice(b);
    }
    let index_at = f.len() as u16;
    for o in offs {
        f.extend_from_slice(&o.to_le_bytes());
    }
    f.extend_from_slice(&index_at.to_le_bytes());
    f
}

#[test]
fn a_synthetic_file_gives_each_resource_its_bytes_by_id() {
    // resource 2: raw; resource 3: runs [unicode "Hi" (2 bytes), raw 0x01 0x00]
    let f = build(&[(b"\x01\x02\x03", false), (&[2, b'H', b'i', 2, 1, 0], true)]);
    let r = RscFile::parse(&f).unwrap();
    assert_eq!((r.count(), r.id_offset), (3, 0x0abc_d000));
    assert_eq!(r.resource(0x0abc_d002).unwrap(), vec![1, 2, 3]);
    assert_eq!(r.resource(3).unwrap(), vec![b'H', 0, b'i', 0, 1, 0], "an offset-less id is accepted");
    assert_eq!(r.resource(0x0abc_d004), Err(RscError::NotOwned(0x0abc_d004)));
    assert_eq!(r.resource(0x0123_4002), Err(RscError::NotOwned(0x0123_4002)), "a foreign offset");
    assert_eq!(r.resource(0), Err(RscError::NotOwned(0)));
}

#[test]
fn an_odd_unicode_run_is_padded_and_an_ltext_reads_through_the_pad() {
    // runs: empty unicode, raw 5 bytes (LONG 0 + the LTEXT length 2, so the text would start at odd offset 5), unicode "OK"
    let res = [0, 5, 0, 0, 0, 0, 2, 2, b'O', b'K'];
    let f = build(&[(&res, true)]);
    let r = RscFile::parse(&f).unwrap();
    let d = r.resource(2).unwrap();
    assert_eq!(d, vec![0, 0, 0, 0, 2, 0xAB, b'O', 0, b'K', 0]);
    let mut rd = Reader::new(&d);
    assert_eq!(rd.long().unwrap(), 0);
    assert_eq!(rd.ltext().unwrap(), "OK");
}

#[test]
fn malformed_files_are_named_errors_not_panics() {
    assert!(matches!(RscFile::parse(&[]), Err(RscError::Truncated(_))));
    let mut f = build(&[(b"x", false)]);
    let n = f.len();
    f[n - 2] = 0xFF;
    f[n - 1] = 0xFF;
    assert!(matches!(RscFile::parse(&f), Err(RscError::BadRuns(_))));
    let mut d = build(&[(b"x", false)]);
    d[0..4].copy_from_slice(&UID1_DICTIONARY.to_le_bytes());
    assert!(matches!(RscFile::parse(&d), Err(RscError::Unsupported(m)) if m.contains("dictionary")));
    // a run longer than its resource, and an SCSU tag
    let f = build(&[(&[9, b'a'], true)]);
    assert!(matches!(RscFile::parse(&f).unwrap().resource(2), Err(RscError::BadRuns(_))));
    let f = build(&[(&[1, 0x0F], true)]);
    assert_eq!(RscFile::parse(&f).unwrap().resource(2), Err(RscError::Scsu(0x0F)));
    let mut rd = Reader::new(&[1]);
    assert!(matches!(rd.long(), Err(RscError::Field(_))));
}

#[test]
fn hero_of_sparta_resources_decode_to_its_caption() {
    let Some(rsc) = real("hero-of-sparta/resource/apps/heroofsparta.rsc") else { return };
    let r = RscFile::parse(&rsc).unwrap();
    assert_eq!((r.uid1, r.uid3, r.count(), r.id_offset), (UID1_PLAIN, 0x8f6f, 4, 0x08f6_f000));
    // id 2 is the TBUF caption; id 4 is LOCALISABLE_APP_INFO { LONG, LLINK, LTEXT short_caption, CAPTION_AND_ICON_INFO { LONG, LLINK, LTEXT caption, WORD icons, LTEXT icon_file }, ... }
    let mut c = Reader::new(&r.resource(0x08f6_f002).unwrap()).rest_utf16().unwrap();
    assert_eq!(c, "HeroOfSparta");
    let info = r.resource(0x08f6_f004).unwrap();
    let mut rd = Reader::new(&info);
    assert_eq!((rd.long().unwrap(), rd.long().unwrap()), (0, 0));
    assert_eq!(rd.ltext().unwrap(), "HeroOfSparta");
    assert_eq!((rd.long().unwrap(), rd.long().unwrap()), (0, 0));
    c = rd.ltext().unwrap();
    assert_eq!(c, "HeroOfSparta");
    assert_eq!(rd.word().unwrap(), 1);
    assert_eq!(rd.ltext().unwrap(), "\\resource\\apps\\heroofsparta_icon.mif");
}

#[test]
fn the_registration_files_name_the_localisable_resource() {
    for (rel, app, uid3) in [
        ("hero-of-sparta/private/10003a3f/import/apps/heroofsparta_reg.rsc", "HeroOfSparta", 0x2003_0e78u32),
        ("nokiafilebrowser/private/10003a3f/import/apps/filebrowser_reg.rsc", "FileBrowser", 0x1028_28d6),
    ] {
        let Some(bytes) = real(rel) else { continue };
        let r = RscFile::parse(&bytes).unwrap();
        assert_eq!((r.uid3, r.count(), r.id_offset), (uid3, 1, 0));
        let d = r.resource(1).unwrap();
        let mut rd = Reader::new(&d);
        assert_eq!((rd.long().unwrap(), rd.long().unwrap()), (0, 0));
        assert_eq!(rd.ltext().unwrap(), app); // app_file
        assert_eq!(rd.long().unwrap(), 0); // attributes
        std::println!("{app}: localisable file {:?} id {:#x}", rd.ltext().unwrap(), rd.long().unwrap());
    }
}

#[test]
fn every_resource_of_file_browsers_rsc_decodes() {
    let Some(rsc) = real("nokiafilebrowser/resource/apps/filebrowser.rsc") else { return };
    let r = RscFile::parse(&rsc).unwrap();
    assert_eq!(r.uid3, 0x3ec90);
    assert_eq!(r.id_offset, 0x3ec9_0000);
    std::println!("filebrowser.rsc: {} resources", r.count());
    for i in 1..=r.count() as u32 {
        let d = r.resource(r.id_offset | i).unwrap_or_else(|e| panic!("resource {i}: {e:?}"));
        assert!(d.len() < 4096);
    }
}
