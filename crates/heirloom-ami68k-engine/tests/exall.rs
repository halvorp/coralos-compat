//! ExAll driven by a compiled 68k program (tests/guest/c/exall.c), including a MatchFunc hook that is guest code.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/exall.bin");

#[test]
fn exall_levels_continuation_filters_hook_and_errors() {
    let mut a = machine();
    a.fs.vol.put_file("SYS:Dir/a.txt", b"abc".to_vec()).unwrap();
    a.fs.vol.put_file("SYS:Dir/b.txt", b"0123456789".to_vec()).unwrap();
    a.fs.vol.put_file("SYS:Dir/c.info", b"x".to_vec()).unwrap();
    a.fs.vol.mkdirs("SYS:Dir/Sub").unwrap();
    let f = a.fs.vol.resolve(0, "SYS:Dir/a.txt").unwrap();
    a.fs.vol.get_mut(f).comment = "hello".into();
    let r = run(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2], r[3], r[4]), (0, 232, 4, 377, 0), "ED_NAME: FALSE/ERROR_NO_MORE_ENTRIES with 4 records chained and terminated");
    assert_eq!((r[5], r[6], r[7], r[8]), (0, 1, 1, 1), "ED_COMMENT: name, type, size and comment of a.txt; Sub is a directory");
    assert_eq!((r[9], r[10], r[11]), (4, 377, 1), "a small buffer takes several calls through eac_LastKey and yields every entry exactly once");
    assert_eq!((r[12], r[13]), (0, 2), "eac_MatchString #?.txt keeps two entries");
    assert_eq!((r[14], r[15], r[16], r[17]), (0, 1, 4, 1), "the MatchFunc hook (guest code) is asked about all four and keeps only b.txt");
    assert_eq!((r[18], r[19]), (0, 303), "a buffer too small for one record: ERROR_BUFFER_OVERFLOW");
    assert_eq!((r[20], r[21]), (0, 115), "an unknown data level: ERROR_BAD_NUMBER");
    assert_eq!((r[22], r[23]), (0, 212), "ExAll on a file: ERROR_OBJECT_WRONG_TYPE");
    assert_eq!((r[24], r[25]), (1, 0), "ExAllEnd mid-scan resets eac_LastKey");
    assert!(a.fs.exall.is_empty(), "no scan state left behind");
    assert_eq!(a.calls_in_flight(), 0);
}
