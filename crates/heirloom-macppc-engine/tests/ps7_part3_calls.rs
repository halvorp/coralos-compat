//! The calls Photoshop 7.0 reached after its tryout dialog (part 3), each against its contract with a negative control: the FSRef / parameter-
//! block File Manager forms (PBGetCatalogInfoSync, FSOpenResFile, PBHOpenDenySync), the Resource Manager's map globals, VInstall / VRemove with
//! a due task run at a safepoint, StuffHex, GetWVariant, GetScriptVariable, the trap-era name aliases.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const SPEC: u32 = 0x3000;
const REF: u32 = 0x3100;
const CAT: u32 = 0x3300;
const NAME: u32 = 0x3400;
const PB: u32 = 0x3600;
const T: u32 = 0x3800;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }

/// "Macintosh HD:Adobe:Plug" (data "hello", a resource fork with 'STR ' 128) and its FSRef at REF.
fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    let dir = m.mac.fm.tree.mkdirs(root, ["Adobe"].iter().copied()).unwrap();
    let f = m.mac.fm.tree.put_file(dir, "Plug", b"hello".to_vec()).unwrap();
    m.mac.fm.tree.get_mut(f).rsrc = heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"STR ", 128, None, b"\x02hi").build();
    let id = m.mac.fm.dir_id(dir);
    pstr(&mut m, NAME, "Plug");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, id, NAME, SPEC]), 0);
    assert_eq!(call(&mut m, "FSpMakeFSRef", &[SPEC, REF]), 0);
    (m, id)
}

#[test]
fn pb_get_catalog_info_and_fs_open_res_file_take_an_fsref() {
    let (mut m, dir) = machine();
    // FSRefParam: ref 28, whichInfo 32, catInfo 36, spec 52, parentRef 56, outName 68; ioResult 16
    m.mac.mem.zero(PB, 80).unwrap();
    for (o, v) in [(28, REF), (32, 0x2 | 0x4 | 0x8), (36, CAT)] { m.mac.mem.write_u32(PB + o, v).unwrap(); }
    assert_eq!(call(&mut m, "PBGetCatalogInfoSync", &[PB]), 0);
    assert_eq!((m.mac.mem.read_u16(PB + 16).unwrap(), m.mac.mem.read_u32(CAT + 4).unwrap()), (0, dir), "ioResult noErr, parentDirID");
    // NEGATIVE CONTROL: a reference to nothing
    m.mac.mem.write_u32(PB + 28, PB + 0x200).unwrap();
    assert_ne!(call(&mut m, "PBGetCatalogInfoSync", &[PB]), 0);
    let rn = call(&mut m, "FSOpenResFile", &[REF, 1]);
    assert!(rn > 0, "a refNum: {rn}");
    assert_eq!(m.mac.rm.cur, rn as i16, "the file is the current resource file");
    // the map globals answer from the Resource Manager: TopMapHndl is the newest map, CurMap its refNum; set/restore round-trips
    let top = call(&mut m, "LMGetTopMapHndl", &[]) as u32;
    assert!(top != 0);
    let p = m.mac.mem.read_u32(top).unwrap();
    assert_eq!(m.mac.mem.read_u16(p + 20).unwrap() as i16, rn as i16, "the map handle carries its file's refNum");
    assert_eq!(call(&mut m, "LMGetCurMap", &[]) as i16, rn as i16);
    call(&mut m, "LMSetCurMap", &[0]); // the System file's map
    assert_eq!(m.mac.rm.cur, 0);
    call(&mut m, "LMSetCurMap", &[rn as u32]);
    call(&mut m, "LMSetTopMapHndl", &[top]);
    assert_eq!(call(&mut m, "LMGetTopMapHndl", &[]) as u32, top);
    // NEGATIVE CONTROLS: a refNum or a map handle that is not open is refused by name and changes nothing
    call(&mut m, "LMSetCurMap", &[777]);
    call(&mut m, "LMSetTopMapHndl", &[0x1234]);
    assert_eq!(m.mac.rm.cur, rn as i16);
    assert!(m.mac.refused.iter().filter(|r| r.contains("LMSetCurMap(777)") || r.contains("LMSetTopMapHndl(0x1234)")).count() == 2, "{:?}", m.mac.refused);
    // NEGATIVE CONTROL: a reference to nothing opens nothing: -1
    assert_eq!(call(&mut m, "FSOpenResFile", &[PB + 0x200, 1]), -1);
}

#[test]
fn pbh_open_deny_opens_with_the_access_its_deny_modes_ask_and_puts_the_word_back() {
    let (mut m, dir) = machine();
    m.mac.mem.zero(PB, 80).unwrap();
    m.mac.mem.write_u32(PB + 18, NAME).unwrap();
    m.mac.mem.write_u32(PB + 48, dir).unwrap();
    m.mac.mem.write_u16(PB + 26, 0x0021).unwrap(); // dmRd | dmNoneDenyWr
    assert_eq!(call(&mut m, "PBHOpenDenySync", &[PB]), 0);
    assert!(m.mac.mem.read_u16(PB + 24).unwrap() as i16 > 0, "a refNum");
    assert_eq!(m.mac.mem.read_u16(PB + 26).unwrap(), 0x0021, "ioDenyModes is the caller's again");
    // NEGATIVE CONTROL: a name that is not there
    pstr(&mut m, NAME, "Nope");
    assert_eq!(call(&mut m, "PBHOpenDenySync", &[PB]), -43);
}

#[test]
fn vinstall_queues_a_task_that_runs_when_due_and_vremove_takes_it_away() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.zero(T, 14).unwrap();
    m.mac.mem.write_u16(T + 4, 1).unwrap(); // vType
    m.mac.mem.write_u16(T + 10, 2).unwrap(); // vblCount
    assert_eq!(call(&mut m, "VInstall", &[T]), 0);
    assert_eq!(m.mac.vbl.queue, vec![T]);
    m.mac.advance_ticks(2);
    assert!(m.mac.vbl_due(), "the count reached zero: due at the next safepoint");
    assert_eq!(call(&mut m, "VRemove", &[T]), 0);
    assert!(!m.mac.vbl_due(), "a removed task is no longer due");
    // NEGATIVE CONTROLS: not installed -> qErr; a record that is not vType -> vTypErr, nothing queued
    assert_eq!(call(&mut m, "VRemove", &[T]), -1);
    m.mac.mem.write_u16(T + 4, 3).unwrap();
    assert_eq!(call(&mut m, "VInstall", &[T]), -2);
    assert!(m.mac.vbl.queue.is_empty());
}

#[test]
fn stuff_hex_get_w_variant_and_the_script_variables() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    pstr(&mut m, NAME, "0aFF10");
    call(&mut m, "StuffHex", &[T, NAME]);
    assert_eq!((0..3).map(|i| m.mac.mem.read_u8(T + i).unwrap()).collect::<Vec<_>>(), vec![0x0A, 0xFF, 0x10]);
    // NEGATIVE CONTROL: not hexadecimal -> refused by name, nothing stored
    pstr(&mut m, NAME, "zz");
    call(&mut m, "StuffHex", &[T + 8, NAME]);
    assert_eq!(m.mac.mem.read_u8(T + 8).unwrap(), 0);
    assert!(m.mac.refused.iter().any(|r| r.contains("StuffHex")));
    // GetScriptVariable: a script that is not installed answers 0; Roman's flags and direction
    assert_eq!(call(&mut m, "GetScriptVariable", &[1, 4]), 0);
    assert_eq!(call(&mut m, "GetScriptVariable", &[0, 4]), 0xFF);
    assert_eq!(call(&mut m, "GetScriptVariable", &[0, 6]), 0, "Roman is left to right");
    assert_eq!(call(&mut m, "GetScriptVariable", &[0, 22]), 3);
    // the trap-era aliases bind to the same handlers
    for n in ["RemoveResource", "Get1IndResource", "SetItemMark"] { assert!(!matches!(m.bind(n), Binding::Unbound(_)), "{n}: {:?}", m.bind(n)); }
    // ... and an alias whose trap has no handler stays unbound BY THE TRAP'S NAME (never a silent 0)
    assert!(matches!(m.bind("CalcVisBehind"), Binding::Unbound(w) if w.contains("_CalcVBehind")));
}
