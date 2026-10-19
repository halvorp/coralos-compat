//! The File Manager as a program sees it: real 68k code issues `_Create` / `_Open` / `_Write` / `_Read` / `_Close` ...
//! with parameter blocks at their Inside Macintosh offsets, over the SHARED volume tree. Each behaviour has a negative
//! control (an error number, an unchanged tree).
use guestui_abi::RecordingGuestUi;
use mac68k_engine::Mac;
use retro_resfork::writer::ForkBuilder;

const PB: u32 = 0x3400;
const NAME: u32 = 0x3800;
const BUF: u32 = 0x3C00;
const OUT: u32 = 0x3000;

/// A tiny 68k assembler: just what these programs need.
#[derive(Default)]
struct Asm(Vec<u8>);
impl Asm {
    fn w(&mut self, ws: &[u16]) -> &mut Self { for x in ws { self.0.extend_from_slice(&x.to_be_bytes()); } self }
    fn lea_a0(&mut self, a: u32) -> &mut Self { self.w(&[0x41F9, (a >> 16) as u16, a as u16]) }
    fn moveq_d0(&mut self, v: u8) -> &mut Self { self.w(&[0x7000 | u16::from(v)]) }
    fn trap(&mut self, op: u16) -> &mut Self { self.w(&[op]) }
    fn d0_to_w(&mut self, a: u32) -> &mut Self { self.w(&[0x33C0, (a >> 16) as u16, a as u16]) }
    /// A call: A0 = the block, then the trap, then D0 (the OSErr) saved at `save`.
    fn call(&mut self, op: u16, save: u32) -> &mut Self { self.lea_a0(PB).trap(op).d0_to_w(save) }
    fn exit(&mut self) -> &mut Self { self.trap(0xA9F4) }
}

fn pstr(s: &str) -> Vec<u8> { let mut v = vec![s.len() as u8]; v.extend_from_slice(s.as_bytes()); v }
fn machine() -> Mac<RecordingGuestUi> { Mac::new(RecordingGuestUi::new()).unwrap() }
fn run(m: &mut Mac<RecordingGuestUi>, a: &Asm) {
    let mut code = a.0.clone();
    code.extend_from_slice(&[0xA9, 0xF4]); // _ExitToShell (a second one after an explicit exit is harmless)
    let r = m.run_program(&code, 0, 200_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?}\n{:#?}", r.stats, m.log);
}
fn w(m: &Mac<RecordingGuestUi>, a: u32) -> i16 { m.mem.read_u16(a).unwrap() as i16 }
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }
fn set(m: &mut Mac<RecordingGuestUi>, off: u32, v: &[u8]) { m.mem.write_bytes(off, v).unwrap(); }
fn setw(m: &mut Mac<RecordingGuestUi>, a: u32, v: u16) { m.mem.write_u16(a, v).unwrap(); }
fn setl(m: &mut Mac<RecordingGuestUi>, a: u32, v: u32) { m.mem.write_u32(a, v).unwrap(); }

#[test]
fn create_open_write_close_open_read_round_trips_through_the_shared_tree() {
    let mut m = machine();
    set(&mut m, NAME, &pstr("Notes"));
    setl(&mut m, PB + 18, NAME); // ioNamePtr
    let mut a = Asm::default();
    a.call(0xA008, OUT).call(0xA008, OUT + 2); // _Create twice: noErr, then dupFNErr
    setl(&mut m, PB + 18, NAME);
    setw(&mut m, PB + 22, 0); // ioVRefNum: default
    m.mem.write_u8(PB + 27, 3).unwrap(); // ioPermssn: read/write
    a.call(0xA000, OUT + 4); // _Open
    run(&mut m, &a);
    assert_eq!((w(&m, OUT), w(&m, OUT + 2), w(&m, OUT + 4)), (0, -48, 0), "create ok, duplicate refused, open ok");
    let rn = w(&m, PB + 24);
    assert!(rn > 0 && rn % 2 == 0, "ioRefNum {rn}");
    assert_eq!(m.mem.read_u16(PB + 16).unwrap(), 0, "ioResult mirrors D0");
    // write "hello mac" through the open fork
    set(&mut m, BUF, b"hello mac");
    setl(&mut m, PB + 32, BUF);
    setl(&mut m, PB + 36, 9);
    setw(&mut m, PB + 44, 1); // fsFromStart
    setl(&mut m, PB + 46, 0);
    let mut b = Asm::default();
    b.call(0xA003, OUT + 6); // _Write
    run(&mut m, &b);
    assert_eq!((w(&m, OUT + 6), l(&m, PB + 40), l(&m, PB + 46)), (0, 9, 9), "write ok, ioActCount 9, mark 9");
    // read it back from the start, asking for more than exists
    setl(&mut m, PB + 32, BUF + 0x100);
    setl(&mut m, PB + 36, 20);
    setl(&mut m, PB + 46, 0);
    let mut c = Asm::default();
    c.call(0xA002, OUT + 8).call(0xA011, OUT + 10).call(0xA001, OUT + 12).call(0xA001, OUT + 14); // Read, GetEOF, Close, Close again
    run(&mut m, &c);
    assert_eq!(w(&m, OUT + 8), -39, "reading past the end: eofErr");
    assert_eq!(l(&m, PB + 40), 9, "but the nine bytes arrived");
    assert_eq!(m.mem.bytes(m.mem.span(BUF + 0x100, 9).unwrap()).unwrap(), b"hello mac");
    assert_eq!((w(&m, OUT + 10), l(&m, PB + 28)), (0, 9), "GetEOF returns the length in ioMisc");
    assert_eq!((w(&m, OUT + 12), w(&m, OUT + 14)), (0, -51), "Close, then rfNumErr on a closed refnum");
    let f = m.fm.locate(0, 0, Some(b"Notes")).unwrap().node.unwrap();
    assert_eq!(m.fm.tree.file_data(f).unwrap(), b"hello mac", "the guest's data is in the SHARED tree");
}

#[test]
fn errors_are_the_inside_macintosh_numbers() {
    let mut m = machine();
    set(&mut m, NAME, &pstr("Missing"));
    setl(&mut m, PB + 18, NAME);
    m.mem.write_u8(PB + 27, 1).unwrap();
    set(&mut m, NAME + 0x40, &pstr("NoSuchVol:file"));
    let mut a = Asm::default();
    a.call(0xA000, OUT).call(0xA009, OUT + 2).call(0xA00C, OUT + 4);
    run(&mut m, &a);
    assert_eq!((w(&m, OUT), w(&m, OUT + 2), w(&m, OUT + 4)), (-43, -43, -43), "fnfErr for open, delete and info");
    setl(&mut m, PB + 18, NAME + 0x40); // "NoSuchVol:file"
    let mut b = Asm::default();
    b.call(0xA000, OUT + 6);
    run(&mut m, &b);
    assert_eq!(w(&m, OUT + 6), -35, "nsvErr for an unknown volume");
    set(&mut m, NAME + 0x80, &pstr(".Sony"));
    setl(&mut m, PB + 18, NAME + 0x80);
    let mut c = Asm::default();
    c.call(0xA000, OUT + 8);
    run(&mut m, &c);
    assert_eq!(w(&m, OUT + 8), -56, "a driver name finds no driver (nsDrvErr): the Device Manager is not modelled");
}

#[test]
fn an_async_call_runs_its_completion_routine_with_the_block_in_a0_and_the_result_in_d0() {
    let mut m = machine();
    set(&mut m, NAME, &pstr("A"));
    setl(&mut m, PB + 18, NAME);
    // The completion routine at $4400: move.w d0,OUT+0 ; move.l a0,OUT+2 ; moveq #77,d1 ; rts
    let comp = 0x4400u32;
    set(&mut m, comp, &[0x33, 0xC0, 0x00, 0x00, 0x30, 0x00, 0x23, 0xC8, 0x00, 0x00, 0x30, 0x02, 0x72, 0x4D, 0x4E, 0x75]);
    setl(&mut m, PB + 12, comp); // ioCompletion
    let mut a = Asm::default();
    a.lea_a0(PB).trap(0xA408).d0_to_w(OUT + 6).w(&[0x33C1, 0, (OUT + 8) as u16]); // _Create async ; D0 -> OUT+6 ; D1 -> OUT+8
    run(&mut m, &a);
    assert_eq!(w(&m, OUT), 0, "the completion routine ran with D0 = ioResult (0)");
    assert_eq!(l(&m, OUT + 2), PB, "... and A0 = the parameter block");
    assert_eq!(w(&m, OUT + 6), 0, "the trap itself returned noErr in D0 AFTER the completion routine trashed it");
    assert_eq!(w(&m, OUT + 8), 77, "the program resumed after the trap with the routine's side effects");
    assert!(m.fm.locate(0, 0, Some(b"A")).unwrap().node.is_some(), "the file was created");
    // NEGATIVE control: the same call, synchronous (bit 10 clear), does NOT call ioCompletion
    let mut m2 = machine();
    set(&mut m2, NAME, &pstr("A"));
    setl(&mut m2, PB + 18, NAME);
    set(&mut m2, comp, &[0x33, 0xC0, 0x00, 0x00, 0x30, 0x00, 0x4E, 0x75]);
    setl(&mut m2, PB + 12, comp);
    setw(&mut m2, OUT, 0x5555);
    let mut s = Asm::default();
    s.call(0xA008, OUT + 6);
    run(&mut m2, &s);
    assert_eq!(w(&m2, OUT), 0x5555, "completion routine untouched by a synchronous call");
    // an async failure completes with the error in ioResult and D0 of the routine, while the trap returns noErr
    setw(&mut m, OUT, 0x5555);
    let mut e = Asm::default();
    e.lea_a0(PB).trap(0xA408).d0_to_w(OUT + 6);
    run(&mut m, &e);
    assert_eq!((w(&m, OUT), w(&m, OUT + 6), w(&m, PB + 16)), (-48, 0, -48), "dupFNErr to the routine and ioResult; the queue accepted the call (D0 = 0)");
}

#[test]
fn get_cat_info_through_hfs_dispatch_walks_a_directory_by_index() {
    let mut m = machine();
    let root = m.fm.def_vol;
    let d = m.fm.tree.create_dir(root, "Docs").unwrap();
    let f = m.fm.tree.put_file(d, "ReadMe", b"abcdef".to_vec()).unwrap();
    m.fm.tree.get_mut(f).rsrc = b"RR".to_vec();
    m.fm.tree.get_mut(f).finfo.file_type = *b"TEXT";
    m.fm.tree.get_mut(f).finfo.creator = *b"ttxt";
    m.fm.tree.put_file(d, "Second", vec![]).unwrap();
    let dir_id = m.fm.dir_id(d);
    // PBGetCatInfo: ioNamePtr = a buffer for the name, ioDirID = Docs, ioFDirIndex = 1
    setl(&mut m, PB + 18, NAME);
    setw(&mut m, PB + 22, 0xFFFF);
    setw(&mut m, PB + 28, 1);
    setl(&mut m, PB + 48, dir_id);
    let mut a = Asm::default();
    a.moveq_d0(9).lea_a0(PB).trap(0xA260).d0_to_w(OUT);
    run(&mut m, &a);
    assert_eq!(w(&m, OUT), 0);
    assert_eq!(m.mem.read_cstr(NAME + 1).unwrap().get(..6), Some("ReadMe"));
    assert_eq!(m.mem.read_u8(PB + 30).unwrap() & 0x10, 0, "a file: ioFlAttrib bit 4 clear");
    assert_eq!(m.mem.bytes(m.mem.span(PB + 32, 8).unwrap()).unwrap(), b"TEXTttxt", "FInfo type and creator");
    assert_eq!((l(&m, PB + 54), l(&m, PB + 64)), (6, 2), "data and resource fork lengths");
    assert_eq!(l(&m, PB + 100), dir_id, "ioFlParID");
    assert_eq!(l(&m, PB + 48), m.fm.file_id(f), "ioDirID holds the file number for a file");
    // the directory itself: index -1 with its dirID
    setw(&mut m, PB + 28, 0xFFFF);
    setl(&mut m, PB + 48, dir_id); // the call overwrote it with the file number
    let mut b = Asm::default();
    b.moveq_d0(9).lea_a0(PB).trap(0xA260).d0_to_w(OUT + 2);
    run(&mut m, &b);
    assert_eq!(w(&m, OUT + 2), 0);
    assert_eq!(m.mem.read_u8(PB + 30).unwrap() & 0x10, 0x10, "a directory: attribute bit 4");
    assert_eq!(w(&m, PB + 52), 2, "ioDrNmFls: two items");
    // negative controls: index past the end, and an unknown selector (a NAMED refusal)
    setw(&mut m, PB + 28, 3);
    setl(&mut m, PB + 48, dir_id);
    let mut c = Asm::default();
    c.moveq_d0(9).lea_a0(PB).trap(0xA260).d0_to_w(OUT + 4).moveq_d0(99).lea_a0(PB).trap(0xA260);
    run(&mut m, &c);
    assert_eq!(w(&m, OUT + 4), -43, "index 3 of a 2-item directory: fnfErr");
    assert!(m.refused.iter().any(|r| r.contains("HFSDispatch selector 0x63 is not modelled")), "{:?}", m.refused);
}

#[test]
fn resource_forks_open_as_resource_files_and_close_with_the_file_manager() {
    let mut m = machine();
    let fork = ForkBuilder::new().add(b"STR ", 128, None, b"\x05hello").build();
    let root = m.fm.def_vol;
    let f = m.fm.tree.put_file(root, "Res", vec![]).unwrap();
    m.fm.tree.get_mut(f).rsrc = fork;
    set(&mut m, NAME, &pstr("Res"));
    // OpenResFile(name) ; GetResource('STR ',128) ; CloseResFile
    let mut a = Asm::default();
    a.w(&[0x4267]).w(&[0x2F3C, (NAME >> 16) as u16, NAME as u16]).trap(0xA997).w(&[0x33DF, 0, OUT as u16]);
    a.w(&[0x42A7, 0x2F3C, 0x5354, 0x5220, 0x3F3C, 128]).trap(0xA9A0).w(&[0x23DF, 0, (OUT + 4) as u16]);
    a.w(&[0x3F39, 0, OUT as u16]).trap(0xA99A);
    a.exit();
    run(&mut m, &a);
    let rn = w(&m, OUT);
    assert!(rn > 0, "OpenResFile gave refnum {rn}");
    let h = l(&m, OUT + 4);
    assert_ne!(h, 0, "GetResource found the resource in the opened file");
    assert_eq!(m.handle_bytes(h).unwrap(), b"\x05hello");
    assert!(m.fm.open_fork(rn).is_none(), "CloseResFile closed the File Manager fork too");
    assert_eq!(m.rm.files.iter().filter(|f| f.ref_num == rn).count(), 0);
    // NEGATIVE control: a file without a resource fork cannot be opened as one (eofErr, refnum -1); a missing file is fnfErr
    m.fm.tree.put_file(root, "Plain", vec![1]).unwrap();
    set(&mut m, NAME, &pstr("Plain"));
    let mut b = Asm::default();
    b.w(&[0x4267]).w(&[0x2F3C, (NAME >> 16) as u16, NAME as u16]).trap(0xA997).w(&[0x33DF, 0, (OUT + 8) as u16]).exit();
    run(&mut m, &b);
    assert_eq!(w(&m, OUT + 8), -1);
    assert_eq!(m.mem.read_u16(mac68k_engine::lowmem::RES_ERR).unwrap() as i16, -39);
}

#[test]
fn open_rf_perm_opens_a_resource_file_with_the_asked_permission_and_the_first_refnum_is_not_the_applications() {
    let mut m = machine();
    let fork = ForkBuilder::new().add(b"STR ", 128, None, b"\x01x").build();
    let root = m.fm.def_vol;
    let f = m.fm.tree.put_file(root, "Prefs", vec![]).unwrap();
    m.fm.tree.get_mut(f).rsrc = fork.clone();
    let ro = m.fm.add_volume("Disc", false);
    let g = m.fm.tree.put_file(ro, "Locked", vec![]).unwrap();
    m.fm.tree.get_mut(g).rsrc = fork;
    // OpenRFPerm(name, vRefNum 0, denyModes) -> refnum
    let open = |m: &mut Mac<RecordingGuestUi>, name: &str, perm: u8, at: u32| {
        set(m, NAME, &pstr(name));
        let mut a = Asm::default();
        a.w(&[0x4267, 0x2F3C, (NAME >> 16) as u16, NAME as u16, 0x3F3C, 0, 0x3F3C, u16::from(perm) << 8]).trap(0xA9C4).w(&[0x33DF, 0, at as u16]).exit();
        run(m, &a);
    };
    open(&mut m, "Prefs", 3, OUT); // fsRdWrPerm
    let rn = w(&m, OUT);
    assert!(rn > 0 && rn != mac68k_engine::resmgr::APP_REF_NUM, "refnum {rn}: the application's own fork already holds refnum 2");
    assert!(m.fm.open_fork(rn).is_some_and(|f| f.perm.write), "opened for writing as asked");
    assert_eq!(m.mem.read_u16(mac68k_engine::lowmem::RES_ERR).unwrap(), 0);
    // NEGATIVE CONTROLS: read/write on a read-only volume is wPrErr (-44) with refnum -1 and no fork left open; read-only works there;
    // a missing file is fnfErr (-43).
    let before = m.fm.open_forks().count();
    open(&mut m, "Disc:Locked", 3, OUT + 2);
    assert_eq!(w(&m, OUT + 2), -1);
    assert_eq!(m.mem.read_u16(mac68k_engine::lowmem::RES_ERR).unwrap() as i16, -44);
    assert_eq!(m.fm.open_forks().count(), before);
    open(&mut m, "Disc:Locked", 1, OUT + 4);
    assert!(w(&m, OUT + 4) > 0 && m.fm.open_fork(w(&m, OUT + 4)).is_some_and(|f| !f.perm.write));
    open(&mut m, "Nothing", 1, OUT + 6);
    assert_eq!((w(&m, OUT + 6), m.mem.read_u16(mac68k_engine::lowmem::RES_ERR).unwrap() as i16), (-1, -43));
    // The File Manager never hands out the application's refnum, whatever it opens first.
    let l = m.fm.locate(0, 0, Some(b"Prefs")).unwrap();
    assert_ne!(m.fm.open(&l, false, 1).unwrap(), mac68k_engine::resmgr::APP_REF_NUM);
}
