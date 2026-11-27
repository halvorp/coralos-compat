//! Host oracle tests for REUSE U15 (Symbian `msgs` / `sendas2` / `sendui` stateless adapter over
//! `guestsvc_abi::GuestMessages`, backed by CoralOS `messages.service`).
//!
//! Proves:
//! 1. `RSendAs` + `RSendAsMessage` (`Connect`, `CreateL`, `AddRecipientL`, `SetSubjectL`, `SetBodyTextL`,
//!    `AddAttachment`, `SaveMessageAndCloseL`, `SendMessageAndCloseL`) persists drafts and queues composed
//!    sends in `FakeMessages` with zero engine-local message database.
//! 2. `CSendUi` + `CMessageData` (`NewL`, `AppendToAddressL`, `SetSubjectL`, `AppendAttachmentL`,
//!    `CreateAndSendMessageL`) composes and queues through `messages.service`.
//! 3. NEGATIVE CONTROL (radio bearer): SMS (`0x1000102C`) and MMS (`0x100058E1`) MTM UIDs refuse at the
//!    send step BY NAME with `KErrNotSupported` (-5) while preserving the saved draft in `messages.service`.
//! 4. NEGATIVE CONTROL (no grant): `FakeMessages::not_granted()` and unattached host return
//!    `KErrPermissionDenied` (-46) with zero silent in-engine fallback.
//! 5. NEGATIVE CONTROL (traversal & lost service): `..\secret` attachment refused `KErrArgument` (-6);
//!    `GuestSvcError::Lost` refused `KErrServerTerminated` (-15).
use std::cell::RefCell;
use std::rc::Rc;

use guestsvc_abi::{
    FakeMessages, GuestAttachmentRef, GuestConv, GuestDraft, GuestMessages, GuestRecipient, GuestRecipientKind,
    GuestSvcError,
};
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::loader::{self, Process};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::sym_messages::{
    KERR_ARGUMENT, KERR_NONE, KERR_NOT_SUPPORTED, KERR_PERMISSION_DENIED, KERR_SERVER_TERMINATED,
    KUID_MSG_TYPE_MMS, KUID_MSG_TYPE_SMS, KUID_MSG_TYPE_SMTP,
};

#[derive(Clone)]
struct SharedMessages(Rc<RefCell<FakeMessages>>);

impl GuestMessages for SharedMessages {
    fn accounts(&mut self) -> Result<Vec<String>, GuestSvcError> {
        self.0.borrow_mut().accounts()
    }
    fn conversations(&mut self, account: &str) -> Result<Vec<GuestConv>, GuestSvcError> {
        self.0.borrow_mut().conversations(account)
    }
    fn save_draft(
        &mut self,
        account: &str,
        draft_id: Option<u64>,
        conv: Option<&str>,
        to: &[GuestRecipient],
        subject: &str,
        body: &str,
        attachments: &[GuestAttachmentRef],
    ) -> Result<u64, GuestSvcError> {
        self.0.borrow_mut().save_draft(account, draft_id, conv, to, subject, body, attachments)
    }
    fn drafts(&mut self, account: &str) -> Result<Vec<GuestDraft>, GuestSvcError> {
        self.0.borrow_mut().drafts(account)
    }
    fn delete_draft(&mut self, account: &str, draft_id: u64) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().delete_draft(account, draft_id)
    }
    fn send_draft(&mut self, account: &str, draft_id: u64) -> Result<u64, GuestSvcError> {
        self.0.borrow_mut().send_draft(account, draft_id)
    }
    fn delete_msg(&mut self, account: &str, conv: &str, seq: u64) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().delete_msg(account, conv, seq)
    }
}

const STUBS: &[&str] = &[
    "_ZN4User6AllocZEi",
    "_ZN4User4FreeEPv",
    "_ZN11CMsvSession9OpenSyncLER19MMsvSessionObserver",
    "_ZN11CMsvSession11RemoveEntryEl",
    "_ZN7RSendAs7ConnectEv",
    "_ZN14RSendAsMessage7CreateLER7RSendAs4TUid",
    "_ZN14RSendAsMessage13AddRecipientLERK7TDesC16NS_20TSendAsRecipientTypeE",
    "_ZN14RSendAsMessage11SetSubjectLERK7TDesC16",
    "_ZN14RSendAsMessage12SetBodyTextLERK7TDesC16",
    "_ZN14RSendAsMessage13AddAttachmentERK7TDesC16RK6TDesC8R14TRequestStatus",
    "_ZN14RSendAsMessage20SaveMessageAndCloseLEv",
    "_ZN14RSendAsMessage20SendMessageAndCloseLEv",
    "_ZN14RSendAsMessage11SendMessageER14TRequestStatus",
    "_ZN12CMessageData4NewLEv",
    "_ZN12CMessageData11SetSubjectLEPK7TDesC16",
    "_ZN12CMessageData16AppendToAddressLERK7TDesC16S2_",
    "_ZN12CMessageData17AppendAttachmentLERK7TDesC16",
    "_ZN7CSendUi4NewLEv",
    "_ZN7CSendUi21CreateAndSendMessageLE4TUidPK12CMessageDataS0_i",
];

fn make_proc(store: Option<SharedMessages>) -> Process {
    let mut p = loader::bare(STUBS).unwrap();
    p.uid3 = 0x1028_28d6;
    if let Some(s) = store {
        p.messages_host = Some(Box::new(s));
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

/// Write a Symbian EBufC16 descriptor (`(0 << 28) | len` followed by UTF-16LE code units) onto guest heap.
fn write_desc16(p: &mut Process, s: &str) -> u32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let bytes = 4 + units.len() * 2;
    let addr = alloc_z(p, bytes as u32);
    p.mem.write_u32(addr, units.len() as u32).unwrap();
    for (i, u) in units.iter().enumerate() {
        p.mem.write_u16(addr + 4 + (i as u32) * 2, *u).unwrap();
    }
    addr
}

/// Write a Symbian EBufC8 descriptor (`(0 << 28) | len` followed by raw bytes) onto guest heap.
fn write_desc8(p: &mut Process, s: &str) -> u32 {
    let b = s.as_bytes();
    let addr = alloc_z(p, (4 + b.len()) as u32);
    p.mem.write_u32(addr, b.len() as u32).unwrap();
    for (i, &byte) in b.iter().enumerate() {
        p.mem.write_u8(addr + 4 + i as u32, byte).unwrap();
    }
    addr
}

#[test]
fn sendas_saves_and_sends_composed_message_with_attachment_through_messages_service() {
    let fake = Rc::new(RefCell::new(FakeMessages::new()));
    let mut p = make_proc(Some(SharedMessages(fake.clone())));

    let sess = alloc_z(&mut p, 8);
    let msg = alloc_z(&mut p, 8);
    let status = alloc_z(&mut p, 4);
    assert_eq!(call(&mut p, "_ZN7RSendAs7ConnectEv", &[sess]) as i32, KERR_NONE);

    // 1. Save draft only (SaveMessageAndCloseL).
    assert_eq!(call(&mut p, "_ZN14RSendAsMessage7CreateLER7RSendAs4TUid", &[msg, sess, KUID_MSG_TYPE_SMTP]) as i32, KERR_NONE);
    let to_d = write_desc16(&mut p, "@bob:coral.lan");
    let subj_d = write_desc16(&mut p, "Weekly notes");
    let body_d = write_desc16(&mut p, "Hello from Symbian");
    let path_d = write_desc16(&mut p, "C:\\Users\\Default\\Documents\\notes.txt");
    let mime_d = write_desc8(&mut p, "text/plain");
    call(&mut p, "_ZN14RSendAsMessage13AddRecipientLERK7TDesC16NS_20TSendAsRecipientTypeE", &[msg, to_d, 0]);
    call(&mut p, "_ZN14RSendAsMessage11SetSubjectLERK7TDesC16", &[msg, subj_d]);
    call(&mut p, "_ZN14RSendAsMessage12SetBodyTextLERK7TDesC16", &[msg, body_d]);
    call(&mut p, "_ZN14RSendAsMessage13AddAttachmentERK7TDesC16RK6TDesC8R14TRequestStatus", &[msg, path_d, mime_d, status]);
    assert_eq!(p.mem.read_u32(status).unwrap() as i32, KERR_NONE);
    assert_eq!(call(&mut p, "_ZN14RSendAsMessage20SaveMessageAndCloseLEv", &[msg]) as i32, KERR_NONE);

    {
        let f = fake.borrow();
        assert_eq!(f.stored_drafts().len(), 1);
        let d = &f.stored_drafts()[0].1;
        assert_eq!(d.subject, "Weekly notes");
        assert_eq!(d.body, "Hello from Symbian");
        assert_eq!(d.to, [GuestRecipient { kind: GuestRecipientKind::To, addr: "@bob:coral.lan".into() }]);
        assert_eq!(d.attachments[0].path, "Users/Default/Documents/notes.txt");
        assert_eq!(d.attachments[0].mime, "text/plain");
    }

    // 2. Compose & send non-radio message (SendMessageAndCloseL) -> promoted to outbox.
    assert_eq!(call(&mut p, "_ZN14RSendAsMessage7CreateLER7RSendAs4TUid", &[msg, sess, KUID_MSG_TYPE_SMTP]) as i32, KERR_NONE);
    call(&mut p, "_ZN14RSendAsMessage13AddRecipientLERK7TDesC16NS_20TSendAsRecipientTypeE", &[msg, to_d, 0]);
    call(&mut p, "_ZN14RSendAsMessage12SetBodyTextLERK7TDesC16", &[msg, body_d]);
    assert_eq!(call(&mut p, "_ZN14RSendAsMessage20SendMessageAndCloseLEv", &[msg]) as i32, KERR_NONE);
    assert_eq!(fake.borrow().sent_drafts().len(), 1);
    assert_eq!(fake.borrow().sent_drafts()[0].2.body, "Hello from Symbian");
}

#[test]
fn sendui_sends_file_attachment_and_radio_sms_mms_bearers_are_refused_by_name_preserving_draft() {
    let fake = Rc::new(RefCell::new(FakeMessages::new()));
    let mut p = make_proc(Some(SharedMessages(fake.clone())));

    let sendui = call(&mut p, "_ZN7CSendUi4NewLEv", &[]);
    let md = call(&mut p, "_ZN12CMessageData4NewLEv", &[]);
    let to_d = write_desc16(&mut p, "@carol:coral.lan");
    let alias_d = write_desc16(&mut p, "Carol");
    let subj_d = write_desc16(&mut p, "Photo");
    let att_d = write_desc16(&mut p, "Users/Default/Documents/pic.png");
    call(&mut p, "_ZN12CMessageData16AppendToAddressLERK7TDesC16S2_", &[md, to_d, alias_d]);
    call(&mut p, "_ZN12CMessageData11SetSubjectLEPK7TDesC16", &[md, subj_d]);
    call(&mut p, "_ZN12CMessageData17AppendAttachmentLERK7TDesC16", &[md, att_d]);

    // Non-radio send succeeds through messages.service.
    assert_eq!(
        call(&mut p, "_ZN7CSendUi21CreateAndSendMessageLE4TUidPK12CMessageDataS0_i", &[sendui, KUID_MSG_TYPE_SMTP, md, 0, 0]) as i32,
        KERR_NONE
    );
    assert_eq!(fake.borrow().sent_drafts().len(), 1);
    assert_eq!(fake.borrow().sent_drafts()[0].2.attachments[0].path, "Users/Default/Documents/pic.png");

    // NEGATIVE CONTROL: SMS and MMS radio bearers refuse by name with KErrNotSupported (-5),
    // while leaving the composed draft saved in messages.service (doc 21 ruling).
    for radio_uid in [KUID_MSG_TYPE_SMS, KUID_MSG_TYPE_MMS] {
        let rc = call(&mut p, "_ZN7CSendUi21CreateAndSendMessageLE4TUidPK12CMessageDataS0_i", &[sendui, radio_uid, md, 0, 0]) as i32;
        assert_eq!(rc, KERR_NOT_SUPPORTED, "radio uid {radio_uid:#x} must refuse KErrNotSupported");
    }
    assert!(p.log.iter().any(|l| l.contains("SMS radio bearer")), "SMS refusal named in log: {:?}", p.log);
    assert!(p.log.iter().any(|l| l.contains("MMS radio bearer")), "MMS refusal named in log: {:?}", p.log);
    assert_eq!(fake.borrow().stored_drafts().len(), 2, "both radio-refused drafts preserved in messages.service");
    assert_eq!(fake.borrow().sent_drafts().len(), 1, "radio bearers queued nothing to outbox");
}

#[test]
fn negative_controls_refuse_not_granted_traversal_and_lost_service() {
    // NC1: unattached host and FakeMessages::not_granted() both return KErrPermissionDenied (-46).
    let mut unattached = make_proc(None);
    let sess = alloc_z(&mut unattached, 8);
    assert_eq!(call(&mut unattached, "_ZN7RSendAs7ConnectEv", &[sess]) as i32, KERR_PERMISSION_DENIED);

    let denied = Rc::new(RefCell::new(FakeMessages::not_granted()));
    let mut p_denied = make_proc(Some(SharedMessages(denied.clone())));
    let sess2 = alloc_z(&mut p_denied, 8);
    assert_eq!(call(&mut p_denied, "_ZN7RSendAs7ConnectEv", &[sess2]) as i32, KERR_PERMISSION_DENIED);
    assert!(denied.borrow().stored_drafts().is_empty());

    // NC2: attachment path traversal (`..`) refused KErrArgument (-6); Lost service refused KErrServerTerminated (-15).
    let fake = Rc::new(RefCell::new(FakeMessages::new()));
    let mut p = make_proc(Some(SharedMessages(fake.clone())));
    let s3 = alloc_z(&mut p, 8);
    let m3 = alloc_z(&mut p, 8);
    let st = alloc_z(&mut p, 4);
    assert_eq!(call(&mut p, "_ZN7RSendAs7ConnectEv", &[s3]) as i32, KERR_NONE);
    assert_eq!(call(&mut p, "_ZN14RSendAsMessage7CreateLER7RSendAs4TUid", &[m3, s3, KUID_MSG_TYPE_SMTP]) as i32, KERR_NONE);
    let bad_path = write_desc16(&mut p, "..\\System\\secret");
    let mime = write_desc8(&mut p, "text/plain");
    call(&mut p, "_ZN14RSendAsMessage13AddAttachmentERK7TDesC16RK6TDesC8R14TRequestStatus", &[m3, bad_path, mime, st]);
    assert_eq!(call(&mut p, "_ZN14RSendAsMessage20SaveMessageAndCloseLEv", &[m3]) as i32, KERR_ARGUMENT);

    fake.borrow_mut().set_injected_error(Some(GuestSvcError::Lost));
    assert_eq!(call(&mut p, "_ZN7RSendAs7ConnectEv", &[s3]) as i32, KERR_SERVER_TERMINATED);
}
