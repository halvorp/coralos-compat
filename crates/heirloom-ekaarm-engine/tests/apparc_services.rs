//! Host oracle tests for REUSE U19 (`RApaLsSession` over `guestsvc_abi::GuestAppArc`,
//! backed on CoralOS by `coral-descriptor` + `coral-opener`).
use std::cell::RefCell;
use std::rc::Rc;

use guestsvc_abi::{FakeAppArc, GuestAppArc, GuestSvcError, GuestTypeInfo, CONF_CERTAIN, CONF_NOT_RECOGNIZED};
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::loader::{self, Process};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::sym_apparc::{
    stem_to_uid, KERR_NONE, KERR_NOT_FOUND, KERR_PERMISSION_DENIED, RECOG_CONFIDENCE_OFF,
};

#[derive(Clone)]
struct SharedAppArc(Rc<RefCell<FakeAppArc>>);

impl GuestAppArc for SharedAppArc {
    fn recognize_data(&mut self, name: &str, head: &[u8]) -> Result<GuestTypeInfo, GuestSvcError> {
        self.0.borrow_mut().recognize_data(name, head)
    }
    fn handler_for(&mut self, mime: &str, name: &str) -> Result<Option<String>, GuestSvcError> {
        self.0.borrow_mut().handler_for(mime, name)
    }
}

const STUBS: &[&str] = &[
    "_ZN4User6AllocZEi",
    "_ZN13RApaLsSession7ConnectEv",
    "_ZN13RApaLsSession5CloseEv",
    "_ZNK13RApaLsSession13RecognizeDataERK7TDesC16RK6TDesC8R22TDataRecognitionResult",
    "_ZNK13RApaLsSession14AppForDataTypeERK9TDataTypeR4TUid",
    "_ZN13RApaLsSession13StartDocumentERK7TDesC164TUidR9TThreadIdNS_11TLaunchTypeE",
];

fn make_proc(aa: Option<SharedAppArc>) -> Process {
    let mut p = loader::bare(STUBS).unwrap();
    if let Some(a) = aa {
        p.apparc_host = Some(Box::new(a));
    }
    p
}

fn call(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} missing"));
    for (i, &a) in args.iter().enumerate() {
        p.cpu.regs[i] = a;
    }
    match natives::call(p, idx) {
        NativeOutcome::Return => p.cpu.regs[0],
        other => panic!("{name}: unexpected {other:?}"),
    }
}

fn alloc_z(p: &mut Process, len: u32) -> u32 {
    call(p, "_ZN4User6AllocZEi", &[len])
}

fn mk_tbufc16(p: &mut Process, text: &str) -> u32 {
    let u16s: Vec<u16> = text.encode_utf16().collect();
    let addr = alloc_z(p, 4 + u16s.len() as u32 * 2);
    p.mem.write_u32(addr, u16s.len() as u32).unwrap();
    for (i, &c) in u16s.iter().enumerate() {
        p.mem.write_u16(addr + 4 + i as u32 * 2, c).unwrap();
    }
    addr
}

fn mk_tbufc8(p: &mut Process, bytes: &[u8]) -> u32 {
    let addr = alloc_z(p, 4 + bytes.len() as u32);
    p.mem.write_u32(addr, bytes.len() as u32).unwrap();
    for (i, &b) in bytes.iter().enumerate() {
        p.mem.write_u8(addr + 4 + i as u32, b).unwrap();
    }
    addr
}

fn mk_recog_result(p: &mut Process) -> u32 {
    let addr = alloc_z(p, 280);
    // EBuf = type 3 << 28 | len 0, max_len 256
    p.mem.write_u32(addr, 3u32 << 28).unwrap();
    p.mem.write_u32(addr + 4, 256).unwrap();
    addr
}

#[test]
fn apparc_recognizes_types_and_resolves_handlers_through_coral_host() {
    let fake = Rc::new(RefCell::new(FakeAppArc::new()));
    fake.borrow_mut().add_magic(b"\x89PNG\r\n\x1a\n", "image/png", CONF_CERTAIN);
    fake.borrow_mut().add_ext("txt", "text/plain");
    fake.borrow_mut().add_handler("image/png", "Preview");
    fake.borrow_mut().add_handler("text/plain", "Edit");
    let mut p = make_proc(Some(SharedAppArc(fake)));

    let sess = alloc_z(&mut p, 16);
    assert_eq!(call(&mut p, "_ZN13RApaLsSession7ConnectEv", &[sess]) as i32, KERR_NONE);

    let name = mk_tbufc16(&mut p, "photo.png");
    let head = mk_tbufc8(&mut p, b"\x89PNG\r\n\x1a\nIHDR");
    let res = mk_recog_result(&mut p);
    assert_eq!(call(&mut p, "_ZNK13RApaLsSession13RecognizeDataERK7TDesC16RK6TDesC8R22TDataRecognitionResult", &[sess, name, head, res]) as i32, KERR_NONE);
    assert_eq!(heirloom_ekaarm_engine::desc::read8(&p, res).unwrap(), b"image/png");
    assert_eq!(p.mem.read_u32(res + RECOG_CONFIDENCE_OFF).unwrap() as i32, CONF_CERTAIN);

    let uid_out = alloc_z(&mut p, 4);
    assert_eq!(call(&mut p, "_ZNK13RApaLsSession14AppForDataTypeERK9TDataTypeR4TUid", &[sess, res, uid_out]) as i32, KERR_NONE);
    assert_eq!(p.mem.read_u32(uid_out).unwrap(), stem_to_uid("Preview"));

    // Extension fallback via StartDocument -> Edit
    let txt_name = mk_tbufc16(&mut p, "readme.txt");
    assert_eq!(call(&mut p, "_ZN13RApaLsSession13StartDocumentERK7TDesC164TUidR9TThreadIdNS_11TLaunchTypeE", &[sess, txt_name, 0, 0]) as i32, KERR_NONE);
    call(&mut p, "_ZN13RApaLsSession5CloseEv", &[sess]);
}

#[test]
fn negative_controls_unrecognized_no_handler_and_not_granted() {
    let fake = Rc::new(RefCell::new(FakeAppArc::new()));
    let mut p = make_proc(Some(SharedAppArc(fake)));
    let sess = alloc_z(&mut p, 16);
    let name = mk_tbufc16(&mut p, "mystery.bin");
    let head = mk_tbufc8(&mut p, &[0xDE, 0xAD, 0xBE, 0xEF]);
    let res = mk_recog_result(&mut p);
    assert_eq!(call(&mut p, "_ZNK13RApaLsSession13RecognizeDataERK7TDesC16RK6TDesC8R22TDataRecognitionResult", &[sess, name, head, res]) as i32, KERR_NONE);
    assert_eq!(p.mem.read_u32(res + RECOG_CONFIDENCE_OFF).unwrap() as i32, CONF_NOT_RECOGNIZED);

    let uid_out = alloc_z(&mut p, 4);
    assert_eq!(call(&mut p, "_ZNK13RApaLsSession14AppForDataTypeERK9TDataTypeR4TUid", &[sess, res, uid_out]) as i32, KERR_NOT_FOUND);
    assert_eq!(p.mem.read_u32(uid_out).unwrap(), 0);
    assert_eq!(call(&mut p, "_ZN13RApaLsSession13StartDocumentERK7TDesC164TUidR9TThreadIdNS_11TLaunchTypeE", &[sess, name, 0, 0]) as i32, KERR_NOT_FOUND);

    // Unattached / NotGranted refuses KErrPermissionDenied (-46).
    let mut pu = make_proc(None);
    let s2 = alloc_z(&mut pu, 16);
    let n2 = mk_tbufc16(&mut pu, "a.png");
    let h2 = mk_tbufc8(&mut pu, b"\x89PNG");
    let r2 = mk_recog_result(&mut pu);
    assert_eq!(call(&mut pu, "_ZNK13RApaLsSession13RecognizeDataERK7TDesC16RK6TDesC8R22TDataRecognitionResult", &[s2, n2, h2, r2]) as i32, KERR_PERMISSION_DENIED);
}
