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
    // AbsoluteTime is a STRUCT returned by value: the Mac OS PowerPC ABI passes the caller's result buffer in r3 and expects it back (Storm's
    // FPMicroseconds read a never-written buffer, so Warcraft III's 10 ms thread wait never ended)
    let mut regs = [0u32; 32];
    regs[3] = PB + 16;
    m.mac.mem.write_u32(PB + 16, 0xDEAD_BEEF).unwrap();
    m.bind("UpTime");
    assert_eq!(m.glue_call("UpTime", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3], PB + 16, "the result buffer comes back in r3");
    let up = u64::from(m.mac.mem.read_u32(PB + 16).unwrap()) << 32 | u64::from(m.mac.mem.read_u32(PB + 20).unwrap());
    assert_eq!(up, 1_000_000_000, "one second of ticks is 1e9 AbsoluteTime units (nanoseconds), in the caller's buffer");
    // AbsoluteToNanoseconds(AbsoluteTime by value in r4/r5) -> Nanoseconds, also through the r3 buffer: the identity in these units
    let mut regs = [0u32; 32];
    regs[3] = PB + 24; regs[4] = 0x1; regs[5] = 0x2;
    m.bind("AbsoluteToNanoseconds");
    assert_eq!(m.glue_call("AbsoluteToNanoseconds", &mut regs), GlueOutcome::Return);
    assert_eq!((regs[3], m.mac.mem.read_u32(PB + 24).unwrap(), m.mac.mem.read_u32(PB + 28).unwrap()), (PB + 24, 1, 2));
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
    assert_eq!((m.mac.mem.read_u32(PB).unwrap(), m.mac.mem.read_u32(PB + 4).unwrap()), (0, heirloom_mac68k_engine::processes::FIRST_PSN), "a real serial number: kCurrentProcess {{0,2}} only means the caller");
    assert_eq!(call(&mut m, "SetFrontProcess", &[PB]) as i32, 0);
    // SameProcess(psn1, psn2, Boolean*): the real serial number and kCurrentProcess are the same process
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
    // NEGATIVE CONTROLS: a refnum that is open nowhere is rfNumErr; an unmodelled PB call stays unbound
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u16(PB + 24, 0x3FFE).unwrap();
    assert_eq!(call(&mut m, "PBGetEOFSync", &[PB]) as i32, -51);
    assert!(matches!(m.bind("PBCatSearchSync"), Binding::Unbound(_)));
    let _ = refused;
}

/// `PBReadAsync` with a completion routine (Warcraft III's MPQ reads): the read is done when the call returns (ioResult final), and the routine runs with the block in
/// r3 at the NEXT glue call, never inside the issuing one; the interrupted call is entered again when it returns.
#[test]
fn an_async_completion_routine_runs_at_the_next_safepoint_with_the_block_in_r3() {
    use heirloom_macppc_engine::cpu::{Cpu, Host};
    use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_PPC};
    const TV: u32 = 0x0003_0000;
    const CODE: u32 = 0x0003_1000;
    const TOC: u32 = 0x0003_8000;
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    m.mac.fm.tree.put_file(root, "Data", b"0123456789".to_vec()).unwrap();
    pstr(&mut m, A, "Data");
    assert_eq!(call(&mut m, "FSOpen", &[A, 0, PB + 0x100]) as i32, 0);
    let r = m.mac.mem.read_u16(PB + 0x100).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    let upp = new_routine_descriptor(&mut m.mac, TV, 0xC0, ISA_PPC).unwrap();
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u32(PB + 12, upp).unwrap(); // ioCompletion
    m.mac.mem.write_u16(PB + 24, r).unwrap();
    m.mac.mem.write_u32(PB + 32, B).unwrap();
    m.mac.mem.write_u32(PB + 36, 3).unwrap();
    assert_eq!(call(&mut m, "PBReadAsync", &[PB]) as i32, 0);
    assert_eq!((m.mac.mem.read_u16(PB + 16).unwrap(), m.get_bytes(B, 3, "t").unwrap()), (0, b"012".to_vec()), "done (ioResult final) when the call returns");
    assert_eq!(m.callback_depth(), 0, "the routine did NOT run inside the issuing call");
    m.bind("TickCount");
    let id = m.glue.intern(&mut m.mac, "CFM", "TickCount").unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let mut cpu = Cpu::new(0x5000, 0x0004_0000);
    cpu.gpr[2] = 0x6000; cpu.gpr[3] = 0x77;
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[2]), (CODE, PB, TOC), "the next glue call enters the routine with r3 = the block");
    assert_eq!(m.callback_depth(), 1);
    let ret = cpu.lr;
    assert!(m.native(&mut cpu, ret));
    assert_eq!((cpu.pc, cpu.gpr[3]), (stub, 0x77), "the interrupted call is entered again, its registers restored");
    // NEGATIVE CONTROL: it runs once - the next glue call is just the call
    let mut cpu = Cpu::new(0x5000, 0x0004_0000);
    m.native(&mut cpu, stub);
    assert_ne!(cpu.pc, CODE);
    assert_eq!(m.callback_depth(), 0);
}

/// The launch installs the application's own file in the folder the launcher named (`set_home`), and `GetProcessInformation`'s processAppSpec names it - Warcraft III
/// finds its War3.mpq beside it that way (it stopped at "not properly installed" while processAppSpec was left unfilled).
#[test]
fn the_application_file_lives_in_its_home_folder_and_process_app_spec_names_it() {
    use heirloom_common::GuestMachine;
    let app = heirloom_macppc_engine::fixture::macbinary_app(&heirloom_macppc_engine::fixture::startup_app());
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let game = GuestMachine::volume(&mut m).add_volume("Game", false);
    assert!(GuestMachine::set_home(&mut m, &["Elsewhere"]).is_err(), "NEGATIVE CONTROL: a folder not on the volumes is refused by name");
    GuestMachine::set_home(&mut m, &["Game"]).unwrap();
    GuestMachine::start_file(&mut m, &app, "Startup").unwrap();
    let node = m.mac.fm.app.expect("the launch recorded the application's file");
    assert_eq!(m.mac.fm.tree.parent(node), game);
    // ProcessInfoRec: processInfoLength 60, processName NULL, processAppSpec -> B
    m.mac.mem.zero(PB, 64).unwrap();
    m.mac.mem.write_u32(PB, 60).unwrap();
    m.mac.mem.write_u32(PB + 56, B).unwrap();
    m.mac.mem.zero(B, 70).unwrap();
    m.mac.mem.write_u32(A, 0).unwrap(); m.mac.mem.write_u32(A + 4, 2).unwrap(); // kCurrentProcess
    assert_eq!(call(&mut m, "GetProcessInformation", &[A, PB]) as i32, 0);
    let vref = m.mac.mem.read_u16(B).unwrap() as i16;
    assert_eq!(vref, m.mac.fm.vref_of(game), "vRefNum = the home volume");
    assert_eq!(m.mac.mem.read_u32(B + 2).unwrap(), 2, "parID = the folder it is in: the volume root (fsRtDirID)");
    assert_eq!(rd_p(&m, B + 6), m.mac.fm.tree.name(node));
}

/// UpperString (spec Misc/UpperString.md vectors), GetGrayRgn (= the GrayRgn low-memory global after InitWindows), TEFromScrap / TEToScrap
/// (spec TextEdit/TEFromScrap.md): the Warcraft III Installer's CD-key fields use the scrap calls for Paste, and uppercase what was typed.
#[test]
fn upper_string_gray_rgn_and_the_te_scrap_glue_follow_their_specs() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_bytes(A, &[5, b'a', b'b', b'c', 0x8E, b'1']).unwrap();
    call(&mut m, "UpperString", &[A, 1]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(A, 6).unwrap()).unwrap(), &[5, b'A', b'B', b'C', 0x83, b'1'], "diacSensitive: e-acute -> E-acute");
    m.mac.mem.write_bytes(A, &[2, 0x8E, b'z']).unwrap();
    call(&mut m, "UpperString", &[A, 0]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(A, 3).unwrap()).unwrap(), &[2, b'E', b'Z'], "NEGATIVE CONTROL: marks stripped without diacSensitive");
    call(&mut m, "InitGraf", &[0x1000]);
    call(&mut m, "InitWindows", &[]);
    let gray = call(&mut m, "GetGrayRgn", &[]);
    assert_ne!(gray, 0);
    assert_eq!(gray, m.mac.mem.read_u32(0x9EE).unwrap());
    call(&mut m, "TEInit", &[]);
    call(&mut m, "ZeroScrap", &[]);
    assert_eq!(call(&mut m, "TEFromScrap", &[]) as i32, -102, "no TEXT on the desk scrap: noTypeErr");
    m.mac.mem.write_bytes(B, b"xy").unwrap();
    call(&mut m, "PutScrap", &[2, u32::from_be_bytes(*b"TEXT"), B]);
    assert_eq!(call(&mut m, "TEFromScrap", &[]), 0);
    assert_eq!(m.mac.mem.read_u16(0xAB0).unwrap(), 2, "TEScrpLength");
    call(&mut m, "ZeroScrap", &[]);
    assert_eq!(call(&mut m, "TEToScrap", &[]), 0);
    assert_eq!(m.mac.scrap.get(*b"TEXT"), Some(&b"xy"[..]), "the TE scrap went back to the desk scrap");
}

/// `GetParamText` hands back what `ParamText` set (a NIL pointer skips a string); `LMGetCurApName` is the ADDRESS of the CurApName Str31;
/// `LMSetResErr` / `LMGetResErr` and `LMSetRndSeed` / `LMGetRndSeed` are the low-memory words (Installer VISE calls each of them).
#[test]
fn get_param_text_cur_ap_name_res_err_and_rnd_seed() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    pstr(&mut m, A, "Photoshop");
    pstr(&mut m, A + 0x40, "7.0");
    call(&mut m, "ParamText", &[A, A + 0x40, 0, 0]);
    pstr(&mut m, B + 0x80, "untouched");
    call(&mut m, "GetParamText", &[B, B + 0x40, 0, B + 0xC0]);
    assert_eq!((rd_p(&m, B), rd_p(&m, B + 0x40), rd_p(&m, B + 0xC0)), ("Photoshop".into(), "7.0".into(), String::new()), "^0, ^1, and an empty ^3");
    assert_eq!(rd_p(&m, B + 0x80), "untouched", "NEGATIVE CONTROL: a NIL pointer is skipped");
    assert_eq!(call(&mut m, "LMGetCurApName", &[]), heirloom_mac68k_engine::lowmem::CUR_AP_NAME);
    call(&mut m, "LMSetResErr", &[(-192i16) as u16 as u32]);
    assert_eq!(call(&mut m, "LMGetResErr", &[]) as u16 as i16, -192);
    assert_eq!(call(&mut m, "ResError", &[]) as u16 as i16, -192, "the Resource Manager reads the same word");
    assert_eq!(call(&mut m, "GetMBarHeight", &[]), 20, "GetMBarHeight reads MBarHeight");
    assert_eq!(call(&mut m, "GetSysFont", &[]), 0, "systemFont (0): SysFontFam");
    call(&mut m, "LMSetRndSeed", &[0x1234_5678]);
    assert_eq!(call(&mut m, "LMGetRndSeed", &[]), 0x1234_5678);
}

/// The Universal Interfaces names of two traps whose trap-table names differ (`GetFontName` = _GetFName, `UpdateDialog` = _UpdtDialog) bind
/// to them, also from CarbonLib; `GetResInfo` with NIL `theID` / `theType` writes nothing at address 0 (Installer VISE passes NIL, and the
/// 'TEXT' it left at 0 then sent a call through an absent weak import to 0x54455854 instead of naming it).
#[test]
fn get_font_name_update_dialog_aliases_and_get_res_info_with_nil_pointers() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    for (lib, n) in [("InterfaceLib", "GetFontName"), ("CarbonLib", "GetFontName"), ("CarbonLib", "UpdateDialog")] {
        assert!(!matches!(m.bind_from(lib, n), Binding::Unbound(_)), "{lib}/{n}: {:?}", m.bind_from(lib, n));
    }
    call(&mut m, "GetFontName", &[0, A]);
    assert_eq!(rd_p(&m, A), "Chicago", "font family 0 is the system font");
    let fork = ForkBuilder::new().add(b"TEXT", 9012, None, b"hello").build();
    m.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork).unwrap());
    let h = call(&mut m, "Get1Resource", &[u32::from_be_bytes(*b"TEXT"), 9012]);
    assert_ne!(h, 0);
    m.mac.mem.write_u32(0, 0xDEAD_BEEF).unwrap();
    call(&mut m, "GetResInfo", &[h, 0, 0, 0]);
    assert_eq!(m.mac.mem.read_u32(0).unwrap(), 0xDEAD_BEEF, "NIL theID / theType: address 0 untouched");
    call(&mut m, "GetResInfo", &[h, B, B + 2, 0]);
    assert_eq!((m.mac.mem.read_u16(B).unwrap(), m.mac.mem.read_u32(B + 2).unwrap().to_be_bytes()), (9012, *b"TEXT"), "POSITIVE CONTROL: real pointers are filled");
}

/// GetIntlResource(theID) is the 68k IUGetIntl body by C name: the built-in itl0 for 0 (".,;$" separators), the same handle again; an ID it
/// does not provide (itl2's collation tables) is NIL and NAMED in refused, the program goes on (Installer VISE asks for itl0 / itl1).
#[test]
fn get_intl_resource_is_iugetintl() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let h = call(&mut m, "GetIntlResource", &[0]);
    assert_ne!(h, 0);
    let p = m.mac.mm.block(h).unwrap().ptr;
    assert_eq!(m.get_bytes(p, 4, "t").unwrap(), b".,;$");
    assert_eq!(call(&mut m, "GetIntlResource", &[0]), h, "the same handle");
    assert_ne!(call(&mut m, "GetIntlResource", &[1]), 0, "itl1");
    assert_eq!(call(&mut m, "GetIntlResource", &[2]), 0, "NEGATIVE CONTROL: no itl2");
    assert!(m.mac.refused.iter().any(|r| r.contains("itl2")), "{:?}", m.mac.refused);
}

/// LMGetBootDrive is the System Folder's working directory: resolving it gives the boot volume's System Folder (what SysEnvirons answers too).
#[test]
fn lm_get_boot_drive_is_the_system_folder_working_directory() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let wd = call(&mut m, "LMGetBootDrive", &[]) as u16 as i16;
    let (vref, dir) = m.mac.fm.find_folder(0, *b"macs").unwrap();
    assert_eq!(wd, m.mac.fm.system_folder_wd().unwrap(), "the same WD every time");
    let l = m.mac.fm.locate(wd, 0, None).unwrap();
    assert_eq!((m.mac.fm.vref_of(l.vol), m.mac.fm.dir_id(l.node.unwrap())), (vref, dir), "it names the System Folder");
}

/// DateString / TimeString by C name are the 68k IUDateString / IUTimeString texts (the Photoshop 7 installer stamps its install log with them).
#[test]
fn date_string_and_time_string() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let t = heirloom_mac68k_engine::intl::mac_seconds(2002, 11, 5, 14, 7, 9);
    call(&mut m, "DateString", &[t, 1, A, 0]);
    assert_eq!(rd_p(&m, A), heirloom_mac68k_engine::intl::date_text(t, 1).unwrap());
    call(&mut m, "TimeString", &[t, 1, A, 0]);
    assert_eq!(rd_p(&m, A), "2:07:09 PM");
    assert!(refused(&mut m, "DateString", &[t, 7, A, 0]).contains("DateForm 7"), "NEGATIVE CONTROL");
}

/// FindFolder's volume root ('root', kVolumeRootFolderType: Installer VISE resolves its "boot volume root" location with it), Application
/// Support and Shutdown Items in the System Folder.
#[test]
fn find_folder_root_application_support_and_shutdown_items() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "FindFolder", &[0xFFFF_8000, u32::from_be_bytes(*b"root"), 1, PB, PB + 4]), 0);
    assert_eq!((m.mac.mem.read_u16(PB).unwrap() as i16, m.mac.mem.read_u32(PB + 4).unwrap()), (-1, 2), "the boot volume's root directory (fsRtDirID)");
    for (ty, name) in [(*b"asup", "Application Support"), (*b"shdf", "Shutdown Items"), (*b"prof", "ColorSync Profiles")] {
        assert_eq!(call(&mut m, "FindFolder", &[0xFFFF_8000, u32::from_be_bytes(ty), 1, PB, PB + 4]), 0);
        let (vref, dir) = (m.mac.mem.read_u16(PB).unwrap() as i16, m.mac.mem.read_u32(PB + 4).unwrap());
        let node = m.mac.fm.dir_node(m.mac.fm.volume_of_vref(vref).unwrap(), dir).unwrap();
        assert_eq!(m.mac.fm.tree.name(node), name);
        assert_eq!(m.mac.fm.tree.name(m.mac.fm.tree.parent(node)), "System Folder");
    }
    assert_eq!(call(&mut m, "FindFolder", &[0xFFFF_8000, u32::from_be_bytes(*b"apps"), 1, PB, PB + 4]), 0);
    let node = m.mac.fm.dir_node(m.mac.fm.def_vol, m.mac.mem.read_u32(PB + 4).unwrap()).unwrap();
    assert_eq!((m.mac.fm.tree.name(node), m.mac.fm.tree.parent(node)), ("Applications (Mac OS 9)", m.mac.fm.def_vol), "Applications at the root");
    // kChewableItemsFolderType 'flnt': Temporary Items:Cleanup At Startup; kOnAppropriateDisk (-32767) is the boot volume on Mac OS 9
    assert_eq!(call(&mut m, "FindFolder", &[0xFFFF_8001, u32::from_be_bytes(*b"flnt"), 1, PB, PB + 4]), 0);
    let node = m.mac.fm.dir_node(m.mac.fm.def_vol, m.mac.mem.read_u32(PB + 4).unwrap()).unwrap();
    assert_eq!((m.mac.fm.tree.name(node), m.mac.fm.tree.name(m.mac.fm.tree.parent(node))), ("Cleanup At Startup", "Temporary Items"));
    // NEGATIVE controls: a type with no folder is fnfErr (-43); a domain this Mac OS 9 volume set does not have stays nsvErr (-35)
    assert_eq!(call(&mut m, "FindFolder", &[0xFFFF_8000, u32::from_be_bytes(*b"zzzz"), 1, PB, PB + 4]) as i32, -43);
    assert_eq!(call(&mut m, "FindFolder", &[0xFFFF_8006, u32::from_be_bytes(*b"flnt"), 1, PB, PB + 4]) as i32, -35);
}

/// LaunchApplication is MEDIATED (process_c.rs): a host whose launch policy refuses a second process gets memFullErr naming the file; with
/// the default policy a file that is not an application is paramErr, named; a block that is not a LaunchParamBlockRec ('LC') is paramErr.
/// (tests/processes.rs launches a real second application.)
#[test]
fn launch_application_is_mediated_by_policy_and_file_type() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    m.mac.fm.tree.put_file(root, "VSetupT", vec![0; 4]).unwrap();
    pstr(&mut m, B, "VSetupT");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, 0, B, B + 0x100]), 0);
    m.mac.mem.zero(PB, 44).unwrap();
    m.mac.mem.write_u16(PB + 6, 0x4C43).unwrap();
    m.mac.mem.write_u16(PB + 14, 0xC000).unwrap();
    m.mac.mem.write_u32(PB + 16, B + 0x100).unwrap();
    m.launch_policy = heirloom_macppc_engine::process_c::LaunchPolicy::Refuse;
    assert_eq!(call(&mut m, "LaunchApplication", &[PB]) as i32, -108);
    assert!(m.mac.log.iter().any(|l| l.contains("LaunchApplication \"Macintosh HD:VSetupT\"") && l.contains("policy refuses")), "{:?}", m.mac.log.last());
    m.launch_policy = Default::default();
    assert_eq!(call(&mut m, "LaunchApplication", &[PB]) as i32, -50, "a file that is not an 'APPL' is not launched");
    assert!(m.mac.log.iter().any(|l| l.contains("is not an application")), "{:?}", m.mac.log.last());
    m.mac.mem.write_u16(PB + 6, 0).unwrap();
    assert_eq!(call(&mut m, "LaunchApplication", &[PB]) as i32, -50, "NEGATIVE CONTROL: not a LaunchParamBlockRec");
}

/// A Toolbox manager initialised before QuickDraw (a library's InitWindows / InitFonts before the application's InitGraf: Photoshop 7's DRM
/// runs from static initialisers) gets the process's QuickDraw initialised first, instead of a refusal; the application's own InitGraf of the
/// same `qd` block later is the same state.
#[test]
fn init_windows_before_init_graf_initialises_quickdraw_first() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(m.mac.qd_globals, 0);
    call(&mut m, "InitFonts", &[]);
    call(&mut m, "InitWindows", &[]);
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    assert_ne!(m.mac.qd_globals, 0, "QuickDraw was initialised for the process");
    let qd = m.qd_block().unwrap();
    assert_eq!(m.mac.qd_globals, qd + heirloom_macppc_engine::machine::QD_GLOBALS_SIZE - 4, "in the process's qd block (thePort, the last field)");
    let q = m.mac.qd_globals;
    call(&mut m, "InitGraf", &[q]);
    assert!(m.mac.refused.is_empty(), "the application's own InitGraf afterwards is accepted: {:?}", m.mac.refused);
}

/// Photoshop 7's start-up calls (HANDOFF-2): LongSecondsToDate / LongDateToSeconds round trip (2002-04-30 13:45:10 is a Tuesday, day 120,
/// week 18), the one heap zone (ApplicationZone = SystemZone = HandleZone = TheZone; GetApplLimit - ApplicationZone is the heap a program may
/// grow into), RelString's three answers, Navigation Services' availability, the FractEnable / ROMMapInsert low-memory accessors and
/// NSetTrapAddress going into the shared patch table (GetTrapAddress then answers the patch).
#[test]
fn photoshop_start_up_calls_long_dates_zones_rel_string_nav_low_memory_and_trap_patches() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // 2002-04-30 13:45:10 = 3103019110 seconds since 1904 (a LongDateTime: high word 0)
    m.mac.mem.write_u32(A, 0).unwrap(); m.mac.mem.write_u32(A + 4, 3_103_019_110).unwrap();
    assert_eq!(call(&mut m, "LongSecondsToDate", &[A, B]), 0);
    let f: Vec<i16> = (0..11).map(|i| m.mac.mem.read_u16(B + 2 * i).unwrap() as i16).collect();
    assert_eq!(f, [0, 2002, 4, 30, 13, 45, 10, 3, 120, 18, 1], "era year month day hour minute second dayOfWeek(Tue=3) dayOfYear weekOfYear pm");
    m.mac.mem.write_u32(A, 0xFFFF_FFFF).unwrap();
    assert_eq!(call(&mut m, "LongDateToSeconds", &[B, A]), 0);
    assert_eq!((m.mac.mem.read_u32(A).unwrap(), m.mac.mem.read_u32(A + 4).unwrap()), (0, 3_103_019_110));
    // the zone calls
    let z = call(&mut m, "ApplicationZone", &[]);
    assert_eq!(z, call(&mut m, "SystemZone", &[]));
    assert_eq!(z, m.mac.mem.read_u32(0x118).unwrap(), "TheZone");
    let h = call(&mut m, "NewHandle", &[16]);
    assert_eq!(call(&mut m, "HandleZone", &[h]), z);
    assert!(call(&mut m, "GetApplLimit", &[]).wrapping_sub(z) >= 3 << 20, "the heap a program may grow into is the machine's, not 68k ApplLimit's 128 KB");
    assert!(refused(&mut m, "SetZone", &[z + 0x100]).contains("the only heap zone"), "NEGATIVE CONTROL: another zone");
    // RelString(a, b, caseSens, diacSens)
    pstr(&mut m, A, "apple"); pstr(&mut m, B, "Apple");
    assert_eq!(call(&mut m, "RelString", &[A, B, 0, 1]) as i32, 0);
    assert_eq!(call(&mut m, "RelString", &[A, B, 1, 1]) as i32, 1, "case-sensitive: lowercase sorts after");
    assert_eq!(call(&mut m, "RelString", &[B, A, 1, 1]) as i32, -1);
    // Navigation Services is part of the Mac OS 9.2.1 personality
    assert_eq!(call(&mut m, "NavServicesAvailable", &[]) & 0xFF, 1);
    assert_eq!(call(&mut m, "NavLibraryVersion", &[]), 0x0110_8000);
    // low memory accessors
    call(&mut m, "LMSetROMMapInsert", &[0xFF]);
    assert_eq!(call(&mut m, "LMGetROMMapInsert", &[]) & 0xFF, 0xFF);
    assert_eq!(m.mac.mem.read_u8(0x0B9E).unwrap(), 0xFF);
    // NSetTrapAddress(routine, _ExitToShell, ToolTrap): the shared patch table; GetToolTrapAddress answers the patch
    m.mac.mem.write_u16(PB, 0x4E75).unwrap();
    call(&mut m, "NSetTrapAddress", &[PB, 0xA9F4, 1]);
    assert_eq!(call(&mut m, "GetToolTrapAddress", &[0xA9F4]), PB);
    assert!(m.mac.log.iter().any(|l| l.contains("still reaches this engine's native")));
}
