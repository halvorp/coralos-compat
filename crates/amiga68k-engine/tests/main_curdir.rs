//! A CLI process has a current-directory LOCK. The main program's pr_CurrentDir (Process + 152) was 0, so the startup code that does
//! `CurrentDir(0)` and `DupLock(old)` (SAS/C's, SnoopDos 3.11's) got NULL, took it for a failure and printed "I need AT LEAST Kickstart 2.0".
mod gfxcommon;
use gfxcommon::*;

const CURRENT_DIR: i32 = -126;
const DUP_LOCK: i32 = -96;
const DOS_BASE: u32 = 0x5000;

#[test]
fn the_main_programs_current_directory_is_a_real_lock_that_duplicates() {
    let mut g = G::new();
    g.run(&[(DOS_BASE, CURRENT_DIR, vec![d(1, 0)]), (DOS_BASE, DUP_LOCK, vec![d_res(1, 0)]), (DOS_BASE, CURRENT_DIR, vec![d_res(1, 0)])]);
    let old = g.result(0);
    assert_ne!(old, 0, "CurrentDir(0) returns the old lock: the boot volume's root, not NULL");
    assert_ne!(g.result(1), 0, "DupLock(old) works");
    assert_eq!(g.result(2), 0, "CurrentDir(old) returns what the previous CurrentDir(0) set: 0");
    // NEGATIVE CONTROL: DupLock(NULL) is still NULL (rom/dos/duplock.c: `if (lock == BNULL) return BNULL`).
    g.run(&[(DOS_BASE, DUP_LOCK, vec![d(1, 0)])]);
    assert_eq!(g.result(0), 0);
}
