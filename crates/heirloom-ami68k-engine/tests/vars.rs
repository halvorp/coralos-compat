//! dos.library variables driven by a compiled 68k program (tests/guest/c/vars.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/vars.bin");
const NEG1: u32 = u32::MAX;

#[test]
fn local_alias_and_global_variables_follow_the_flags() {
    let mut a = machine();
    let r = run(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2], r[3]), (NEG1, 3, 1, 7), "SetVar ok; text GetVar stops at the newline; IoErr is the full length");
    assert_eq!((r[4], r[5]), (7, 1), "GVF_BINARY_VAR keeps the newline and the rest");
    assert_eq!((r[6], r[7], r[8]), (NEG1, 303, 1), "a value that does not fit: -1, ERROR_BUFFER_OVERFLOW, truncated + terminated buffer");
    assert_eq!((r[9], r[10]), (NEG1, 205), "an undefined variable");
    assert_eq!((r[11], r[12], r[13], r[14]), (NEG1, 4, 1, NEG1), "aliases are a separate namespace from variables");
    assert_eq!((r[15], r[16], r[17], r[18]), (NEG1, 4, 1, NEG1), "a global lives in ENV:, and GVF_LOCAL_ONLY does not see it");
    assert_eq!((r[19], r[20], r[21]), (NEG1, 1, 1), "a local shadows the global; GVF_GLOBAL_ONLY still reaches it");
    assert_eq!((r[22], r[23], r[24], r[25]), (1, 1, 1, 0), "FindVar returns the real LocalVar node: type, name, value, length");
    assert_eq!((r[34], &r[26..29]), (3, &[b'F' as u32, b'G' as u32, b'l' as u32][..]), "pr_LocalVars lists them alphabetically, ignoring case");
    assert_eq!((r[35], r[36], r[37], r[38]), (NEG1, NEG1, 0, 205), "DeleteVar once works, twice is FALSE with 205");
    assert_eq!((r[39], r[40]), (NEG1, NEG1), "DeleteVar GLOBAL_ONLY removes ENV:G, and GetVar does not fall back to the ENVARC: copy");
    assert_eq!((r[41], r[42]), (NEG1, 115), "size 0 is ERROR_BAD_NUMBER");
    assert!(a.fs.vol.resolve(0, "ENV:G").is_err(), "ENV:G was deleted");
    let saved = a.fs.vol.resolve(0, "ENVARC:G").unwrap();
    assert_eq!(a.fs.vol.file_data(saved).unwrap(), b"glob", "GVF_SAVE_VAR also wrote the ENVARC: copy");
}
