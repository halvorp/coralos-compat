//! Buffered I/O, DateStamp/Info and AssignLock, driven by a real 68k program (tests/guest/progmisc.s).
use heirloom_ami68k_engine::Amiga;
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_fmt_hunk::writer::{build, Hunk};

const PROG: &[u8] = include_bytes!("guest/progmisc.bin");
const BUF: u32 = 4 + 128;

#[test]
fn fgets_fgetc_ungetc_fread_dates_info_and_assignlock() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.put_file("SYS:x", b"1".to_vec()).unwrap();
    let r = a.run_hunk(&build(&[Hunk::code(PROG.to_vec(), vec![])], None), 500_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let base = r.loaded.bases[0];
    let v: Vec<u32> = (0..13).map(|i| a.mem.read_u32(base + 4 + 4 * i).unwrap()).collect();
    assert_eq!((v[0], v[1]), (97, 4), "FPutC returns the char; FWrite the block count");
    assert_eq!(v[2], base + BUF, "FGets returns its buffer");
    assert_eq!((v[3], v[4]), (120, 120), "UnGetC makes the next FGetC return the same byte");
    assert_eq!(v[5], 3, "FRead reports whole blocks actually read");
    assert_eq!(v[6], 0xFFFF_FFFF, "FGetC at EOF is -1");
    assert_eq!(v[7] as i32, heirloom_ami68k_engine::dos_misc::EPOCH_DAYS_2026, "DateStamp");
    assert_eq!(v[8], v[7], "a file written now carries today's date in its FileInfoBlock");
    assert_eq!((v[9], v[10], v[11]), (82, 512, 80), "Info: RAM: validated, SYS: write protected, 512-byte blocks");
    assert_ne!(v[12], 0, "AssignLock WK: -> RAM: then Open(WK:f) works");
    assert!(a.fs.vol.resolve(0, "RAM:f").is_ok(), "the file really landed in RAM:");
    assert_eq!(a.fs.vol.file_data(a.fs.vol.resolve(0, "RAM:tf").unwrap()).unwrap(), b"a\nxyz\n");
    assert!(a.fs.files.is_empty());
}
