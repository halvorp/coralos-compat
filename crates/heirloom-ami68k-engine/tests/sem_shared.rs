//! Shared SignalSemaphore holds (tests/guest/c/sem_shared.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/sem_shared.bin");

#[test]
fn shared_holders_coexist_and_an_exclusive_obtain_waits_for_all_of_them() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    assert_eq!(r[0], 0, "ss_Owner is NULL while only shared holders exist");
    assert_eq!(r[1], 1, "a second task obtained it SHARED while the first still held it");
    assert_eq!((r[2], r[3]), (0, 0), "an exclusive obtain waits while shared holders remain");
    assert_eq!(r[4], 1, "and gets it when the last one releases");
    assert_eq!(a.refused.len(), 1, "{:?}", a.refused);
    assert!(a.refused[0].contains("ReleaseSemaphore") && a.refused[0].contains("not held"), "{:?}", a.refused);
}
