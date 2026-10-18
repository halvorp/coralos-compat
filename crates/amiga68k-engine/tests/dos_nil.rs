//! dos.library's NIL: device and Close(0), driven by a compiled 68k program (tests/guest/c/dosnil.c). The corpus found both
//! blocking ~30 AROS programs: the stdcio/posixc startup opens NIL: and its cleanup closes a NULL handle.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/dosnil.bin");
const TRUE: u32 = u32::MAX;

#[test]
fn nil_reads_eof_discards_writes_and_close_of_null_succeeds() {
    let mut a = machine();
    let r = run(&mut a, PROG);
    assert_eq!(r[0], 1, "Open(NIL:, OLDFILE) gives a handle");
    assert_eq!(r[1], 0, "reading NIL: is EOF at once");
    assert_eq!(r[2], 1, "Open(nil:, NEWFILE) is another handle (the name is case-insensitive)");
    assert_eq!(r[3], 5, "a write to NIL: reports every byte written");
    assert_eq!(r[4], 0, "... and the bytes are gone");
    assert_eq!((r[5], r[6]), (TRUE, TRUE), "Close on a NIL: handle is DOSTRUE");
    assert_eq!(r[7], TRUE, "Close(0) is DOSTRUE");
    assert_eq!(r[8], 0, "CONTROL: a device that does not exist is still a failed Open");
    assert_eq!(r[9], 1, "the program ran to its end");
    assert!(a.fs.files.is_empty(), "no handle leaks");
}
