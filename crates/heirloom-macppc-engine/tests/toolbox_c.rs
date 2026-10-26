//! Toolbox library glue that has no trap (GetIndString, NumToString, text copies, low-memory accessors, FindFolder, the clocks), the Process Manager and the File Manager's
//! `PB...Sync/Async` calls, called by C name through the PowerPC glue (r3.. = the arguments).
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_fmt_resfork::writer::ForkBuilder;

type M = MacPpc<RecordingGuestUi>;
const A: u32 = 0x4000;
const B: u32 = 0x4200;
const PB: u32 = 0x4400;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}
fn refused(m: &mut M, name: &str, args: &[u32]) -> String {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    match m.glue_call(name, &mut regs) { GlueOutcome::Refused(w) => w, o => panic!("{name}: expected a refusal, got {o:?}") }
}
fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }
fn rd_p(m: &M, at: u32) -> String { let n = m.mac.mem.read_u8(at).unwrap() as u32; String::from_utf8(m.get_bytes(at + 1, n, "t").unwrap()).unwrap() }

#[test]
fn string_lists_text_copies_number_conversion_and_low_memory() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // STR# 200: three strings
    let mut list = vec![0, 3];
    for s in ["one", "", "three"] { list.push(s.len() as u8); list.extend(s.bytes()); }
    let fork = ForkBuilder::new().add(b"STR#", 200, None, &list).build();
    m.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork).unwrap());
    call(&mut m, "GetIndString", &[A, 200, 3]);
    assert_eq!(rd_p(&m, A), "three");
    call(&mut m, "GetIndString", &[A, 200, 1]);
    assert_eq!(rd_p(&m, A), "one");
    // NEGATIVE CONTROL: an index past the list (and a missing list) give the empty string
    call(&mut m, "GetIndString", &[A, 200, 4]);
    assert_eq!(rd_p(&m, A), "");
    pstr(&mut m, A, "zzz");
    call(&mut m, "GetIndString", &[A, 999, 1]);
    assert_eq!(rd_p(&m, A), "");
    // Pascal <-> C copies and in-place conversion
    pstr(&mut m, A, "hello");
    call(&mut m, "CopyPascalStringToC", &[A, B]);
    assert_eq!(m.get_bytes(B, 6, "t").unwrap(), b"hello\0");
    m.mac.mem.write_bytes(B, b"world!\0").unwrap();
    call(&mut m, "CopyCStringToPascal", &[B, A]);
    assert_eq!(rd_p(&m, A), "world!");
    assert_eq!(call(&mut m, "c2pstr", &[B]), B);
    assert_eq!(m.mac.mem.read_u8(B).unwrap(), 6);
    assert_eq!(call(&mut m, "p2cstr", &[B]), B);
    assert_eq!(m.get_bytes(B, 7, "t").unwrap(), b"world!\0");
    // EqualString(a, b, caseSens, diacSens)
    pstr(&mut m, A, "Abc"); pstr(&mut m, B, "aBC");
    assert_eq!((call(&mut m, "EqualString", &[A, B, 0, 0]), call(&mut m, "EqualString", &[A, B, 1, 0])), (1, 0));
    // NumToString / StringToNum
    call(&mut m, "NumToString", &[(-1234i32) as u32, A]);
    assert_eq!(rd_p(&m, A), "-1234");
    pstr(&mut m, B, "+77x");
    call(&mut m, "StringToNum", &[B, PB]);
    assert_eq!(m.mac.mem.read_u32(PB).unwrap(), 77);
    // low-memory accessors: a Set is seen by the Get; MBarHeight starts at its Toolbox value
    assert_eq!(call(&mut m, "LMGetMBarHeight", &[]), 20);
    call(&mut m, "LMSetMBarHeight", &[33]);
    assert_eq!(call(&mut m, "LMGetMBarHeight", &[]), 33);
    call(&mut m, "SetEventMask", &[0x1234]);
    assert_eq!(m.mac.mem.read_u16(0x144).unwrap(), 0x1234);
}

#[test]
fn clocks_folders_and_the_memory_queries_answer_from_the_machine() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "GetDateTime", &[PB]);
    assert!(m.mac.mem.read_u32(PB).unwrap() > 3_000_000_000, "seconds since 1904");
    m.mac.advance_ticks(60);
    let mut regs = [0u32; 32];
    m.bind("UpTime");
    assert_eq!(m.glue_call("UpTime", &mut regs), GlueOutcome::Return);
    assert_eq!((u64::from(regs[3]) << 32 | u64::from(regs[4])), 1_000_000_000, "one second of ticks is 1e9 AbsoluteTime units (nanoseconds)");
    call(&mut m, "Microseconds", &[PB]);
    assert_eq!((m.mac.mem.read_u32(PB).unwrap(), m.mac.mem.read_u32(PB + 4).unwrap()), (0, 1_000_000));
    // FindFolder('pref'): the Preferences folder exists (made on first use) on the boot volume; an unknown type is an error
    call(&mut m, "FindFolder", &[0xFFFF_8000, u32::from_be_bytes(*b"pref"), 1, PB, PB + 4]);
    let (vref, dir) = (m.mac.mem.read_u16(PB).unwrap() as i16, m.mac.mem.read_u32(PB + 4).unwrap());
    assert!(vref < 0 && dir >= 2, "{vref} {dir}");
    assert_ne!(call(&mut m, "FindFolder", &[0, u32::from_be_bytes(*b"zzzz"), 0, PB, PB + 4]) as i32, 0, "an unknown folder type is an error");
    // memory queries: MaxBlock is the largest free block, TempFreeMem the total, HLockHi locks
    let max = call(&mut m, "MaxBlock", &[]);
    // The machine is sized from the program now (no SIZE here -> the Finder default partition), so the bound is the machine, not 512 MiB.
    assert!(max > 4 << 20 && (max as usize) < m.mac.mem.len() && call(&mut m, "TempFreeMem", &[]) >= max, "MaxBlock {max:#x} of {:#x}", m.mac.mem.len());
    let h = call(&mut m, "NewHandle", &[64]);
    call(&mut m, "HLockHi", &[h]);
    assert_eq!(m.mac.mm.state(h).unwrap() & 0x80, 0x80, "the handle is locked");
}

#[test]
fn the_process_manager_knows_exactly_one_process_the_application() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "GetCurrentProcess", &[PB]) as i32, 0);
    assert_eq!((m.mac.mem.read_u32(PB).unwrap(), m.mac.mem.read_u32(PB + 4).unwrap()), (0, 2), "{{0, kCurrentProcess}}");
    assert_eq!(call(&mut m, "SetFrontProcess", &[PB]) as i32, 0);
    // SameProcess(psn1, psn2, Boolean*)
    m.mac.mem.write_u32(PB + 8, 0).unwrap(); m.mac.mem.write_u32(PB + 12, 2).unwrap();
    call(&mut m, "SameProcess", &[PB, PB + 8, PB + 16]);
    assert_eq!(m.mac.mem.read_u8(PB + 16).unwrap(), 1);
    // NEGATIVE CONTROL: a serial number that is not the application's is procNotFound (-600)
    m.mac.mem.write_u32(PB + 12, 9).unwrap();
    assert_eq!(call(&mut m, "SetFrontProcess", &[PB + 8]) as i32, -600);
}

/// An IOParam block for a refnum: ioRefNum at 24, ioBuffer 32, ioReqCount 36, ioActCount 40, ioPosMode 44, ioPosOffset 46.
#[test]
fn parameter_block_calls_run_the_file_manager_traps_on_the_callers_block() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    m.mac.fm.tree.put_file(root, "Data", b"0123456789".to_vec()).unwrap();
    pstr(&mut m, A, "Data");
    // PBHOpenDFSync-less route: FSOpen to get a refnum, then PBReadSync / PBGetEOFSync / PBSetFPosSync on blocks the program built
    assert_eq!(call(&mut m, "FSOpen", &[A, 0, PB + 0x100]) as i32, 0);
    let r = m.mac.mem.read_u16(PB + 0x100).unwrap();
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u16(PB + 24, r).unwrap();
    m.mac.mem.write_u32(PB + 32, B).unwrap();
    m.mac.mem.write_u32(PB + 36, 4).unwrap();
    assert_eq!(call(&mut m, "PBReadSync", &[PB]) as i32, 0);
    assert_eq!((m.mac.mem.read_u32(PB + 40).unwrap(), m.get_bytes(B, 4, "t").unwrap()), (4, b"0123".to_vec()));
    assert_eq!(m.mac.mem.read_u16(PB + 16).unwrap(), 0, "ioResult is the final result");
    assert_eq!(call(&mut m, "PBGetEOFSync", &[PB]) as i32, 0);
    assert_eq!(m.mac.mem.read_u32(PB + 28).unwrap(), 10, "ioMisc = the logical EOF");
    // HFS catalog info by name: PBGetCatInfoSync (an _HFSDispatch selector): ioNamePtr, ioFDirIndex = 0, ioDirID = 0 (the default directory)
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u32(PB + 18, A).unwrap();
    assert_eq!(call(&mut m, "PBGetCatInfoSync", &[PB]) as i32, 0);
    assert_eq!(m.mac.mem.read_u32(PB + 54).unwrap(), 10, "ioFlLgLen of the data fork");
    // NEGATIVE CONTROLS: a refnum that is open nowhere is rfNumErr; an Async call with a completion routine is refused BY NAME; an unmodelled PB call stays unbound
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u16(PB + 24, 0x3FFE).unwrap();
    assert_eq!(call(&mut m, "PBGetEOFSync", &[PB]) as i32, -51);
    m.mac.mem.write_u32(PB + 12, 0x1234).unwrap();
    assert!(refused(&mut m, "PBReadAsync", &[PB]).contains("completion routine"));
    assert!(matches!(m.bind("PBCatSearchSync"), Binding::Unbound(_)));
}
