use retro_resfork::container::{crc16, ContainerError};
use retro_resfork::writer::{appledouble, macbinary, ForkBuilder};
use retro_resfork::{open, ForkError, Kind, ResFork};

fn fork() -> Vec<u8> {
    ForkBuilder::new()
        .add(b"CODE", 0, None, &[0, 0, 0, 0x20, 1, 2, 3, 4])
        .add(b"CODE", 1, Some("Main"), b"seg one")
        .add(b"STR ", 128, Some("Hello"), b"\x05hello")
        .attrs(0x20)
        .add(b"STR ", -3, None, b"neg")
        .build()
}

#[test]
fn parses_types_ids_names_attrs_and_data() {
    let f = ResFork::parse(fork()).unwrap();
    assert_eq!(f.len(), 4);
    assert_eq!(f.types(), vec![*b"CODE", *b"STR "]);
    assert_eq!((f.count(*b"CODE"), f.count(*b"STR "), f.count(*b"MENU")), (2, 2, 0));
    let i = f.find(*b"STR ", 128).unwrap();
    assert_eq!(f.data(i).unwrap(), b"\x05hello");
    assert_eq!(f.name(i).unwrap(), b"Hello");
    assert_eq!(f.entry(i).unwrap().attrs, 0x20);
    assert_eq!(f.data(f.find(*b"STR ", -3).unwrap()).unwrap(), b"neg", "negative ids survive");
    assert_eq!(f.name(f.find(*b"STR ", -3).unwrap()), None);
    // GetNamedResource folds case; GetIndResource is 1-based
    assert_eq!(f.find_named(*b"CODE", b"main"), f.find(*b"CODE", 1));
    assert_eq!(f.nth(*b"CODE", 2), f.find(*b"CODE", 1));
    // negative controls
    assert_eq!(f.find(*b"CODE", 2), None, "no such id");
    assert_eq!(f.find(*b"MENU", 0), None, "no such type");
    assert_eq!(f.find_named(*b"STR ", b"main"), None, "right name, wrong type");
    assert_eq!(f.nth(*b"CODE", 0), None, "index 0 is not a resource");
    assert_eq!(f.nth(*b"CODE", 3), None, "index past the end");
}

#[test]
fn an_empty_fork_is_valid_and_empty() {
    let f = ResFork::parse(ForkBuilder::new().build()).unwrap();
    assert!(f.is_empty() && f.types().is_empty());
}

#[test]
fn malformed_forks_are_named_refusals() {
    let good = fork();
    assert!(matches!(ResFork::parse(vec![0; 8]), Err(ForkError::Truncated("header"))));
    // cut off the map
    assert!(matches!(ResFork::parse(good[..good.len() - 4].to_vec()), Err(ForkError::BadOffset("map outside the fork"))));
    // type list offset inside the fixed part of the map
    let mut bad = good.clone();
    let map_off = u32::from_be_bytes(bad[4..8].try_into().unwrap()) as usize;
    bad[map_off + 24..map_off + 26].copy_from_slice(&4u16.to_be_bytes());
    assert_eq!(ResFork::parse(bad).unwrap_err(), ForkError::BadOffset("type list offset"));
    // a resource whose data offset points past the data area
    let mut bad = good.clone();
    let rl = map_off + 28 + 2 + 2 * 8; // first reference list entry
    bad[rl + 5..rl + 8].copy_from_slice(&[0x7F, 0xFF, 0xFF]);
    assert!(ResFork::parse(bad).is_err());
    // a resource whose length claims more than the data area holds
    let mut bad = good.clone();
    let data_off = u32::from_be_bytes(bad[0..4].try_into().unwrap()) as usize;
    bad[data_off..data_off + 4].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
    assert_eq!(ResFork::parse(bad).unwrap_err(), ForkError::BadOffset("resource data past the data area"));
    // a name offset past the map
    let mut bad = good;
    bad[rl + 14..rl + 16].copy_from_slice(&0x7000u16.to_be_bytes());
    assert!(ResFork::parse(bad).is_err());
}

#[test]
fn macbinary_ii_locates_both_forks_and_checks_its_crc() {
    let rsrc = fork();
    let data = vec![0xAA; 130]; // pads to 256
    let mb = macbinary("Test App", b"APPL", b"TEST", &data, &rsrc);
    let o = open(&mb).unwrap();
    assert_eq!((o.kind, o.name.as_deref(), o.file_type, o.creator), (Kind::MacBinary, Some("Test App"), Some(*b"APPL"), Some(*b"TEST")));
    assert_eq!(&mb[o.data.clone().unwrap()], &data[..]);
    assert_eq!(&mb[o.rsrc.clone()], &rsrc[..]);
    assert_eq!(o.rsrc.start, 128 + 256, "resource fork follows the data fork padded to 128");
    ResFork::parse(mb[o.rsrc].to_vec()).unwrap();
    // NEGATIVE: one flipped header byte -> a named CRC refusal, not a silent raw-fork reading
    let mut bad = mb.clone();
    bad[69] ^= 1;
    assert!(matches!(open(&bad), Err(ContainerError::BadCrc { .. })), "{:?}", open(&bad));
    // NEGATIVE: fork length beyond the file
    let mut bad = mb;
    bad[87..91].copy_from_slice(&0x0100_0000u32.to_be_bytes());
    let crc = crc16(&bad[..124]);
    bad[124..126].copy_from_slice(&crc.to_be_bytes());
    assert!(open(&bad).is_err());
}

#[test]
fn crc16_matches_the_xmodem_check_value() {
    assert_eq!(crc16(b"123456789"), 0x31C3);
}

#[test]
fn macbinary_without_a_resource_fork_is_refused_by_name() {
    let mb = macbinary("Data Only", b"TEXT", b"ttxt", b"hello", &[]);
    assert_eq!(open(&mb), Err(ContainerError::NoResourceFork));
}

#[test]
fn appledouble_header_file_yields_the_resource_fork() {
    let rsrc = fork();
    let ad = appledouble("Test App", &rsrc);
    let o = open(&ad).unwrap();
    assert_eq!((o.kind, o.name.as_deref(), o.data.clone()), (Kind::AppleDouble, Some("Test App"), None));
    assert_eq!(&ad[o.rsrc], &rsrc[..]);
    // NEGATIVE: an entry that runs past the file
    let mut bad = ad.clone();
    bad[26 + 12 + 8..26 + 12 + 12].copy_from_slice(&0xFFFFu32.to_be_bytes());
    assert!(matches!(open(&bad), Err(ContainerError::BadAppleDouble(_))));
    // NEGATIVE: only a name entry -> no resource fork
    let mut no_rsrc = ad;
    no_rsrc[26 + 12..26 + 16].copy_from_slice(&9u32.to_be_bytes()); // entry id 2 -> 9 (unknown)
    assert_eq!(open(&no_rsrc), Err(ContainerError::NoResourceFork));
}

#[test]
fn a_raw_fork_is_taken_as_is_and_garbage_is_refused_by_the_fork_parser() {
    let raw = fork();
    let o = open(&raw).unwrap();
    assert_eq!((o.kind, o.rsrc.clone()), (Kind::Raw, 0..raw.len()));
    let junk = vec![0x55u8; 300];
    let o = open(&junk).unwrap();
    assert_eq!(o.kind, Kind::Raw);
    assert!(ResFork::parse(junk).is_err());
}

#[test]
fn a_changed_fork_keeps_its_indices_and_writes_back_in_the_resource_manager_format() {
    let mut f = ResFork::parse(fork()).unwrap();
    let (code1, hello) = (f.find(*b"CODE", 1).unwrap(), f.find(*b"STR ", 128).unwrap());
    // add: the next index, ids need not be new, the other indices do not move
    let menu = f.add(*b"MENU", 5, Some(b"File"), 0x02, b"menu".to_vec());
    assert_eq!((menu, f.len(), f.find(*b"CODE", 1), f.find(*b"STR ", 128)), (4, 5, Some(code1), Some(hello)));
    assert_eq!((f.data(menu).unwrap(), f.name(menu).unwrap(), f.entry(menu).unwrap().attrs), (&b"menu"[..], &b"File"[..], 0x02));
    // rewrite bytes (the name and attributes survive), change id and name, set attributes
    assert!(f.set_data(hello, b"\x02hi".to_vec()) && f.set_attrs(hello, 0x22));
    assert_eq!((f.data(hello).unwrap(), f.name(hello).unwrap(), f.entry(hello).unwrap().attrs), (&b"\x02hi"[..], &b"Hello"[..], 0x22));
    assert!(f.set_info(code1, 7, Some(b"seven")) && f.set_info(hello, 129, None));
    assert_eq!((f.find(*b"CODE", 7), f.find(*b"CODE", 1), f.name(code1).unwrap(), f.name(hello).unwrap()), (Some(code1), None, &b"seven"[..], &b"Hello"[..]));
    // remove: the index stays and answers nothing; counts and lookups skip it
    assert!(f.remove(f.find(*b"STR ", -3).unwrap()));
    assert_eq!((f.len(), f.count(*b"STR "), f.entry(3).is_none(), f.data(3)), (5, 1, true, None));
    assert!(!f.remove(3) && !f.set_data(3, vec![1]), "a removed index cannot be changed or removed again");
    // the file image parses back to the same resources, each type's ids sorted
    let back = ResFork::parse(f.to_bytes()).unwrap();
    assert_eq!(back.types(), vec![*b"CODE", *b"STR ", *b"MENU"]);
    assert_eq!((back.count(*b"CODE"), back.count(*b"STR "), back.count(*b"MENU")), (2, 1, 1));
    assert_eq!(back.data(back.find(*b"STR ", 129).unwrap()).unwrap(), b"\x02hi");
    assert_eq!(back.entry(back.find(*b"STR ", 129).unwrap()).unwrap().attrs, 0x22);
    assert_eq!(back.name(back.find(*b"MENU", 5).unwrap()).unwrap(), b"File");
    assert_eq!(back.find(*b"CODE", 7).map(|i| back.name(i).unwrap().to_vec()), Some(b"seven".to_vec()));
    let ids: Vec<i16> = (1..=2).map(|n| back.entry(back.nth(*b"CODE", n).unwrap()).unwrap().id).collect();
    assert_eq!(ids, vec![0, 7], "sorted by id inside a type");
}

#[test]
fn a_fork_is_laid_out_as_createresfile_lays_it_out() {
    // header (16) + 240 reserved bytes, data at 256; a new empty file is 256 + a 30-byte map = 286 bytes (Inside Macintosh: More
    // Macintosh Toolbox, "Resource File Format"): a program that checks a fresh preferences file's length is checking exactly this
    let empty = ForkBuilder::new().build();
    assert_eq!(empty.len(), 286);
    assert_eq!(&empty[..16], [0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 30], "dataOff 256, mapOff 256, dataLen 0, mapLen 30");
    assert!(empty[16..256].iter().all(|&b| b == 0), "the reserved area is zero");
    let one = ForkBuilder::new().add(b"TEXT", 1, None, b"abc").build();
    assert_eq!(u32::from_be_bytes(one[..4].try_into().unwrap()), 256);
    assert_eq!(&one[256..263], [0, 0, 0, 3, b'a', b'b', b'c'], "length-prefixed data starts at 256");
    assert_eq!(ResFork::parse(one).unwrap().data(0).unwrap(), b"abc");
}
