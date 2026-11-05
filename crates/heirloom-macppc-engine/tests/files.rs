//! The File Manager through the PowerPC glue (`heirloom_macppc_engine::files`): the C-callable `Files.h` routines a PowerPC program imports from
//! InterfaceLib, called the way a PowerPC CPU calls them (r3.. = the C arguments), over the SAME volume the 68k engine serves.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

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

/// HGetVol / HSetVol (Files.h, the PBHGetVol / PBHSetVol glue): the default volume AND directory, read as (vRefNum, dirID) and set back.
/// The Photoshop 7 installer (Installer VISE) asks HGetVol first thing, to know the folder it was launched from.
#[test]
fn hgetvol_and_hsetvol_read_and_move_the_default_directory() {
    let mut m = machine();
    let root = m.mac.fm.def_vol;
    let folder = m.mac.fm.tree.mkdirs(root, ["Folder"].iter().copied()).unwrap();
    let folder_id = m.mac.fm.dir_id(folder);
    assert_eq!(call(&mut m, "HGetVol", &[0, OUT, OUT + 4]), 0);
    assert_eq!((m.mac.mem.read_u16(OUT).unwrap() as i16, m.mac.mem.read_u32(OUT + 4).unwrap()), (-1, 2), "the boot volume's root (fsRtDirID 2)");
    assert_eq!(call(&mut m, "HSetVol", &[0, (-1i16) as u16 as u32, folder_id]), 0);
    assert_eq!(call(&mut m, "HGetVol", &[BUF, OUT, OUT + 4]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT + 4).unwrap(), folder_id, "HSetVol moved the default directory");
    assert_eq!(m.mac.mem.read_u8(BUF).unwrap() as usize, "Macintosh HD".len(), "volName receives the default volume's name");
    // negative control: a directory that does not exist is refused and the default stays
    assert_ne!(call(&mut m, "HSetVol", &[0, (-1i16) as u16 as u32, 0x7777]), 0);
    assert_eq!(call(&mut m, "HGetVol", &[0, OUT, OUT + 4]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT + 4).unwrap(), folder_id);
}

/// The `(vRefNum, dirID, name, ...)` routines an installer leans on (Installer VISE, Photoshop 7): DirCreate answers the new folder's ID,
/// HCreate puts a file in it, HGetFInfo / HSetFInfo read and change its Finder info, HSetFLock makes a delete fail with fLckdErr until
/// HRstFLock, and HRename renames it in place. Each through the PB call a program could issue itself.
#[test]
fn dircreate_finfo_lock_and_rename_by_directory_id() {
    let mut m = machine();
    const FINFO: u32 = 0x3500;
    pstr(&mut m, NAME, "Adobe");
    assert_eq!(call(&mut m, "DirCreate", &[0, 2, NAME, OUT]), 0);
    let dir = m.mac.mem.read_u32(OUT).unwrap();
    assert!(dir > 2, "a new folder ID");
    assert_eq!(call(&mut m, "DirCreate", &[0, 2, NAME, OUT]), -48, "dupFNErr: the folder exists");
    pstr(&mut m, NAME, "Plug");
    assert_eq!(call(&mut m, "HCreate", &[0, dir, NAME, u32::from_be_bytes(*b"8BIM"), u32::from_be_bytes(*b"8BFM")]), 0);
    assert_eq!(call(&mut m, "HGetFInfo", &[0, dir, NAME, FINFO]), 0);
    assert_eq!(m.mac.mem.read_u32(FINFO).unwrap().to_be_bytes(), *b"8BFM", "fdType");
    assert_eq!(m.mac.mem.read_u32(FINFO + 4).unwrap().to_be_bytes(), *b"8BIM", "fdCreator");
    m.mac.mem.write_bytes(FINFO, b"TEXTttxt").unwrap();
    assert_eq!(call(&mut m, "HSetFInfo", &[0, dir, NAME, FINFO]), 0);
    m.mac.mem.write_bytes(FINFO, &[0; 8]).unwrap();
    assert_eq!(call(&mut m, "HGetFInfo", &[0, dir, NAME, FINFO]), 0);
    assert_eq!(m.mac.mem.read_u32(FINFO).unwrap().to_be_bytes(), *b"TEXT", "HSetFInfo wrote the folder's file, not another");
    assert_eq!(call(&mut m, "HSetFLock", &[0, dir, NAME, 0]), 0);
    assert_eq!(call(&mut m, "HDelete", &[0, dir, NAME]), -45, "fLckdErr: a locked file is not deleted");
    assert_eq!(call(&mut m, "HRstFLock", &[0, dir, NAME, 0]), 0);
    pstr(&mut m, BUF, "Plug-in");
    assert_eq!(call(&mut m, "HRename", &[0, dir, NAME, BUF]), 0);
    assert_eq!(call(&mut m, "HGetFInfo", &[0, dir, NAME, FINFO]), -43, "the old name is gone");
    assert_eq!(call(&mut m, "HGetFInfo", &[0, dir, BUF, FINFO]), 0);
    assert_eq!(call(&mut m, "HDelete", &[0, dir, BUF]), 0, "unlocked, it deletes");
    // NEGATIVE CONTROL: a folder ID that does not exist
    assert_eq!(call(&mut m, "HGetFInfo", &[0, 0x7777, BUF, FINFO]), -120, "dirNFErr");
}

/// PBGetFCBInfo fills FCBPBRec at the Files.h offsets (ioFCBIndx a SHORT at 28, ioFCBFlNm 32, ioFCBFlags 36, ioFCBEOF 40, ioFCBVRefNum 52,
/// ioFCBParID 58), and a zero ioFCBIndx with garbage in filler1 (30) still means "the file ioRefNum names". The Photoshop 7 installer finds its
/// own data fork this way; with the old offsets (34.. 60) it read vRef 0 and dirID garbage and stopped as "damaged".
#[test]
fn pbgetfcbinfo_answers_at_the_files_h_offsets() {
    let mut m = machine();
    const PB: u32 = 0x3600;
    let r = open(&mut m, "Data");
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u32(PB + 18, BUF).unwrap();
    m.mac.mem.write_u16(PB + 24, r as u16).unwrap();
    m.mac.mem.write_u16(PB + 30, 0xBEEF).unwrap(); // filler1: garbage a C program leaves on its stack
    assert_eq!(call(&mut m, "PBGetFCBInfoSync", &[PB]), 0);
    let node = m.mac.fm.locate(0, 0, Some(b"Data")).unwrap().node.unwrap();
    let rd16 = |m: &M, o: u32| m.mac.mem.read_u16(PB + o).unwrap() as i16;
    let rd32 = |m: &M, o: u32| m.mac.mem.read_u32(PB + o).unwrap();
    assert_eq!(rd32(&m, 32), m.mac.fm.file_id(node), "ioFCBFlNm");
    assert_eq!(rd32(&m, 40), 10, "ioFCBEOF");
    assert_eq!(rd16(&m, 52), -1, "ioFCBVRefNum: the boot volume");
    assert_eq!(rd32(&m, 58), 2, "ioFCBParID: the root");
    assert_eq!(m.mac.mem.read_u8(BUF).unwrap(), 4, "ioNamePtr receives \"Data\"");
    // indexed: ioFCBIndx 1 is the first open file
    m.mac.mem.write_u16(PB + 24, 0).unwrap();
    m.mac.mem.write_u16(PB + 28, 1).unwrap();
    assert_eq!(call(&mut m, "PBGetFCBInfoSync", &[PB]), 0);
    assert_eq!(rd16(&m, 24), r, "ioRefNum answered");
    // NEGATIVE CONTROL: index past the open files
    m.mac.mem.write_u16(PB + 28, 99).unwrap();
    assert_eq!(call(&mut m, "PBGetFCBInfoSync", &[PB]), -51);
}

/// PBHGetFInfo fills an HFileParam, which ENDS at ioFlMdDat (offset 80): nothing past it is written (the catalog fields ioFlBkDat .. ioFlClpSiz
/// are PBGetCatInfo's CInfoPBRec only). Writing them overran Installer VISE's 80-byte block into its stack locals; the Photoshop 7 install then
/// lost its destination folder and stopped "Could Not Find Application". POSITIVE CONTROL: PBGetCatInfo does write ioFlParID (100).
#[test]
fn pbhgetfinfo_writes_nothing_past_the_hfileparam() {
    let mut m = machine();
    const PB: u32 = 0x3700;
    m.mac.mem.zero(PB, 128).unwrap();
    pstr(&mut m, NAME, "Data");
    m.mac.mem.write_u32(PB + 18, NAME).unwrap();
    for o in (80..112).step_by(4) { m.mac.mem.write_u32(PB + o, 0xA5A5_A5A5).unwrap(); }
    assert_eq!(call(&mut m, "PBHGetFInfoSync", &[PB]), 0);
    assert_eq!(m.mac.mem.read_u32(PB + 54).unwrap(), 10, "ioFlLgLen is filled");
    for o in (80..112).step_by(4) { assert_eq!(m.mac.mem.read_u32(PB + o).unwrap(), 0xA5A5_A5A5, "offset {o} untouched"); }
    m.mac.mem.write_u32(PB + 48, 0).unwrap(); // the get answered the file number in ioDirID
    assert_eq!(call(&mut m, "PBGetCatInfoSync", &[PB]), 0);
    assert_eq!(m.mac.mem.read_u32(PB + 100).unwrap(), 2, "PBGetCatInfo: ioFlParID is the root");
}

/// PBHGetDirAccess / PBHSetDirAccess: no AppleShare privileges on a local volume (GetVolParms' bAccessCntl is clear): paramErr, nsvErr for a
/// volume that does not exist (the Photoshop 7 Tryout asks before writing its folder).
#[test]
fn dir_access_is_not_supported_on_a_local_volume() {
    let mut m = machine();
    const PB: u32 = 0x3700;
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u16(PB + 22, (-1i16) as u16).unwrap();
    assert_eq!(call(&mut m, "PBHGetDirAccessSync", &[PB]), -50);
    assert_eq!(call(&mut m, "PBHSetDirAccessSync", &[PB]), -50);
    m.mac.mem.write_u16(PB + 22, (-9i16) as u16).unwrap();
    assert_eq!(call(&mut m, "PBHGetDirAccessSync", &[PB]), -35, "NEGATIVE CONTROL: no such volume");
}
