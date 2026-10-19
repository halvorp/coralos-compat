//! keymap.library, exec RawPutChar, aros.library ArosInquireA, commodities BrokerCommand and the console's DupLockFromFH, driven through
//! the library ABI by a compiled 68k program (tests/guest/c/libs_misc.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/libs_misc.bin");

#[test]
fn the_small_libraries_answer_as_their_originals_do() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    let i = |k: usize| r[k] as i32;
    // keymap.library
    assert_eq!((r[0], r[1]), (1, 1), "AskKeyMapDefault returns the one built-in KeyMap, every time");
    assert_eq!((r[2], r[3], r[4]), (1, b'a' as u32, b'A' as u32), "MapRawKey: 'a', shift 'A'");
    assert_eq!((r[5], r[6]), (3, b'0' as u32), "F1 types three bytes (CSI 0 ~): the second is '0'");
    assert_eq!(i(7), -1, "a buffer too small is -1");
    assert_eq!(r[8], 3, "MapANSI(\"Hi!\") = three (code, qualifier) pairs");
    assert_eq!(r[9], 0x0017_0125, "pairs (code, qualifier): shift+h (0x25, shift), then i (0x17, none)");
    assert_eq!(r[20], 0, "no key types '|' in the shipped layout: MapANSI answers 0");
    assert_eq!((r[21], r[10], r[12], r[11]), (1, b'z' as u32, b'a' as u32, b'a' as u32), "SetKeyMapDefault swaps the default ('z'), an explicit keymap wins ('a'), and putting the original back restores 'a'");
    // aros.library
    assert_eq!((r[13], r[14], r[15], r[16], r[17]), (3, 41, 1, 0xDEAD, 0xFFFF_FFFF), "ArosInquireA: version 41, release 1, ABI -1; the Kickstart base is not answered: untouched, and its index (3) is the result");
    // DupLockFromFH on the console
    assert_eq!((r[18], r[19]), (0, 209), "the console handler has no directory: NULL with ERROR_ACTION_NOT_KNOWN");
    // commodities
    assert_eq!((i(22), r[23]), (-1, 1), "BrokerCommand(NULL) names Exchange, which does not run here: -1; CxBroker works");
    assert_eq!((i(24), r[25], r[26], r[27], r[28]), (0, 1, 64, 15, 0), "a command to a broker with a port: a CxMsg of type CXM_COMMAND, ID = the command, no input event");
    assert_eq!((i(29), i(30), i(31), r[32]), (-1, -2, 0, 1), "an unknown broker -1; a broker without a port -2; CXCMD_KILL then signals its task (CTRL-E)");
    // exec RawPutChar goes to the engine log, never to the program's stdout.
    assert!(a.log.iter().any(|l| l == "[serial] ser"), "{:?}", a.log.iter().filter(|l| l.contains("serial")).collect::<Vec<_>>());
    assert!(a.console.stdout.is_empty() && a.console.serial.is_empty(), "the line was completed; nothing reached stdout");
    assert!(a.refused.len() == 1 && a.refused[0].starts_with("aros.library/ArosInquireA:") && a.refused[0].contains("AI_KickstartBase"), "{:?}", a.refused);
}
