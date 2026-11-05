//! A fat loader, the way Installer VISE's `FatLoader` starts the Photoshop 7 installer: the application fragment is a tiny stub whose
//! INIT routine reads the `CFragInitBlock` the Code Fragment Manager hands it (its data-fork locator: the application file's FSSpec,
//! the stub's own offset and length), calls `GetDiskFragment` on the SAME file for the real code, and whose main jumps through the
//! `mainAddr` that call returned. Both halves are checked: the init block's fields, and the real fragment's main running.
use heirloom_common::{GuestMachine, Step};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::writer::PefBuilder;
use heirloom_fmt_resfork::writer::{macbinary, ForkBuilder};
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::{cfrg, MacPpc};
use heirloom_ui_abi::RecordingGuestUi;

/// Where the fixtures leave their evidence (absolute guest addresses, below the heap).
const MARK: u32 = 0x3000;
const BLOCK_AT: u32 = 0x3004;

/// The real code: main stores 0x77 at MARK and returns 0.
fn real() -> Vec<u8> {
    let mut b = PefBuilder::new();
    b.code(&{ let mut c = Asm::new(); c.li(4, 0x77).stw(4, MARK as i32, 0).li(3, 0).blr(); c.finish() });
    let d = b.data(&[0u8; 16], 16);
    b.reloc(d as u16, &[0x4600]);
    b.main(d as i32, 0);
    b.build()
}

/// The stub: data = main TVector 0, init TVector 8, GetDiskFragment's TVector 16, mainAddr 24. Init (r3 = the init block) stores r3 at
/// BLOCK_AT and returns GetDiskFragment(block->fileSpec, off, len, NULL, kLoadCFrag, NULL, &mainAddr, NULL); main jumps through mainAddr.
fn stub(off: i32, len: i32) -> Vec<u8> {
    let mut c = Asm::new();
    c.lwz(12, 24, 2).lwz(0, 0, 12).mtctr(0).lwz(2, 4, 12).bctr();
    let init_at = c.here();
    c.mflr(0).stw(0, 8, 1).stwu(1, -64, 1).stw(3, BLOCK_AT as i32, 0);
    c.lwz(3, 16, 3).li(4, off).li(5, len).li(6, 0).li(7, 1).li(8, 0).addi(9, 2, 24).li(10, 0).call_toc(16);
    c.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let mut data = vec![0u8; 32];
    data[8..12].copy_from_slice(&init_at.to_be_bytes());
    let mut b = PefBuilder::new();
    b.code(&c.finish());
    let d = b.data(&data, 32);
    b.import_library("CarbonLib", false, &[("GetDiskFragment", SymbolClass::TVector, false)]);
    b.reloc(d as u16, &[0x4601, 0x4A00]);
    b.main(d as i32, 0).init(d as i32, 8);
    b.build()
}

/// The fat file: data fork = stub (at 0) then the real code (at `off`, after `junk` bytes that are not a fragment); `cfrg` names the stub.
fn fat(junk: usize) -> (Vec<u8>, u32, u32) {
    let (r, len0) = (real(), stub(0, 0).len());
    let off = (len0 + 15) / 16 * 16 + junk;
    let s = stub(off as i32, r.len() as i32);
    assert_eq!(s.len(), len0, "the stub's size does not depend on its immediates");
    let mut fork = s.clone();
    fork.resize(off, 0xEE);
    fork.extend_from_slice(&r);
    let rsrc = ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", cfrg::USAGE_APPLICATION, cfrg::WHERE_DATA_FORK, 0, s.len() as u32, "FatLoader")).build();
    (macbinary("Fat App", b"APPL", b"FAT?", &fork, &rsrc), 0, s.len() as u32)
}

fn launched(file: &[u8]) -> MacPpc<RecordingGuestUi> {
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), file).unwrap();
    GuestMachine::start_file(&mut m, file, "Fat App").unwrap();
    m
}

#[test]
fn a_fat_loader_reads_its_init_block_and_runs_the_real_code_it_loads_from_its_own_file() {
    let (file, off, len) = fat(0);
    let mut m = launched(&file);
    assert_eq!(m.run_loop(1_000_000).0, Step::Exited, "{:?} {:?}", m.fault(), m.mac.log);
    assert_eq!(m.mac.mem.read_u32(MARK).unwrap(), 0x77, "the real fragment's main ran, through the mainAddr GetDiskFragment returned");
    // the CFragInitBlock: contextID/closureID/connectionID, kDataForkCFragLocator, the file's FSSpec, the stub's span, libName
    let p = m.mac.mem.read_u32(BLOCK_AT).unwrap();
    assert_ne!(p, 0, "the application's init routine got a CFragInitBlock, not NULL");
    let r = |o: u32| m.mac.mem.read_u32(p + o).unwrap();
    assert_eq!((r(12), r(20), r(24)), (1, off, len), "kDataForkCFragLocator with the stub's offset and length in the data fork");
    let spec = r(16);
    let name_len = m.mac.mem.read_u8(spec + 6).unwrap() as u32;
    let name = m.mac.mem.bytes(m.mac.mem.span(spec + 7, name_len).unwrap()).unwrap().to_vec();
    assert_eq!(name, b"Fat App", "the locator's FSSpec names the application file");
    assert_eq!(m.mac.mem.read_u16(spec).unwrap() as i16, m.mac.fm.vref_of(m.mac.fm.def_vol), "on the volume it was launched from");
    // the launch named the program in CurApName (LMGetCurApName) and CurApRefNum
    let ap = heirloom_mac68k_engine::lowmem::CUR_AP_NAME;
    let n = m.mac.mem.read_u8(ap).unwrap() as u32;
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(ap + 1, n).unwrap()).unwrap(), b"Fat App");
    let lib_name = r(28);
    assert_eq!(m.mac.mem.read_u8(lib_name).unwrap() as usize, "Fat App".len());
}

/// NEGATIVE CONTROL: the span the stub asks for is junk, not a fragment: GetDiskFragment refuses it (cfragFragmentFormatErr), the init
/// routine returns that error, and main never runs (the CFM's rule for a failed init).
#[test]
fn a_fat_loader_whose_real_code_is_not_a_fragment_stops_before_main() {
    let (mut file, _, _) = fat(64);
    // the real fragment moves 64 bytes on, so the stub's offset points into the 0xEE junk: rebuild the stub to ask for the junk
    let s_len = stub(0, 0).len();
    let junk_at = (s_len + 15) / 16 * 16;
    let bad = stub(junk_at as i32, 64);
    file[128..128 + s_len].copy_from_slice(&bad);
    let mut m = launched(&file);
    let _ = m.run_loop(1_000_000);
    assert_ne!(m.mac.mem.read_u32(MARK).unwrap(), 0x77, "main did not run");
    assert!(m.fault().is_some_and(|f| f.contains("init routine failed with OSErr -2806")), "{:?}", m.fault());
}

/// GetMemFragment(memAddr, length, fragName, options, &connID, &mainAddr, errMessage) prepares a PEF container that is already in memory
/// (Installer VISE's 'Xshb' external code, loaded from a locked resource handle): a connection and its main TVector come back, the TVector's
/// code is the fragment's own first instruction. NEGATIVE CONTROL: memory that is not a PEF container is cfragFragmentFormatErr.
#[test]
fn getmemfragment_prepares_a_fragment_held_in_memory() {
    use heirloom_macppc_engine::{GlueOutcome, PpcHost};
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let r = real();
    let at = m.mac.mm.new_ptr(&mut m.mac.mem, &mut m.mac.heap, r.len() as u32, true).unwrap();
    m.mac.mem.write_bytes(at, &r).unwrap();
    m.bind("GetMemFragment");
    let (conn, main) = (0x3100u32, 0x3104u32);
    let mut regs = [0u32; 32];
    regs[3..10].copy_from_slice(&[at, r.len() as u32, 0, 1, conn, main, 0]);
    assert_eq!(m.glue_call("GetMemFragment", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3], 0, "{:?}", m.mac.log);
    assert_ne!(m.mac.mem.read_u32(conn).unwrap(), 0, "a connection");
    let tv = m.mac.mem.read_u32(main).unwrap();
    let code = m.mac.mem.read_u32(tv).unwrap();
    assert_eq!(m.mac.mem.read_u32(code).unwrap(), 0x3880_0077, "mainAddr is the fragment's main: li r4,0x77");
    // NEGATIVE CONTROL
    m.mac.mem.write_bytes(at, b"not a PEF container").unwrap();
    regs[3..10].copy_from_slice(&[at, r.len() as u32, 0, 1, conn, main, 0]);
    assert_eq!(m.glue_call("GetMemFragment", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3] as i32 as i16, -2806);
}

/// A library loaded WHILE the program runs (GetMemFragment, as Photoshop 7 loads its DRM library with GetDiskFragment) has its init routine
/// run before the call returns: the program sees what the init did right after the call; the init gets a CFragInitBlock whose locator is the
/// memory it was loaded from. An init that fails makes the call answer cfragInitFunctionErr (-2821).
#[test]
fn a_library_loaded_at_run_time_runs_its_init_routine_before_the_call_returns() {
    use heirloom_macppc_engine::MacPpc as M;
    const DID: u32 = 0x3010; // the init's mark
    const WHERE: u32 = 0x3014; // the init block's locator: where, address
    const SEEN: u32 = 0x301C; // the program's copy of DID right after the call
    const RESULT: u32 = 0x3020;
    let lib = |status: i32| {
        let mut c = Asm::new();
        c.li(3, 0).blr();
        let init_at = c.here();
        c.li(4, 0x99).stw(4, DID as i32, 0).lwz(5, 12, 3).stw(5, WHERE as i32, 0).lwz(5, 16, 3).stw(5, WHERE as i32 + 4, 0).li(3, status).blr();
        let mut data = vec![0u8; 16];
        data[8..12].copy_from_slice(&init_at.to_be_bytes());
        let mut b = PefBuilder::new();
        b.code(&c.finish());
        let d = b.data(&data, 16);
        b.reloc(d as u16, &[0x4601]);
        b.main(d as i32, 0).init(d as i32, 8);
        b.build()
    };
    let run = |status: i32| {
        let l = lib(status);
        let mut m = M::new(RecordingGuestUi::new()).unwrap();
        let at = m.mac.mm.new_ptr(&mut m.mac.mem, &mut m.mac.heap, l.len() as u32, true).unwrap();
        m.mac.mem.write_bytes(at, &l).unwrap();
        // GetMemFragment(at, len, NULL, kLoadCFrag, &conn, &main, NULL); then SEEN = DID; RESULT = the call's r3
        let mut a = Asm::new();
        a.mflr(0).stw(0, 8, 1).stwu(1, -80, 1);
        a.lis(3, (at >> 16) as i32).ori(3, 3, (at & 0xFFFF) as i32).li(4, l.len() as i32).li(5, 0).li(6, 1).addi(7, 1, 56).addi(8, 1, 60).li(9, 0).call_toc(16);
        a.stw(3, RESULT as i32, 0).lwz(4, DID as i32, 0).stw(4, SEEN as i32, 0);
        a.addi(1, 1, 80).lwz(0, 8, 1).mtlr(0).li(3, 0).blr();
        let mut b = PefBuilder::new();
        b.code(&a.finish());
        let d = b.data(&[0u8; 32], 32);
        b.import_library("CarbonLib", false, &[("GetMemFragment", SymbolClass::TVector, false)]);
        b.reloc(d as u16, &[0x4600, 0x8007, 0x4A00]); // main TVector (0..8), RelocIncrPosition 8, GetMemFragment's TVector at 16
        b.main(d as i32, 0);
        let frag = m.instantiate("prog", &heirloom_fmt_pef::Pef::parse(&b.build()).unwrap()).unwrap();
        m.start(&frag).unwrap();
        assert_eq!(m.run_loop(1_000_000).0, Step::Exited, "{:?} {:?}", m.fault(), m.mac.log);
        let r = |a: u32| m.mac.mem.read_u32(a).unwrap();
        (r(RESULT) as i32, r(SEEN), r(WHERE), r(WHERE + 4), at)
    };
    let (result, seen, where_, address, at) = run(0);
    assert_eq!(result, 0);
    assert_eq!(seen, 0x99, "the init routine had run when GetMemFragment returned");
    assert_eq!((where_, address), (0, at), "kMemoryCFragLocator, at the container's address");
    let (result, ..) = run(-108);
    assert_eq!(result, -2821, "NEGATIVE CONTROL: a failing init routine fails the load with cfragInitFunctionErr");
}
