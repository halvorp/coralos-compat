//! Symbian message-store tree (`msgs`: `CMsvSession::GetChildIdsL`) as a stateless VIEW over `guestsvc_abi::GuestMessages`
//! (CoralOS `messages.service`, R11-01 / `docs/heirloom/21-SHARED-SERVICES.md` "Messaging is native").
//!
//! The engine holds no message store. What it holds is an id table: Symbian addresses every entry by a 32-bit `TMsvId`, the service
//! by `(conversation, seq)` or a draft id, so a listing hands out ids and remembers what each one names for the session. The same
//! entry gets the same id on every listing; ids never survive the engine process.
//!
//! Tree (`msvids.h`; the values are from memory of that header, there is no copy in the reference tree): root `0x1000` -> local service
//! `0x1001` -> Inbox `0x1002`, Outbox `0x1003`, Drafts `0x1004`, Sent `0x1005`.
//! - Inbox children = the newest stored messages of every conversation of the account (`messages`, <= `MSG_READ_MAX` each).
//! - Drafts children = `drafts()`.
//! - Outbox and Sent are listed EMPTY: the host trait carries no outgoing / delivery flag yet, and an invented one would be a lie.
//! - The `CMsvEntryFilter` argument is not interpreted: every child is returned. Said here so nobody believes it filters.
//! - The SMS / MMS / WAP-push / Bluetooth / IrDA TRANSPORT is refused by name at send time (`sym_messages.rs`); reading is not a transport.
//!
//! Without a `messages` grant every call leaves `KErrPermissionDenied` (-46): no engine-RAM fallback, no empty-tree pretence.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestsvc_abi::{GuestMessages, MSG_READ_MAX};
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::sym_messages::{map_svc_err as map_err, KERR_NOT_FOUND, KERR_PERMISSION_DENIED};

pub const KMSV_ROOT: u32 = 0x1000;
pub const KMSV_LOCAL_SERVICE: u32 = 0x1001;
pub const KMSV_INBOX: u32 = 0x1002;
pub const KMSV_OUTBOX: u32 = 0x1003;
pub const KMSV_DRAFTS: u32 = 0x1004;
pub const KMSV_SENT: u32 = 0x1005;
/// The first id handed to a message or draft entry (above every well-known folder id).
pub const KMSV_FIRST_ENTRY: u32 = 0x0010_0000;

/// What a handed-out `TMsvId` names in `messages.service`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MsvTarget {
    Message { account: String, conv: String, seq: u64 },
    Draft { account: String, id: u64 },
}

/// The session's id table (transient: rebuilt by listings, gone with the process).
#[derive(Debug, Default)]
pub struct MsvIndex {
    by_id: BTreeMap<u32, MsvTarget>,
    next: u32,
}

impl MsvIndex {
    fn id_for(&mut self, t: MsvTarget) -> u32 {
        if let Some((&id, _)) = self.by_id.iter().find(|(_, v)| **v == t) {
            return id;
        }
        let id = self.next.max(KMSV_FIRST_ENTRY);
        self.next = id + 1;
        self.by_id.insert(id, t);
        id
    }
    pub fn target(&self, id: u32) -> Option<&MsvTarget> {
        self.by_id.get(&id)
    }
}

/// One entry read back: what a guest's `CMsvStore` body / header read would show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsvRead {
    pub from: String,
    pub subject: String,
    pub body: String,
    pub date_ms: u64,
}

fn first_account(host: &mut dyn GuestMessages) -> Result<String, i32> {
    host.accounts().map_err(map_err)?.into_iter().next().ok_or(KERR_NOT_FOUND)
}

/// The ids under `parent` (`CMsvSession::GetChildIdsL`). An unknown id is `KErrNotFound`.
pub fn children(host: &mut dyn GuestMessages, idx: &mut MsvIndex, parent: u32) -> Result<Vec<u32>, i32> {
    match parent {
        KMSV_ROOT => {
            first_account(host)?; // the root exists only where the grant does
            Ok(alloc::vec![KMSV_LOCAL_SERVICE])
        }
        KMSV_LOCAL_SERVICE => {
            first_account(host)?;
            Ok(alloc::vec![KMSV_INBOX, KMSV_OUTBOX, KMSV_DRAFTS, KMSV_SENT])
        }
        KMSV_OUTBOX | KMSV_SENT => {
            first_account(host)?;
            Ok(Vec::new())
        }
        KMSV_INBOX => {
            let account = first_account(host)?;
            let mut out = Vec::new();
            for conv in host.conversations(&account).map_err(map_err)? {
                for m in host.messages(&account, &conv.id, MSG_READ_MAX).map_err(map_err)? {
                    out.push(idx.id_for(MsvTarget::Message { account: account.clone(), conv: conv.id.clone(), seq: m.seq }));
                }
            }
            Ok(out)
        }
        KMSV_DRAFTS => {
            let account = first_account(host)?;
            let drafts = host.drafts(&account).map_err(map_err)?;
            Ok(drafts.into_iter().map(|d| idx.id_for(MsvTarget::Draft { account: account.clone(), id: d.id })).collect())
        }
        _ => {
            first_account(host)?;
            Err(KERR_NOT_FOUND) // a message or draft has no children
        }
    }
}

/// Read the message or draft `id` names, through the service (never from a copy kept here).
pub fn read(host: &mut dyn GuestMessages, idx: &MsvIndex, id: u32) -> Result<MsvRead, i32> {
    match idx.target(id) {
        None => {
            first_account(host)?;
            Err(KERR_NOT_FOUND)
        }
        Some(MsvTarget::Message { account, conv, seq }) => {
            let msgs = host.messages(account, conv, MSG_READ_MAX).map_err(map_err)?;
            let m = msgs.into_iter().find(|m| m.seq == *seq).ok_or(KERR_NOT_FOUND)?;
            Ok(MsvRead { from: m.sender, subject: String::new(), body: m.body, date_ms: m.ts_ms })
        }
        Some(MsvTarget::Draft { account, id }) => {
            let d = host.drafts(account).map_err(map_err)?.into_iter().find(|d| d.id == *id).ok_or(KERR_NOT_FOUND)?;
            Ok(MsvRead { from: String::new(), subject: d.subject, body: d.body, date_ms: 0 })
        }
    }
}

pub fn native(proc: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r1, r3) = (proc.cpu.regs[1], proc.cpu.regs[3]);
    match name {
        // CMsvSession::GetChildIdsL(TMsvId, const CMsvEntryFilter&, CMsvEntrySelection&): the selection is reset, then filled.
        "_ZN11CMsvSession12GetChildIdsLElRK15CMsvEntryFilterR18CMsvEntrySelection" => {
            let mut idx = core::mem::take(&mut proc.nat.objs.msv);
            let res = match proc.messages_host.as_mut() {
                None => Err(KERR_PERMISSION_DENIED),
                Some(host) => children(host.as_mut(), &mut idx, r1),
            };
            proc.nat.objs.msv = idx;
            let code = match res {
                Ok(ids) => match fill_selection(proc, r3, &ids) {
                    Ok(()) => 0,
                    Err(m) => return Some(NativeOutcome::Refused(format!("msgs {name}: {m}"))),
                },
                Err(code) => code,
            };
            proc.log.push(format!("[messages] {name}: id {r1:#x} -> {code}"));
            if code != 0 && !proc.exception_descriptors.is_empty() {
                return Some(crate::ehabi::leave(proc, code));
            }
            proc.cpu.regs[0] = code as u32;
            Some(NativeOutcome::Return)
        }
        _ => None,
    }
}

fn fill_selection(proc: &mut Process, sel: u32, ids: &[u32]) -> Result<(), String> {
    proc.array_reset(sel)?;
    for (i, id) in ids.iter().enumerate() {
        proc.array_insert_record(sel, i as i32, id.to_le_bytes().to_vec())?;
    }
    Ok(())
}
