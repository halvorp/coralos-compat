//! The Alias Manager by C name through the PowerPC glue over the 68k engine's `_AliasDispatch` bodies: make a record for a file, resolve it, follow an alias FILE (the Finder's
//! 'alis' resource), the `...WithMountFlags` forms (flags dropped: no network volumes, no UI), and the failures.
use guestui_abi::RecordingGuestUi;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use retro_resfork::writer::ForkBuilder;

type M = MacPpc<RecordingGuestUi>;
const NAME: u32 = 0x3200;
const SPEC: u32 = 0x3300;
const SPEC2: u32 = 0x3400;
const OUT: u32 = 0x3500;
const FLAG: u32 = 0x3600;
const FLAG2: u32 = 0x3610;

fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }
fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn make(m: &mut M, name: &str, at: u32) -> i32 { pstr(m, NAME, name); call(m, "FSMakeFSSpec", &[0, 0, NAME, at]) }
fn spec_name(m: &M, at: u32) -> String { let n = m.mac.mem.read_u8(at + 6).unwrap() as u32; String::from_utf8(m.get_bytes(at + 7, n, "t").unwrap()).unwrap() }

#[test]
fn an_alias_record_resolves_to_its_file_and_an_alias_file_is_followed() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    m.mac.fm.tree.put_file(root, "Data", b"0123".to_vec()).unwrap();
    assert_eq!(make(&mut m, "Data", SPEC), 0);
    assert_eq!(call(&mut m, "NewAlias", &[0, SPEC, OUT]), 0);
    let alias = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(alias, 0);
    // ResolveAlias(NIL, alias, &spec2, &wasChanged) finds the same file; GetAliasInfo(asiAliasName) says its name
    assert_eq!(call(&mut m, "ResolveAlias", &[0, alias, SPEC2, FLAG]), 0);
    assert_eq!(spec_name(&m, SPEC2), "Data");
    assert_eq!(m.mac.mem.read_u8(FLAG).unwrap(), 0);
    assert_eq!(call(&mut m, "GetAliasInfo", &[alias, 0, NAME]), 0);
    assert_eq!(m.mac.mem.read_u8(NAME).unwrap(), 4);
    // the record is the documented layout: userType 0, version 2 at offset 6, the file name Str63 at 50
    let rec = m.mac.handle_bytes(alias).unwrap();
    assert_eq!((u16::from_be_bytes([rec[6], rec[7]]), rec[50], &rec[51..55]), (2, 4, &b"Data"[..]));
    // an alias FILE: Finder flag kIsAlias and an 'alis' 0 resource holding the record
    let link = m.mac.fm.tree.put_file(root, "Link", Vec::new()).unwrap();
    m.mac.fm.tree.get_mut(link).finfo.flags |= 0x8000;
    m.mac.fm.tree.get_mut(link).rsrc = ForkBuilder::new().add(b"alis", 0, None, &rec).build();
    assert_eq!(make(&mut m, "Link", SPEC), 0);
    assert_eq!(call(&mut m, "ResolveAliasFile", &[SPEC, 1, FLAG, FLAG2]), 0);
    assert_eq!((spec_name(&m, SPEC), m.mac.mem.read_u8(FLAG).unwrap(), m.mac.mem.read_u8(FLAG2).unwrap()), ("Data".into(), 0, 1), "followed to the target: not a folder, was an alias");
    // a plain file resolves to itself, wasAliased FALSE; the WithMountFlags form takes the extra flags word and behaves the same
    assert_eq!(call(&mut m, "ResolveAliasFileWithMountFlags", &[SPEC, 1, FLAG, FLAG2, 1]), 0);
    assert_eq!((spec_name(&m, SPEC), m.mac.mem.read_u8(FLAG2).unwrap()), ("Data".into(), 0));
}

#[test]
fn failures_are_the_alias_managers_and_the_fsref_forms_stop_by_name() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    let f = m.mac.fm.tree.put_file(root, "Gone", b"x".to_vec()).unwrap();
    let _ = f;
    assert_eq!(make(&mut m, "Gone", SPEC), 0);
    assert_eq!(call(&mut m, "NewAliasMinimal", &[SPEC, OUT]), 0);
    let alias = m.mac.mem.read_u32(OUT).unwrap();
    assert_eq!(call(&mut m, "FSpDelete", &[SPEC]), 0);
    // NEGATIVE CONTROL: the target is gone -> fnfErr (-43), and the spec is not overwritten
    assert_eq!(call(&mut m, "ResolveAlias", &[0, alias, SPEC2, FLAG]), -43);
    assert_eq!(call(&mut m, "ResolveAliasWithMountFlags", &[0, alias, SPEC2, FLAG, 1]), -43);
    // a target that does not exist cannot be given an alias
    assert_eq!(make(&mut m, "Gone", SPEC), -43);
    assert_eq!(call(&mut m, "NewAlias", &[0, SPEC, OUT]), -43);
    // FSRef forms are not modelled
    assert!(matches!(m.bind("FSResolveAlias"), Binding::Unbound(_)));
}
