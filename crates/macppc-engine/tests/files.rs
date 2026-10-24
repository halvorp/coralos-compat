//! The File Manager through the PowerPC glue (`macppc_engine::files`): the C-callable `Files.h` routines a PowerPC program imports from
//! InterfaceLib, called the way a PowerPC CPU calls them (r3.. = the C arguments), over the SAME volume the 68k engine serves.
use guestui_abi::RecordingGuestUi;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const NAME: u32 = 0x3200;
const BUF: u32 = 0x3400;
const OUT: u32 = 0x3300;

fn machine() -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    m.mac.fm.tree.put_file(root, "Data", b"0123456789".to_vec()).unwrap();
    m
}
fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }
/// One call by name; returns r3 as the OSErr.
fn call(m: &mut M, name: &str, args: &[u32]) -> i16 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32 as i16
}
fn open(m: &mut M, name: &str) -> i16 {
    pstr(m, NAME, name);
    assert_eq!(call(m, "FSOpen", &[NAME, 0, OUT]), 0);
    m.mac.mem.read_u16(OUT).unwrap() as i16
}

#[test]
fn read_write_position_and_eof_round_trip_through_the_c_routines() {
    let mut m = machine();
    let r = open(&mut m, "Data");
    assert!(r > 0);
    m.mac.mem.write_u32(OUT, 4).unwrap();
    assert_eq!(call(&mut m, "FSRead", &[r as u32, OUT, BUF]), 0);
    assert_eq!((m.mac.mem.read_u32(OUT).unwrap(), m.mac.mem.bytes(m.mac.mem.span(BUF, 4).unwrap()).unwrap()), (4, &b"0123"[..]));
    assert_eq!(call(&mut m, "GetFPos", &[r as u32, OUT]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 4, "the mark moved with the read");
    assert_eq!(call(&mut m, "GetEOF", &[r as u32, OUT]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 10);
    // a read past the end is eofErr with the SHORT count still stored
    m.mac.mem.write_u32(OUT, 100).unwrap();
    assert_eq!(call(&mut m, "FSRead", &[r as u32, OUT, BUF]), -39);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 6);
    // SetFPos(fsFromStart = 1), write, grow, truncate
    assert_eq!(call(&mut m, "SetFPos", &[r as u32, 1, 2]), 0);
    m.mac.mem.write_bytes(BUF, b"ab").unwrap();
    m.mac.mem.write_u32(OUT, 2).unwrap();
    assert_eq!(call(&mut m, "FSWrite", &[r as u32, OUT, BUF]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 2);
    assert_eq!(call(&mut m, "SetEOF", &[r as u32, 6]), 0);
    assert_eq!(call(&mut m, "FSClose", &[r as u32]), 0);
    let node = m.mac.fm.locate(0, 0, Some(b"Data")).unwrap().node.unwrap();
    assert_eq!(m.mac.fm.tree.file_data(node).unwrap(), b"01ab45");
}

#[test]
fn a_bad_refnum_and_a_missing_file_return_their_error_codes_and_store_nothing() {
    let mut m = machine();
    m.mac.mem.write_u32(OUT, 0xDEAD_BEEF).unwrap();
    assert_eq!(call(&mut m, "GetEOF", &[77, OUT]), -51, "rfNumErr");
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xDEAD_BEEF, "no output on an error");
    assert_eq!(call(&mut m, "FSClose", &[77]), -51);
    pstr(&mut m, NAME, "Nothing");
    m.mac.mem.write_u16(OUT, 0x1234).unwrap();
    assert_eq!(call(&mut m, "FSOpen", &[NAME, 0, OUT]), -43, "fnfErr");
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap(), 0x1234);
}

#[test]
fn the_hfs_forms_create_with_type_and_creator_open_and_delete() {
    let mut m = machine();
    pstr(&mut m, NAME, "New");
    // HCreate(vRefNum, dirID, name, creator, fileType)
    assert_eq!(call(&mut m, "HCreate", &[0, 0, NAME, u32::from_be_bytes(*b"CoRa"), u32::from_be_bytes(*b"TEXT")]), 0);
    assert_eq!(call(&mut m, "HCreate", &[0, 0, NAME, 0, 0]), -48, "dupFNErr: the name is taken");
    let node = m.mac.fm.locate(0, 0, Some(b"New")).unwrap().node.unwrap();
    let f = m.mac.fm.cat_info(node).finfo;
    assert_eq!((f.file_type, f.creator), (*b"TEXT", *b"CoRa"));
    // HOpenDF(vRefNum, dirID, name, permission, *refNum), then GetVRefNum on the refnum
    assert_eq!(call(&mut m, "HOpenDF", &[0, 0, NAME, 3, OUT]), 0);
    let r = m.mac.mem.read_u16(OUT).unwrap() as u32;
    assert_eq!(call(&mut m, "GetVRefNum", &[r, OUT + 4]), 0);
    assert_eq!(m.mac.mem.read_u16(OUT + 4).unwrap() as i16, -1, "the boot volume");
    assert_eq!(call(&mut m, "GetVRefNum", &[999, OUT + 4]), -51);
    assert_eq!(call(&mut m, "FSDelete", &[NAME, 0]), -47, "fBsyErr: an open file is not deleted");
    assert_eq!(call(&mut m, "FSClose", &[r]), 0);
    assert_eq!(call(&mut m, "FSDelete", &[NAME, 0]), 0);
    assert_eq!(call(&mut m, "HOpen", &[0, 0, NAME, 1, OUT]), -43);
}

#[test]
fn the_default_volume_is_read_and_set_by_name() {
    let mut m = machine();
    assert_eq!(call(&mut m, "GetVol", &[BUF, OUT]), 0);
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap() as i16, -1);
    assert_eq!(m.mac.mem.read_u8(BUF).unwrap() as usize, "Macintosh HD".len());
    pstr(&mut m, NAME, "Macintosh HD");
    assert_eq!(call(&mut m, "SetVol", &[NAME, 0]), 0);
    assert_eq!(call(&mut m, "FlushVol", &[NAME, 0]), 0);
    pstr(&mut m, NAME, "No Such Disk:");
    assert_eq!(call(&mut m, "FlushVol", &[NAME, 0]), -35, "nsvErr");
}
