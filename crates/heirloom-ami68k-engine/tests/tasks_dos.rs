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

#[test]
fn detached_create_new_proc_continues_after_main_returns() {
    // Main spawns a detached dos.library Process via CreateNewProc(NP_Entry = child) and immediately returns (RTS).
    // The child must continue running and write 0xCAFE_BABE into RES[0] (0x1F000) before returning.
    // (Negative control for plain AddTask workers lives in tests/tasks.rs.)
    let mut code: Vec<u8> = Vec::new();
    let emit = |c: &mut Vec<u8>, w: u16| c.extend_from_slice(&w.to_be_bytes());
    emit(&mut code, 0x2C78); emit(&mut code, 0x0004); // 0:  MOVE.L (4).W, A6          (SysBase)
    let lea_dos_pos = code.len();
    emit(&mut code, 0x43FA); emit(&mut code, 0x0000); // 4:  LEA dosname(PC), A1       (patched below)
    emit(&mut code, 0x7000);                          // 8:  MOVEQ #0, D0
    emit(&mut code, 0x4EAE); emit(&mut code, 0xFDD8); // 10: JSR -552(A6)              (OpenLibrary)
    emit(&mut code, 0x2C40);                          // 14: MOVE.L D0, A6             (DOSBase)
    let lea_child_pos = code.len();
    emit(&mut code, 0x41FA); emit(&mut code, 0x0000); // 16: LEA child(PC), A0         (patched below)
    emit(&mut code, 0x42A7);                          // 20: CLR.L -(SP)               (TAG_DONE)
    emit(&mut code, 0x2F08);                          // 22: MOVE.L A0, -(SP)          (child entry)
    emit(&mut code, 0x2F3C); emit(&mut code, 0x8000); emit(&mut code, 0x03EB); // 24: MOVE.L #NP_ENTRY, -(SP)
    emit(&mut code, 0x220F);                          // 30: MOVE.L SP, D1
    emit(&mut code, 0x4EAE); emit(&mut code, 0xFE0E); // 32: JSR -498(A6)              (CreateNewProc)
    emit(&mut code, 0x4FEF); emit(&mut code, 0x000C); // 36: LEA 12(SP), SP
    emit(&mut code, 0x7000);                          // 40: MOVEQ #0, D0
    emit(&mut code, 0x4E75);                          // 42: RTS                       (main exits!)
    let child_off = code.len();
    emit(&mut code, 0x23FC); emit(&mut code, 0xCAFE); emit(&mut code, 0xBABE); emit(&mut code, 0x0001); emit(&mut code, 0xF000); // MOVE.L #$CAFEBABE, ($1F000).L
    emit(&mut code, 0x4E75);                          // RTS                           (detached process exits)
    let dos_off = code.len();
    code.extend_from_slice(b"dos.library\0");
    // Patch PC-relative displacements (PC points to extension word = lea_pos + 2)
    let d_dos = (dos_off as i16) - (lea_dos_pos as i16 + 2);
    code[lea_dos_pos + 2..lea_dos_pos + 4].copy_from_slice(&d_dos.to_be_bytes());
    let d_child = (child_off as i16) - (lea_child_pos as i16 + 2);
    code[lea_child_pos + 2..lea_child_pos + 4].copy_from_slice(&d_child.to_be_bytes());

    let mut a = machine();
    let r = run(&mut a, &code);
    assert_eq!(r[0], 0xCAFE_BABE, "detached CreateNewProc child must run after main exits; log={:#?}", a.log);
    assert!(a.log.iter().any(|l| l.contains("main exited; detached process continues")), "{:#?}", a.log);
}
