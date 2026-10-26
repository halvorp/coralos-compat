//! The FSSpec calls through the PowerPC glue (`heirloom_macppc_engine::fsspec`) over the same volume the 68k engine serves.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const NAME: u32 = 0x3200;
const SPEC: u32 = 0x3300;
const OUT: u32 = 0x3400;
const BUF: u32 = 0x3500;

fn machine() -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    m.mac.fm.tree.put_file(root, "Data", b"0123456789".to_vec()).unwrap();
    m
}
fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }
fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
/// FSMakeFSSpec(0, 0, name, &spec)
fn make(m: &mut M, name: &str, at: u32) -> i32 { pstr(m, NAME, name); call(m, "FSMakeFSSpec", &[0, 0, NAME, at]) }

#[test]
fn make_an_fsspec_open_it_read_it_and_resolve_a_missing_name_for_create() {
    let mut m = machine();
    assert_eq!(make(&mut m, "Data", SPEC), 0);
    // FSpOpenDF(&spec, fsRdPerm = 1, &refNum), then the Files.h read through the C routine
    assert_eq!(call(&mut m, "FSpOpenDF", &[SPEC, 1, OUT]), 0);
    let r = m.mac.mem.read_u16(OUT).unwrap() as u32;
    m.mac.mem.write_u32(OUT + 4, 4).unwrap();
    assert_eq!(call(&mut m, "FSRead", &[r, OUT + 4, BUF]), 0);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(BUF, 4).unwrap()).unwrap(), b"0123", "the refnum is the File Manager's: FSRead sees the fork FSpOpenDF opened");
    assert_eq!(call(&mut m, "FSClose", &[r]), 0);
    // a name that does not exist: fnfErr, but the spec is still filled in so FSpCreate can use it
    assert_eq!(make(&mut m, "Fresh", SPEC + 0x60), -43);
    assert_eq!(call(&mut m, "FSpCreate", &[SPEC + 0x60, u32::from_be_bytes(*b"CoRa"), u32::from_be_bytes(*b"TEXT"), 0]), 0);
    assert_eq!(make(&mut m, "Fresh", SPEC + 0x60), 0, "now it exists");
    // FSpGetFInfo(&spec, &fInfo): fdType then fdCreator
    assert_eq!(call(&mut m, "FSpGetFInfo", &[SPEC + 0x60, BUF]), 0);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(BUF, 8).unwrap()).unwrap(), b"TEXTCoRa");
    // FSpRename, FSpDelete
    pstr(&mut m, NAME, "Renamed");
    assert_eq!(call(&mut m, "FSpRename", &[SPEC + 0x60, NAME]), 0);
    assert_eq!(make(&mut m, "Fresh", SPEC + 0x60), -43);
    assert_eq!(make(&mut m, "Renamed", SPEC + 0x60), 0);
    assert_eq!(call(&mut m, "FSpDelete", &[SPEC + 0x60]), 0);
    assert_eq!(make(&mut m, "Renamed", SPEC + 0x60), -43);
}

#[test]
fn errors_and_the_resource_file_forms_report_like_the_68k_calls() {
    let mut m = machine();
    // NEGATIVE CONTROL: opening a spec that names nothing is fnfErr and stores no refnum
    assert_eq!(make(&mut m, "Nope", SPEC), -43);
    m.mac.mem.write_u16(OUT, 0x7777).unwrap();
    assert_eq!(call(&mut m, "FSpOpenDF", &[SPEC, 1, OUT]), -43);
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap(), 0x7777);
    // FSpOpenResFile answers the refnum, -1 on failure (a data-only file has no resource fork: eofErr/-1)
    assert_eq!(make(&mut m, "Data", SPEC), 0);
    assert_eq!(call(&mut m, "FSpOpenResFile", &[SPEC, 1]), -1);
    // FSpCreateResFile makes the file with an (empty) resource fork, and FSpOpenResFile then opens it
    assert_eq!(make(&mut m, "Prefs", SPEC), -43);
    call(&mut m, "FSpCreateResFile", &[SPEC, u32::from_be_bytes(*b"CoRa"), u32::from_be_bytes(*b"pref"), 0]);
    assert_eq!(make(&mut m, "Prefs", SPEC), 0);
    assert!(call(&mut m, "FSpOpenResFile", &[SPEC, 3]) > 0);
    // FSpDirCreate(&spec, script, &dirID) stores the new directory ID
    assert_eq!(make(&mut m, "Folder", SPEC + 0x60), -43);
    assert_eq!(call(&mut m, "FSpDirCreate", &[SPEC + 0x60, 0, OUT + 8]), 0);
    assert!(m.mac.mem.read_u32(OUT + 8).unwrap() >= 3);
    // a name still unserved is refused by name
    assert!(matches!(m.bind("FSpResolve"), Binding::Unbound(why) if why.contains("FSpResolve")));
}
