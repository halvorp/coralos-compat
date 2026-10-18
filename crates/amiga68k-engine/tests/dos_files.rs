//! dos.library on the volume, driven by a real 68k program (tests/guest/progdos.s): locks, Examine/ExNext, Open,
//! Read, Write, Seek, DeleteFile, IoErr codes, NameFromLock, FilePart/PathPart.
use amiga68k_engine::Amiga;
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

const PROG: &[u8] = include_bytes!("guest/progdos.bin");
const RES: u32 = 4;
const BUF: u32 = 4 + 256 + 264;
const NAME: u32 = BUF + 64;

fn run(a: &mut Amiga<RecordingGuestUi>) -> (Vec<u32>, u32) {
    let r = a.run_hunk(&build(&[Hunk::code(PROG.to_vec(), vec![])], None), 500_000).unwrap();
    assert!(r.stats.halted, "ran to the end: {:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    assert_eq!(r.stats.bus_faults, 0, "{:#?}", a.log);
    let base = r.loaded.bases[0];
    ((0..24).map(|i| a.mem.read_u32(base + RES + 4 * i).unwrap()).collect(), base)
}

fn machine() -> Amiga<RecordingGuestUi> {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.put_file("LIBS:foo.library", b"abc".to_vec()).unwrap();
    a.fs.vol.put_file("LIBS:mui/Bar.mcc", b"x".to_vec()).unwrap();
    a
}

#[test]
fn a_program_walks_reads_writes_and_deletes_files_with_the_right_errors() {
    let mut a = machine();
    let (r, base) = run(&mut a);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_ne!(r[0], 0, "[0] Lock of an existing file");
    assert_eq!((r[1], r[2]), (0, 205), "[1,2] Lock of a missing file fails with ERROR_OBJECT_NOT_FOUND");
    assert_eq!((r[3], r[4], r[5] as i32), (0xFFFF_FFFF, 3, -3), "[3-5] Examine: DOSTRUE, size 3, ST_FILE");
    assert_eq!((r[6], r[7]), (2, 232), "[6,7] ExNext saw foo.library and mui, then ERROR_NO_MORE_ENTRIES");
    assert_ne!(r[8], 0, "[8] Open relative to CurrentDir");
    assert_eq!(r[9], 3, "[9] Read returned the 3 bytes");
    assert_eq!(a.mem.read_cstr(base + BUF).unwrap(), "hello", "the buffer ends holding what the RAM: read returned");
    assert_eq!((r[10], r[11]), (3, 2), "[10,11] Seek returns the old position; the next Read gets 2");
    assert_eq!((r[12], r[13]), (0xFFFF_FFFF, 223), "[12,13] Write on an OLDFILE handle: -1, ERROR_WRITE_PROTECTED");
    assert_eq!((r[14], r[15]), (5, 5), "[14,15] RAM: round trip");
    assert_eq!(r[16], 0xFFFF_FFFF, "[16] DeleteFile");
    assert_eq!((r[17], r[18]), (0, 205), "[17,18] the deleted file is gone");
    assert_eq!((r[19], r[20]), (0, 214), "[19,20] NEWFILE on read-only SYS: fails with ERROR_DISK_WRITE_PROTECTED");
    assert_eq!(r[21], 0xFFFF_FFFF);
    assert_eq!(a.mem.read_cstr(base + NAME).unwrap(), "SYS:Libs", "[21] NameFromLock");
    assert_eq!((r[22], r[23]), (8, 7), "[22,23] FilePart / PathPart offsets into xxx:yyy/zzz");
    assert!(a.fs.locks.is_empty() || a.fs.locks.len() == 1, "only the Lock([0]) the program kept: {}", a.fs.locks.len());
    assert!(a.fs.files.is_empty(), "every handle closed");
    assert!(a.fs.vol.resolve(0, "RAM:test").is_err());
}

#[test]
fn the_same_program_on_an_empty_volume_sees_nothing() {
    // Negative control: without the files the Lock fails, nothing is enumerated, nothing is read.
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let (r, _) = run(&mut a);
    assert_eq!((r[0], r[6], r[9]), (0, 0, 0), "{r:?}");
}
