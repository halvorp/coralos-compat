//! exec RawDoFmt (with the classic `move.b d0,(a3)+` PutChProc) and dos VPrintf, driven by tests/guest/c/rawdofmt.c.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/rawdofmt.bin");

#[test]
fn rawdofmt_formats_through_a_guest_proc_that_keeps_a3_between_calls() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    assert_eq!((r[0], r[1]), (1, 32), "all of %d %ld %lx %s %c with widths, left, zero fill and .limit; returns the end of the data used");
    assert_eq!((r[2], r[3]), (1, 1), "negative control: a template with no directive uses no data");
    assert_eq!(r[4], 8, "VPrintf returns the number of characters");
    assert_eq!(a.console.stdout, b"7=seven\n", "... and wrote them to Output()");
    assert_eq!(a.refused.len(), 1, "{:?}", a.refused);
    assert!(a.refused[0].starts_with("exec.library/RawDoFmt:") && a.refused[0].contains("PutChProc is NULL"), "{:?}", a.refused);
}
