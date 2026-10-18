//! The small dos.library services driven by a compiled 68k program (tests/guest/c/dosextra.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/dosextra.bin");
const NEG1: u32 = u32::MAX;

#[test]
fn faults_numbers_dates_streams_and_the_one_cli() {
    let mut a = machine();
    a.fs.vol.put_file("SYS:Dir/f.txt", b"data".to_vec()).unwrap();
    let r = run(&mut a, PROG);
    assert_eq!((r[0], r[1]), (22, 1), "Fault: 'Copy: object not found' is 22 characters");
    assert_eq!((r[2], r[3]), (7, 1), "Fault truncates to the buffer and terminates");
    assert_eq!((r[4], r[5]), (0, 1), "code 0 is the empty string; an unknown code says Error N");
    assert_eq!((r[6], r[7]), (NEG1, 214), "PrintFault returns TRUE and leaves the code in IoErr");
    assert_eq!(String::from_utf8_lossy(&a.console.stdout), "Tool: disk is write-protected\n", "and it printed to the console");
    assert_eq!((r[8], r[9], r[10], r[11]), (6, (-123i32) as u32, NEG1, 0), "StrToLong: sign, blanks, characters used; no number is -1 and 0");
    assert_eq!((r[12], r[13], r[14], r[15], r[16], r[17]), (2, 1, 5, 1, NEG1, 1), "SplitName walks a/bc/d and ends with -1 on the last piece");
    assert_eq!((r[18], r[19], r[20], r[21]), (NEG1, 1, 1, 1), "DateToStr: time, day name, dd-mmm-yy, and mm-dd-yy for FORMAT_USA");
    assert_eq!((r[22], r[23]), (NEG1, 1), "StrToDate reads back exactly what DateToStr wrote");
    assert_eq!((r[24], r[25]), (0, 115), "a date that does not exist: FALSE, ERROR_BAD_NUMBER");
    assert_eq!((r[26], r[27], r[28], r[29], r[30]), (1, NEG1, 0, NEG1, 0), "ParentOfFH; WaitForChar on a file with data, on an empty console; SetMode console yes, file no");
    assert_eq!((r[31], r[32], r[33], r[34]), (0, 202, NEG1, 0), "ChangeMode to exclusive is refused while the file is open and granted once alone; then a second lock is refused");
    assert_eq!((r[35], r[36]), (1, 1), "SelectOutput redirects Output() and restores it");
    assert_eq!((r[37], r[38]), (1, 1), "GetArgStr / SetArgStr");
    assert_eq!((r[39], r[40], r[41]), (1, 1, 0), "one CLI; no pending signals");
    assert_eq!((r[43], r[44]), (NEG1, 212), "ReadLink on a path that is not a link");
    assert_eq!(r[45], NEG1, "record locks are granted");
}
