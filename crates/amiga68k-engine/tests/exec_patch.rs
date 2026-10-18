//! exec SetFunction on an engine library and the cache calls, driven by tests/guest/c/patch.c.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/patch.bin");

#[test]
fn a_patched_vector_runs_the_guests_function_and_the_old_one_stays_callable() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2], r[3]), (77, 1, 0x1234, 1), "SetFunction returns an old-function address; the patched vector runs the guest's function; LIBF_CHANGED is set");
    assert_eq!((r[4], r[5]), (1, 77), "putting the old address back returns our patch and restores the engine's behaviour");
    assert_eq!((r[6], r[7]), (78, 1), "a wrapper that chains to the returned old address reaches the host handler (77 + 1)");
    assert_eq!(r[8], 78, "refused SetFunction calls leave the table alone");
    assert_eq!((r[9], r[10], r[11], r[12], r[13], r[14]), (1, 1, 3, 3, 0, 0), "CacheControl returns the OLD bits: EnableI, then +FreezeI, cleared; clear bits are ignored");
    let refused: Vec<&str> = a.refused.iter().map(String::as_str).collect();
    assert_eq!(refused.len(), 3, "{refused:?}");
    assert!(refused[0].contains("SetFunction") && refused[0].contains("offset -131"), "{refused:?}");
    assert!(refused[1].contains("SetFunction") && refused[1].contains("offset 6"), "{refused:?}");
    assert!(refused[2].contains("SetFunction") && refused[2].contains("offset -60000"), "{refused:?}");
}
