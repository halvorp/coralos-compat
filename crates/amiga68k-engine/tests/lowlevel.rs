//! lowlevel.library driven by a compiled 68k program (tests/guest/c/lowlevel.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/lowlevel.bin");

#[test]
fn lowlevel_reports_no_ports_no_key_and_clears_exactly_the_queried_keys() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    assert_eq!(a.refused, ["lowlevel.library/AddKBInt: unimplemented"], "the one call nothing bridges is refused by name, and only it");
    assert_ne!(r[0], 0, "OpenLibrary(lowlevel.library) works");
    assert_eq!(r[1], 0, "ReadJoyPort: JP_TYPE_NOTAVAIL");
    assert_eq!(r[2], 1, "GetLanguageSelection: LANG_AMERICAN");
    assert_eq!(r[3], 0, "GetKey: nothing waiting");
    assert_eq!(&r[4..12], &[0x45, 0, 0x46, 0, 0x47, 0, 0x48, 0xFFFF], "QueryKeys clears pressed for 3 entries; the 4th (negative control) keeps 0xFFFF");
}
