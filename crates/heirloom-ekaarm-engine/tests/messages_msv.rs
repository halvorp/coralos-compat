//! Host oracle tests for R11-01: the Symbian message-store tree (`CMsvSession::GetChildIdsL`) as a stateless view over
//! `guestsvc_abi::GuestMessages` (CoralOS `messages.service`), with the grant ABSENT and with a fake GRANTED backend.
//!
//! 1. Granted: root -> local service -> Inbox / Outbox / Drafts / Sent; Inbox lists the stored messages of every conversation, Drafts the
//!    drafts, Outbox and Sent are empty (no outgoing flag exists); the same entry keeps its id across listings; a message is READ back
//!    through the service (sender, body, date); a saved draft is listed and read with its subject.
//! 2. NEGATIVE CONTROLS: no host attached, `NotGranted`, `Lost`, a deleted message and an unknown id each end in a named code
//!    (-46 / -46 / -15 / -1 / -1) with nothing served from engine memory; a listing never leaks another room.
use std::cell::RefCell;
use std::rc::Rc;

use guestsvc_abi::{
    FakeMessages, GuestAttachmentRef, GuestConv, GuestDraft, GuestMessages, GuestMsg, GuestRecipient, GuestRecipientKind,
    GuestSvcError,
};
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::loader::{self, Process};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::sym_msv::{
    self, MsvTarget, KMSV_DRAFTS, KMSV_FIRST_ENTRY, KMSV_INBOX, KMSV_LOCAL_SERVICE, KMSV_OUTBOX, KMSV_ROOT, KMSV_SENT,
};

#[derive(Clone)]
struct Shared(Rc<RefCell<FakeMessages>>);

impl GuestMessages for Shared {
    fn accounts(&mut self) -> Result<Vec<String>, GuestSvcError> { self.0.borrow_mut().accounts() }
    fn conversations(&mut self, account: &str) -> Result<Vec<GuestConv>, GuestSvcError> { self.0.borrow_mut().conversations(account) }
    fn messages(&mut self, account: &str, conv: &str, limit: usize) -> Result<Vec<GuestMsg>, GuestSvcError> { self.0.borrow_mut().messages(account, conv, limit) }
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
    fn drafts(&mut self, account: &str) -> Result<Vec<GuestDraft>, GuestSvcError> { self.0.borrow_mut().drafts(account) }
    fn delete_draft(&mut self, account: &str, draft_id: u64) -> Result<(), GuestSvcError> { self.0.borrow_mut().delete_draft(account, draft_id) }
    fn send_draft(&mut self, account: &str, draft_id: u64) -> Result<u64, GuestSvcError> { self.0.borrow_mut().send_draft(account, draft_id) }
    fn delete_msg(&mut self, account: &str, conv: &str, seq: u64) -> Result<(), GuestSvcError> { self.0.borrow_mut().delete_msg(account, conv, seq) }
}

const GET_CHILDREN: &str = "_ZN11CMsvSession12GetChildIdsLElRK15CMsvEntryFilterR18CMsvEntrySelection";
const OPEN: &str = "_ZN11CMsvSession9OpenSyncLER19MMsvSessionObserver";
const CTOR: &str = "_ZN13CArrayFixBaseC1EPFP8CBufBaseiEii";
const AT: &str = "_ZNK13CArrayFixBase2AtEi";
const REP: &str = "_ZN8CBufFlat4NewLEi";
const ACCT: &str = "matrix:@user:coral.lan";
const ROOM: &str = "!room:coral.lan";

fn proc_with(host: Option<Box<dyn GuestMessages>>) -> Process {
    let mut p = loader::bare(&["_ZN4User6AllocZEi", GET_CHILDREN, OPEN, CTOR, AT, REP]).unwrap();
    p.uid3 = 0x1028_28d6;
    p.messages_host = host;
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

/// A guest `CMsvEntrySelection` (a `CArrayFixFlat<TMsvId>`: 4-byte records) built the way the guest's own constructor does.
fn selection(p: &mut Process) -> u32 {
    let sel = call(p, "_ZN4User6AllocZEi", &[24]);
    let rep = p.engine_stub(REP).unwrap();
    call(p, CTOR, &[sel, rep, 4, 8]);
    sel
}

fn ids(p: &mut Process, sel: u32) -> Vec<u32> {
    let n = p.mem.read_u32(sel + 4).unwrap();
    (0..n).map(|i| { let a = call(p, AT, &[sel, i]); p.mem.read_u32(a).unwrap() }).collect()
}

/// `GetChildIdsL(session, id, filter, selection)`: returns the guest's result code and the ids it left in the selection.
fn children(p: &mut Process, session: u32, id: u32) -> (i32, Vec<u32>) {
    let sel = selection(p);
    let rc = call(p, GET_CHILDREN, &[session, id, 0, sel]) as i32;
    let got = ids(p, sel);
    (rc, got)
}

fn granted() -> (Rc<RefCell<FakeMessages>>, Process, u32) {
    let fake = Rc::new(RefCell::new(FakeMessages::new()));
    {
        let mut f = fake.borrow_mut();
        f.add_conv(ACCT, GuestConv { id: ROOM.into(), title: "Room".into(), last_seq: 3, read_seq: 1 });
        f.add_conv(ACCT, GuestConv { id: "!quiet:coral.lan".into(), title: "Quiet".into(), last_seq: 0, read_seq: 0 });
        for seq in 1..=3u64 {
            f.add_msg(ACCT, ROOM, GuestMsg { seq, sender: "@bob:coral.lan".into(), ts_ms: seq * 1000, body: format!("body {seq}") });
        }
        f.add_msg(ACCT, "!hidden:coral.lan", GuestMsg { seq: 1, sender: "@eve:x".into(), ts_ms: 1, body: "not in a listed conversation".into() });
    }
    let mut p = proc_with(Some(Box::new(Shared(fake.clone()))));
    let session = call(&mut p, OPEN, &[0, 0]);
    assert_ne!(session, 0);
    (fake, p, session)
}

#[test]
fn the_store_tree_lists_folders_messages_and_drafts_and_a_message_reads_back_through_the_service() {
    let (fake, mut p, session) = granted();

    assert_eq!(children(&mut p, session, KMSV_ROOT), (0, vec![KMSV_LOCAL_SERVICE]), "list folders: root -> the local service");
    assert_eq!(
        children(&mut p, session, KMSV_LOCAL_SERVICE),
        (0, vec![KMSV_INBOX, KMSV_OUTBOX, KMSV_DRAFTS, KMSV_SENT]),
        "list folders: the four standard folders"
    );
    assert_eq!(children(&mut p, session, KMSV_OUTBOX), (0, vec![]), "no outgoing flag exists: Outbox lists nothing, it invents nothing");
    assert_eq!(children(&mut p, session, KMSV_SENT), (0, vec![]));

    let (rc, inbox) = children(&mut p, session, KMSV_INBOX);
    assert_eq!(rc, 0);
    assert_eq!(inbox.len(), 3, "the three stored messages of the listed room; the unlisted room's message never appears");
    assert!(inbox.iter().all(|&id| id >= KMSV_FIRST_ENTRY));
    assert_eq!(children(&mut p, session, KMSV_INBOX).1, inbox, "the same entry keeps its id across listings");

    // read a message: through the service, by what the id names
    let mut host = p.messages_host.take().unwrap();
    let idx = std::mem::take(&mut p.nat.objs.msv);
    let first = sym_msv::read(host.as_mut(), &idx, inbox[0]).unwrap();
    assert_eq!((first.from.as_str(), first.body.as_str(), first.date_ms), ("@bob:coral.lan", "body 1", 1000));
    assert_eq!(sym_msv::read(host.as_mut(), &idx, inbox[2]).unwrap().body, "body 3");
    assert_eq!(idx.target(inbox[1]), Some(&MsvTarget::Message { account: ACCT.into(), conv: ROOM.into(), seq: 2 }));
    p.messages_host = Some(host);
    p.nat.objs.msv = idx;

    // a message deleted in the service is gone from the next listing and reads as not-found (the engine kept no copy)
    fake.borrow_mut().delete_msg(ACCT, ROOM, 2).unwrap();
    assert_eq!(children(&mut p, session, KMSV_INBOX).1.len(), 2);
    let mut host = p.messages_host.take().unwrap();
    let idx = std::mem::take(&mut p.nat.objs.msv);
    assert_eq!(sym_msv::read(host.as_mut(), &idx, inbox[1]), Err(-1), "NC: the deleted message is KErrNotFound");
    assert_eq!(sym_msv::read(host.as_mut(), &idx, 0x7777), Err(-1), "NC: an id nobody handed out is KErrNotFound");
    p.messages_host = Some(host);
    p.nat.objs.msv = idx;

    // compose + queue is the other half of the mapping (messages_services.rs); here: the draft shows under Drafts and reads back
    let to = [GuestRecipient { kind: GuestRecipientKind::To, addr: "@bob:coral.lan".into() }];
    fake.borrow_mut().save_draft(ACCT, None, None, &to, "Re: notes", "draft text", &[]).unwrap();
    let (rc, drafts) = children(&mut p, session, KMSV_DRAFTS);
    assert_eq!((rc, drafts.len()), (0, 1));
    let mut host = p.messages_host.take().unwrap();
    let idx = std::mem::take(&mut p.nat.objs.msv);
    let d = sym_msv::read(host.as_mut(), &idx, drafts[0]).unwrap();
    assert_eq!((d.subject.as_str(), d.body.as_str()), ("Re: notes", "draft text"));
    p.messages_host = Some(host);
    p.nat.objs.msv = idx;
    assert_eq!(children(&mut p, session, drafts[0]).0, -1, "a message has no children");
}

#[test]
fn without_the_grant_every_listing_is_a_named_refusal_and_nothing_is_served_from_the_engine() {
    // no host attached at all (the ekaarm ceiling has no `messages` word: this is the shipped state)
    let mut p = proc_with(None);
    let sel = selection(&mut p);
    for id in [KMSV_ROOT, KMSV_LOCAL_SERVICE, KMSV_INBOX, KMSV_DRAFTS] {
        assert_eq!(call(&mut p, GET_CHILDREN, &[0, id, 0, sel]) as i32, -46, "KErrPermissionDenied for {id:#x}");
        assert_eq!(ids(&mut p, sel), Vec::<u32>::new(), "nothing in the selection");
    }
    assert!(p.log.iter().any(|l| l.contains("GetChildIdsL") && l.contains("-46")), "the refusal is logged by name: {:?}", p.log);

    // a host that declines (NotGranted) and one that is gone (Lost)
    let mut denied = proc_with(Some(Box::new(FakeMessages::not_granted())));
    let sel = selection(&mut denied);
    assert_eq!(call(&mut denied, GET_CHILDREN, &[0, KMSV_INBOX, 0, sel]) as i32, -46);
    let mut lost = FakeMessages::new();
    lost.set_injected_error(Some(GuestSvcError::Lost));
    let mut gone = proc_with(Some(Box::new(lost)));
    let sel = selection(&mut gone);
    assert_eq!(call(&mut gone, GET_CHILDREN, &[0, KMSV_INBOX, 0, sel]) as i32, -15, "KErrServerTerminated");
    assert_eq!(ids(&mut gone, sel), Vec::<u32>::new());
}
