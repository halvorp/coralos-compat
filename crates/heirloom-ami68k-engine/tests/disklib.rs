//! OpenLibrary of a library on disk: LoadSeg, RomTag, MakeLibrary/InitStruct, the init routine and the Open/Close
//! vectors all run for real, in the guest, on assembled 68k (tests/guest/*.s, rebuilt by tests/guest/build.sh).
use heirloom_ami68k_engine::machine::EXEC_BASE;
use heirloom_ami68k_engine::Amiga;
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_fmt_hunk::writer::{build, Hunk};

const TESTLIB: &[u8] = include_bytes!("guest/testlib.bin");
const TESTLIB_RELOCS: &str = include_str!("guest/testlib.relocs");
const PROG: &[u8] = include_bytes!("guest/prog.bin");

fn testlib_hunk() -> Vec<u8> {
    let relocs = TESTLIB_RELOCS.lines().map(|l| (0, l.trim().parse().unwrap())).collect();
    build(&[Hunk::code(TESTLIB.to_vec(), relocs)], None)
}

fn run_prog(a: &mut Amiga<RecordingGuestUi>) -> Vec<u32> {
    let r = a.run_hunk(&build(&[Hunk::code(PROG.to_vec(), vec![])], None), 200_000).unwrap();
    assert!(r.stats.halted, "the program ran to its end: {:?} {:?}", r.stats, a.refused);
    assert_eq!(r.stats.bus_faults, 0, "{:#?}", a.log);
    (0..14).map(|i| a.mem.read_u32(r.loaded.bases[0] + 4 + 4 * i).unwrap()).collect()
}

#[test]
fn a_disk_library_loads_initialises_opens_works_and_closes() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.put_file("LIBS:test.library", testlib_hunk()).unwrap();
    let r = run_prog(&mut a);
    assert!(a.refused.is_empty(), "{:?}\n{:#?}", a.refused, a.log);
    assert_ne!(r[0], 0, "OpenLibrary returned the library");
    assert_eq!([r[1], r[2], r[3]], [0xD2D2D2D2, 0xD3D3D3D3, 0xA3A3A3A3], "the init routine trashed D3/A3; the caller's registers came back");
    assert_eq!(r[4], EXEC_BASE, "A6 is ExecBase again");
    assert_eq!(r[5], 11, "the library's own function ran through its JMP table");
    let lib = &a.fs.disk_libs[0];
    assert_eq!((r[6], r[0]), (lib.seglist, lib.base), "the init routine got the seglist; OpenLibrary returned the base");
    assert_eq!(r[7], heirloom_ami68k_engine::machine::DOS_BASE, "a bridged OpenLibrary worked from inside the guest init routine");
    assert_eq!(r[8], 3, "lib_Version came from the InitStruct table");
    assert_eq!((r[9], r[10]), (1, 0), "the Open vector counted, the Close vector uncounted");
    assert_eq!(r[11], 0, "version 99 asked for, 3 offered: NULL");
    assert_eq!(r[12], 0, "no such library anywhere: NULL");
    assert_eq!(r[13], r[0], "the second open reuses the loaded library");
    assert_eq!(a.fs.disk_libs.len(), 1, "loaded once");
    assert_eq!(a.fs.disk_libs[0].name, "test.library");
    assert_eq!(a.mem.read_u16(r[0] + 18).unwrap(), 42, "lib_PosSize from the init table");
    assert_eq!(a.mem.read_u16(r[0] - 6).unwrap(), 0x4EF9, "a JMP table sits below the base");
    assert!(a.calls_in_flight() == 0, "every guest call finished");
}

#[test]
fn without_the_file_the_open_is_a_plain_null() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let r = run_prog(&mut a); // negative control: same program, empty LIBS:
    assert_eq!(r[0], 0);
    assert!(a.refused.is_empty(), "an absent library is not a refusal: {:?}", a.refused);
    assert!(a.fs.disk_libs.is_empty());
}

#[test]
fn a_file_that_is_not_a_library_is_a_named_refusal_and_null() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    // A valid hunk executable with no RomTag.
    a.fs.vol.put_file("LIBS:test.library", build(&[Hunk::code(vec![0x70, 0xFF, 0x4E, 0x75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], vec![])], None)).unwrap();
    let heap = a.heap.live_blocks();
    let r = run_prog(&mut a);
    assert_eq!(r[0], 0);
    assert!(a.refused.iter().any(|x| x.contains("has no RomTag")), "{:?}", a.refused);
    assert_eq!(a.heap.live_blocks(), heap + 0, "the failed load gave its segments back");
    // A file that is not a hunk at all.
    let mut b = Amiga::new(RecordingGuestUi::new()).unwrap();
    b.fs.vol.put_file("LIBS:test.library", b"plain text".to_vec()).unwrap();
    assert_eq!(run_prog(&mut b)[0], 0);
    assert!(b.refused.iter().any(|x| x.contains("LoadSeg") && x.contains("IoErr 121")), "{:?}", b.refused);
}

#[test]
fn an_init_routine_that_returns_null_discards_the_library() {
    let mut lib = TESTLIB.to_vec();
    // initroutine's `move.l %d2,%d0` (2400) just before its movem restore -> `moveq #0,%d0; nop`
    let at = lib.windows(2).position(|w| w == [0x20, 0x02]).expect("move.l %d2,%d0");
    lib[at..at + 2].copy_from_slice(&[0x70, 0x00]);
    let relocs = TESTLIB_RELOCS.lines().map(|l| (0, l.trim().parse().unwrap())).collect();
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.put_file("LIBS:test.library", build(&[Hunk::code(lib, relocs)], None)).unwrap();
    let heap = a.heap.live_blocks();
    let r = run_prog(&mut a);
    assert_eq!(r[0], 0);
    assert!(a.refused.iter().any(|x| x.contains("init routine returned NULL")), "{:?}", a.refused);
    assert!(a.fs.disk_libs.is_empty());
    assert_eq!(a.heap.live_blocks(), heap, "library memory and segments were given back");
    assert_eq!([r[1], r[2], r[3]], [0xD2D2D2D2, 0xD3D3D3D3, 0xA3A3A3A3], "registers restored on the failure path too");
}

#[test]
fn a_library_that_gives_every_opener_its_own_base_is_closed_through_that_base() {
    // AROS rellibs (stdc.library, posixc.library) build a base per opener with MakeLibrary in their Open vector; the program
    // closes THAT base. Before disklib remembered it, every AROS program ended with "CloseLibrary: 0x... is not an open library base".
    use heirloom_fmt_hunk::writer::{build, Hunk};
    let relocs = include_str!("guest/perlib.relocs").lines().map(|l| (0, l.trim().parse().unwrap())).collect();
    let lib = build(&[Hunk::code(include_bytes!("guest/perlib.bin").to_vec(), relocs)], None);
    let prog = build(&[Hunk::code(include_bytes!("guest/perlibprog.bin").to_vec(), vec![])], None);
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.put_file("LIBS:per.library", lib.clone()).unwrap();
    let r = a.run_hunk(&prog, 500_000).unwrap();
    assert!(r.stats.halted, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    let res: Vec<u32> = (0..3).map(|i| a.mem.read_u32(0x1F000 + 4 * i).unwrap()).collect();
    assert_eq!(res, [1, 12, 1], "opened, the opener's own function table works (7 + 5), and the close was accepted");
    assert_eq!(a.fs.disk_libs[0].openers.len(), 1, "the per-opener base was recorded");
    assert_ne!(a.fs.disk_libs[0].openers[0], a.fs.disk_libs[0].base);
    assert_eq!(a.fs.disk_libs[0].open_count, 0, "its Close vector ran");
    // NEGATIVE CONTROL (in the guest): CloseLibrary of lib+6, which is no base, is the ONE refusal, by name.
    assert_eq!(a.refused.len(), 1, "{:?}", a.refused);
    assert!(a.refused[0].contains("is not an open library base"), "{:?}", a.refused);
}

#[test]
fn autoinit_library_without_datainit_inherits_romtag_version_name_and_idstring() {
    // Build a minimal RTF_AUTOINIT disk library with dataInit == 0 and rt_Version == 44 (like AROS colorwheel.gadget).
    // Layout:
    //   0x00: open_fn  = move.l a6,d0 (0x200E); rts (0x4E75)
    //   0x04: close_fn = moveq #0,d0  (0x7000); rts (0x4E75)
    //   0x08: func_tbl = [0x00, 0x04, 0x04, 0x04, 0xFFFFFFFF] (relocs at 0x08..0x14)
    //   0x1C: init_tbl = [dataSize=34, funcs=0x08, dataInit=0, initRoutine=0] (reloc at 0x20)
    //   0x2C: RomTag   = matchword 0x4AFC, matchtag=0x2C, endskip=0x46, flags=0x80 (RTF_AUTOINIT), ver=44, type=9, pri=0, name=0x48, id=0x54, init=0x1C
    //   0x48: "ver.library\0" (12 bytes)
    //   0x54: "ver 44.1\0"    (9 bytes + pad)
    let mut code = vec![0u8; 0x64];
    let w16 = |b: &mut [u8], o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
    let w32 = |b: &mut [u8], o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_be_bytes());
    w16(&mut code, 0x00, 0x200E); w16(&mut code, 0x02, 0x4E75);
    w16(&mut code, 0x04, 0x7000); w16(&mut code, 0x06, 0x4E75);
    w32(&mut code, 0x08, 0x00); w32(&mut code, 0x0C, 0x04); w32(&mut code, 0x10, 0x04); w32(&mut code, 0x14, 0x04); w32(&mut code, 0x18, 0xFFFF_FFFF);
    w32(&mut code, 0x1C, 34); w32(&mut code, 0x20, 0x08); w32(&mut code, 0x24, 0); w32(&mut code, 0x28, 0);
    w16(&mut code, 0x2C, 0x4AFC); w32(&mut code, 0x2E, 0x2C); w32(&mut code, 0x32, 0x46);
    code[0x36] = 0x80; // RTF_AUTOINIT
    code[0x37] = 44;   // rt_Version
    code[0x38] = 9;    // NT_LIBRARY
    code[0x39] = 0;    // rt_Pri
    w32(&mut code, 0x3A, 0x48); // rt_Name
    w32(&mut code, 0x3E, 0x54); // rt_IdString
    w32(&mut code, 0x42, 0x1C); // rt_Init
    code[0x48..0x54].copy_from_slice(b"ver.library\0");
    code[0x54..0x5D].copy_from_slice(b"ver 44.1\0");
    let relocs = vec![(0, 0x08), (0, 0x0C), (0, 0x10), (0, 0x14), (0, 0x20), (0, 0x2E), (0, 0x32), (0, 0x3A), (0, 0x3E), (0, 0x42)];
    let lib_hunk = build(&[Hunk::code(code, relocs)], None);

    // Tiny caller: OpenLibrary("ver.library", 39) -> store at 0x1F000; OpenLibrary("ver.library", 45) -> store at 0x1F004; rts.
    let mut p = Vec::new();
    let emit = |p: &mut Vec<u8>, w: &[u16]| { for &x in w { p.extend_from_slice(&x.to_be_bytes()); } };
    emit(&mut p, &[
        0x2C78, 0x0004,             // movea.l 4.w, a6
        0x43FA, 0x0022,             // lea name(pc), a1 (pc = 0x06 -> 0x28)
        0x7027,                     // moveq #39, d0
        0x4EAE, (-552i16) as u16,   // jsr _LVOOpenLibrary(a6)
        0x23C0, 0x0001, 0xF000,     // move.l d0, 0x1F000
        0x43FA, 0x0012,             // lea name(pc), a1 (pc = 0x16 -> 0x28)
        0x702D,                     // moveq #45, d0
        0x4EAE, (-552i16) as u16,   // jsr _LVOOpenLibrary(a6)
        0x23C0, 0x0001, 0xF004,     // move.l d0, 0x1F004
        0x7000,                     // moveq #0, d0
        0x4E75,                     // rts
    ]);
    assert_eq!(p.len(), 0x28);
    p.extend_from_slice(b"ver.library\0");
    let prog = build(&[Hunk::code(p, vec![])], None);

    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.put_file("LIBS:ver.library", lib_hunk).unwrap();
    let r = a.run_hunk(&prog, 200_000).unwrap();
    assert!(r.stats.halted, "{:?} {:?}", r.stats, a.refused);
    let ok_base = a.mem.read_u32(0x1F000).unwrap();
    let too_new = a.mem.read_u32(0x1F004).unwrap();
    assert_ne!(ok_base, 0, "OpenLibrary(ver=39) must succeed for rt_Version=44 library: {:?}", a.log);
    assert_eq!(too_new, 0, "NEGATIVE CONTROL: OpenLibrary(ver=45) must return NULL for rt_Version=44 library");
    assert_eq!(a.mem.read_u16(ok_base + 20).unwrap(), 44, "lib_Version seeded from rt_Version");
    assert_eq!(a.mem.read_u8(ok_base + 8).unwrap(), 9, "ln_Type seeded from rt_Type (NT_LIBRARY)");
    let id_ptr = a.mem.read_u32(ok_base + 24).unwrap();
    assert_eq!(a.mem.read_cstr(id_ptr).unwrap(), "ver 44.1");
}
