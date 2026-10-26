//! dos process creation (tests/guest/c/tasks_dos.c): CreateProc / CreateNewProc over a LoadSeg'd program, the NP_* ownership
//! flags, NP_ExitCode, Exit, killing a process, and the refusals for what is not modelled.
mod common;
use common::*;
use heirloom_fmt_hunk::writer::{build, Hunk};

const DOS: &[u8] = include_bytes!("guest/tasks_dos.bin");
const KID: &[u8] = include_bytes!("guest/tasks_kid.bin");

#[test]
fn processes_made_from_seglists_own_and_release_what_their_tags_say() {
    let mut a = machine();
    a.fs.vol.put_file("SYS:kid", build(&[Hunk::code(KID.to_vec(), vec![])], None)).unwrap();
    // The locks the engine itself holds once a program runs (its DOS list): measured on a program that does nothing.
    let mut idle = machine();
    idle.run_hunk(&build(&[Hunk::code(vec![0x4E, 0x75], vec![])], None), 100).unwrap();
    let locks_before = idle.fs.locks.len();
    let r = run_refusing(&mut a, DOS);
    assert_eq!(&r[0..9], &[1, 1, 1, 1, 1, 0xFFFF_FFFF, 1, 1, 1], "LoadSeg + CreateProc run the kid (port = its pr_MsgPort); UnLoadSeg works afterwards (CreateProc keeps the seglist); pr_SegList holds the seglist");
    assert_eq!((r[9], r[10]), (42, 0xFEED), "NP_ExitCode gets the return code in D0 and NP_ExitData in D1");
    assert_eq!((r[11], r[12], r[13]), (7, 0xFACE, 0), "dos Exit(7) ends the process on the spot, the exit code still runs");
    assert_eq!(r[14], 1, "NP_CurrentDir left out duplicates the parent's current directory");
    assert_eq!((r[15], r[16]), (2, 1), "the child wrote through NP_Output");
    assert_eq!((r[21], r[22]), (0, 1), "a process in DoIO(timer.device) sleeps while the parent waits");
    assert_eq!(a.clock_us, 2_000_000, "... for exactly the 2 s of virtual time it asked for");
    assert_eq!(r[17], 1, "RemTask on a process gives back every byte the engine allocated for it");
    assert_eq!(r[18], 1, "... and it is gone and never ran past its Wait");
    assert_eq!((r[19], r[20]), (0, 0), "negative controls: a seglist LoadSeg never returned, and NP_Synchronous, fail");
    assert!(a.fs.segs.is_empty(), "the NP_Seglist of kid2 was freed when it ended (NP_FreeSeglist defaults TRUE)");
    assert!(a.fs.files.is_empty(), "NP_Output was closed by the process (NP_CloseOutput defaults TRUE); the parent closed the rest");
    assert_eq!(a.fs.locks.len(), locks_before, "the child's duplicated current directory was unlocked at its exit");
    assert_eq!(a.refused.len(), 2, "{:?}", a.refused);
    assert!(a.refused[0].starts_with("dos.library/CreateNewProc:") && a.refused[0].contains("not a seglist LoadSeg returned"), "{:?}", a.refused);
    assert!(a.refused[1].starts_with("dos.library/CreateNewProc:") && a.refused[1].contains("NP_Synchronous is not modelled"), "{:?}", a.refused);
}
