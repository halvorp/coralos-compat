//! Shared harness for the C guest fixtures (tests/guest/c/*.c, built by tests/guest/build-c.sh).
#![allow(dead_code)]
use amiga68k_engine::Amiga;
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

/// Where the fixtures' `RES[]` array lives in guest RAM (tests/guest/c/rt.h).
pub const RES: u32 = 0x1F000;

pub fn machine() -> Amiga<RecordingGuestUi> { Amiga::new(RecordingGuestUi::new()).unwrap() }

/// Run a fixture blob to its end; fails with the machine's log when it faults, hangs or hits a refusal.
pub fn run(a: &mut Amiga<RecordingGuestUi>, blob: &[u8]) -> Vec<u32> {
    let r = a.run_hunk(&build(&[Hunk::code(blob.to_vec(), vec![])], None), 5_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    (0..128).map(|i| a.mem.read_u32(RES + 4 * i).unwrap()).collect()
}

/// Like [`run`] but tolerates named refusals (returned); for tests that expect one.
pub fn run_refusing(a: &mut Amiga<RecordingGuestUi>, blob: &[u8]) -> Vec<u32> {
    let r = a.run_hunk(&build(&[Hunk::code(blob.to_vec(), vec![])], None), 5_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    (0..128).map(|i| a.mem.read_u32(RES + 4 * i).unwrap()).collect()
}
pub mod gtscript;
