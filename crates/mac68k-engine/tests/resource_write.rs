//! Changing a resource file through the Resource Manager: `AddResource`, `ChangedResource`, `WriteResource`, `UpdateResFile`,
//! `RmveResource`, `SetResInfo`, `SetResAttrs`, `UniqueID` over a real file's resource fork on the volume. Each step is a run of
//! scripted Toolbox calls (a real 68k program) on ONE machine; the file's fork is read back from the volume tree.
mod common;
use common::{pstr, put, Prog, GLOBALS};
use guestui_abi::RecordingGuestUi;
use mac68k_engine::fixture::{ScriptArg::Lit, ScriptCall};
use mac68k_engine::lowmem::RES_ERR;
use mac68k_engine::{Mac, Step};
use retro_resfork::ResFork;

const NAME: u32 = 0x3100; // the file's name (Str255)
const RES_NAME: u32 = 0x3200; // a resource's name (Str255)
const REF: u32 = 0x3000; // OpenResFile's answer

fn four(t: &[u8; 4]) -> u32 { u32::from_be_bytes(*t) }
fn c(name: &'static str, args: &[mac68k_engine::fixture::ScriptArg], save: Option<u32>) -> ScriptCall { ScriptCall { name, args: args.to_vec(), save } }

fn machine() -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    put(&mut m, NAME, &pstr("Prefs"));
    put(&mut m, RES_NAME, &pstr("Defaults"));
    m
}

/// Run the calls as one program; the resulting ResErr.
fn run(m: &mut Mac<RecordingGuestUi>, calls: &[ScriptCall]) -> i16 {
    let mut p = Prog::new();
    p.script(m, calls).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    m.mem.read_u16(RES_ERR).unwrap() as i16
}

/// A handle holding `bytes`, as `NewHandle` + a write would give.
fn handle(m: &mut Mac<RecordingGuestUi>, bytes: &[u8]) -> u32 {
    let h = m.mm.new_handle(&mut m.mem, &mut m.heap, bytes.len() as u32, false).unwrap();
    let p = m.mm.block(h).unwrap().ptr;
    m.mem.write_bytes(p, bytes).unwrap();
    h
}
fn rewrite(m: &mut Mac<RecordingGuestUi>, h: u32, bytes: &[u8]) {
    m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, bytes.len() as u32).unwrap();
    let p = m.mm.block(h).unwrap().ptr;
    m.mem.write_bytes(p, bytes).unwrap();
}

/// The Resource Manager's in-memory map of the open file `rn`.
fn mapped(m: &Mac<RecordingGuestUi>, rn: i16) -> &ResFork { &m.rm.files.iter().find(|f| f.ref_num == rn).expect("an open resource file").fork }

/// The fork of "Prefs" on the volume, as the File Manager holds it.
fn on_disk(m: &Mac<RecordingGuestUi>) -> ResFork {
    let l = m.fm.locate(0, 0, Some(b"Prefs")).unwrap();
    ResFork::parse(m.fm.tree.get(l.node.unwrap()).rsrc.clone()).unwrap()
}

/// CreateResFile + OpenResFile (read/write): the refnum.
fn create_and_open(m: &mut Mac<RecordingGuestUi>) -> i16 {
    assert_eq!(run(m, &[c("CreateResFile", &[Lit(NAME)], None), c("OpenResFile", &[Lit(NAME)], Some(REF))]), 0, "{:#?}", m.log);
    m.mem.read_u16(REF).unwrap() as i16
}

#[test]
fn a_resource_added_to_a_new_file_reaches_the_file_when_the_file_is_updated_and_again_when_it_is_closed() {
    let mut m = machine();
    let rn = create_and_open(&mut m);
    assert!(on_disk(&m).is_empty());
    let h = handle(&mut m, b"prefs v1");
    assert_eq!(run(&mut m, &[c("AddResource", &[Lit(h), Lit(four(b"PREF")), Lit(128), Lit(RES_NAME)], None)]), 0);
    assert!(m.rm.is_resource(h), "the handle is a resource handle now");
    assert!(on_disk(&m).is_empty(), "AddResource changes the map in memory only");
    assert_eq!(run(&mut m, &[c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]), 0);
    let f = on_disk(&m);
    let i = f.find(*b"PREF", 128).expect("the resource is in the file");
    assert_eq!((f.data(i).unwrap(), f.name(i).unwrap(), f.entry(i).unwrap().attrs & 2), (&b"prefs v1"[..], &b"Defaults"[..], 0), "its bytes, name, and resChanged cleared by the write");
    // the program changes the handle, ChangedResource flags it, CloseResFile writes it (it updates first)
    rewrite(&mut m, h, b"prefs v2, longer");
    assert_eq!(run(&mut m, &[c("ChangedResource", &[Lit(h)], None)]), 0);
    assert_eq!(on_disk(&m).data(i).unwrap(), b"prefs v1", "ChangedResource writes nothing by itself");
    assert_eq!(run(&mut m, &[c("CloseResFile", &[Lit(rn as u16 as u32)], None)]), 0);
    let f = on_disk(&m);
    assert_eq!(f.data(f.find(*b"PREF", 128).unwrap()).unwrap(), b"prefs v2, longer");
    assert!(m.fm.open_fork(rn).is_none(), "the file is closed");
}

#[test]
fn write_resource_copies_the_bytes_into_the_map_and_the_file_follows_at_the_next_update() {
    let mut m = machine();
    let rn = create_and_open(&mut m);
    let h = handle(&mut m, b"one");
    run(&mut m, &[c("AddResource", &[Lit(h), Lit(four(b"TEXT")), Lit(5), Lit(0)], None), c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]);
    rewrite(&mut m, h, b"two!");
    assert_eq!(run(&mut m, &[c("ChangedResource", &[Lit(h)], None), c("WriteResource", &[Lit(h)], None)]), 0);
    let i = mapped(&m, rn).find(*b"TEXT", 5).unwrap();
    assert_eq!(mapped(&m, rn).entry(i).unwrap().attrs & 2, 0, "WriteResource cleared resChanged");
    assert_eq!(mapped(&m, rn).data(i).unwrap(), b"two!");
    let f = on_disk(&m);
    assert_eq!(f.data(f.find(*b"TEXT", 5).unwrap()).unwrap(), b"one", "the file holds the old bytes until the update");
    run(&mut m, &[c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]);
    let f = on_disk(&m);
    assert_eq!(f.data(f.find(*b"TEXT", 5).unwrap()).unwrap(), b"two!");
}

#[test]
fn add_resource_refuses_a_nil_handle_and_one_that_is_already_a_resource() {
    let mut m = machine();
    let rn = create_and_open(&mut m);
    let h = handle(&mut m, b"x");
    assert_eq!(run(&mut m, &[c("AddResource", &[Lit(0), Lit(four(b"PREF")), Lit(1), Lit(0)], None)]), -194, "NIL: addResFailed");
    assert_eq!(run(&mut m, &[c("AddResource", &[Lit(h), Lit(four(b"PREF")), Lit(1), Lit(0)], None)]), 0);
    assert_eq!(run(&mut m, &[c("AddResource", &[Lit(h), Lit(four(b"PREF")), Lit(2), Lit(0)], None)]), -194, "already a resource: addResFailed");
    assert_eq!(mapped(&m, rn).count(*b"PREF"), 1, "the refused call added nothing");
    // NEGATIVE CONTROL for the rest: a handle that is not a resource is not found by the changing calls
    let plain = handle(&mut m, b"y");
    assert_eq!(run(&mut m, &[c("ChangedResource", &[Lit(plain)], None)]), -192, "resNotFound");
    assert_eq!(run(&mut m, &[c("WriteResource", &[Lit(plain)], None)]), -192);
    assert_eq!(run(&mut m, &[c("RmveResource", &[Lit(0)], None)]), -196, "NIL: rmvResFailed");
}

#[test]
fn rmve_resource_set_res_info_set_res_attrs_and_protection() {
    let mut m = machine();
    let rn = create_and_open(&mut m);
    let (a, b) = (handle(&mut m, b"aaa"), handle(&mut m, b"bbb"));
    run(&mut m, &[c("AddResource", &[Lit(a), Lit(four(b"DATA")), Lit(10), Lit(0)], None), c("AddResource", &[Lit(b), Lit(four(b"DATA")), Lit(11), Lit(0)], None),
                  c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]);
    // SetResInfo alone changes the map in memory; the file follows only once ChangedResource says so
    put(&mut m, RES_NAME, &pstr("renamed"));
    assert_eq!(run(&mut m, &[c("SetResInfo", &[Lit(a), Lit(20), Lit(RES_NAME)], None), c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]), 0);
    assert!(on_disk(&m).find(*b"DATA", 10).is_some() && on_disk(&m).find(*b"DATA", 20).is_none(), "no ChangedResource: the file is not rewritten");
    run(&mut m, &[c("ChangedResource", &[Lit(a)], None), c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]);
    let f = on_disk(&m);
    assert!(f.find(*b"DATA", 10).is_none());
    assert_eq!(f.name(f.find(*b"DATA", 20).unwrap()).unwrap(), b"renamed");
    // resProtected (0x08): neither ChangedResource, SetResInfo nor RmveResource may touch it
    assert_eq!(run(&mut m, &[c("SetResAttrs", &[Lit(b), Lit(0x08)], None)]), 0);
    assert_eq!(run(&mut m, &[c("ChangedResource", &[Lit(b)], None)]), -198, "resAttrErr");
    assert_eq!(run(&mut m, &[c("SetResInfo", &[Lit(b), Lit(99), Lit(0)], None)]), -198);
    assert_eq!(run(&mut m, &[c("RmveResource", &[Lit(b)], None)]), -196, "rmvResFailed");
    // unprotected, it goes
    run(&mut m, &[c("SetResAttrs", &[Lit(b), Lit(0)], None)]);
    assert_eq!(run(&mut m, &[c("RmveResource", &[Lit(b)], None), c("UpdateResFile", &[Lit(rn as u16 as u32)], None)]), 0);
    assert!(!m.rm.is_resource(b), "the handle is a plain handle again");
    let f = on_disk(&m);
    assert_eq!((f.count(*b"DATA"), f.find(*b"DATA", 11)), (1, None));
}

#[test]
fn unique_id_is_the_first_unused_id_from_128_and_a_read_only_file_is_not_written() {
    let mut m = machine();
    let rn = create_and_open(&mut m);
    run(&mut m, &[c("UniqueID", &[Lit(four(b"ICON"))], Some(0x3010))]);
    assert_eq!(m.mem.read_u16(0x3010).unwrap(), 128);
    let h = handle(&mut m, b"i");
    run(&mut m, &[c("AddResource", &[Lit(h), Lit(four(b"ICON")), Lit(128), Lit(0)], None), c("UniqueID", &[Lit(four(b"ICON"))], Some(0x3010)), c("UniqueID", &[Lit(four(b"CURS"))], Some(0x3012))]);
    assert_eq!((m.mem.read_u16(0x3010).unwrap(), m.mem.read_u16(0x3012).unwrap()), (129, 128), "per type; 128 is taken for ICON");
    run(&mut m, &[c("CloseResFile", &[Lit(rn as u16 as u32)], None)]);
    // reopened read-only (OpenRFPerm fsRdPerm): Add works in memory, WriteResource is wrPermErr, UpdateResFile writes nothing and says so by silence
    assert_eq!(run(&mut m, &[c("OpenRFPerm", &[Lit(NAME), Lit(0), Lit(1)], Some(REF))]), 0);
    let ro = m.mem.read_u16(REF).unwrap() as i16;
    let g = handle(&mut m, b"g");
    run(&mut m, &[c("AddResource", &[Lit(g), Lit(four(b"ICON")), Lit(200), Lit(0)], None)]);
    assert_eq!(run(&mut m, &[c("WriteResource", &[Lit(g)], None)]), -61, "wrPermErr");
    assert_eq!(run(&mut m, &[c("UpdateResFile", &[Lit(ro as u16 as u32)], None)]), 0);
    assert!(on_disk(&m).find(*b"ICON", 200).is_none(), "the read-only file did not change");
    // an unknown refnum
    assert_eq!(run(&mut m, &[c("UpdateResFile", &[Lit(77)], None)]), -193, "resFNotFound");
}
