//! Guest messaging host trait (`GuestMessages`, REUSE-AUDIT U15) backed by CoralOS `messages.service`.
//!
//! Adapters translate guest messaging APIs (Symbian `msgs` / `sendas2` / `sendui`) onto this trait with
//! plain host types, zero CoralOS wire types, and NEVER choose an owner (the loader admits each launch to
//! at most one owner per `messages.service` C2 policy). Radio bearers (SMS/MMS) are refused by name at
//! the send step while drafts remain in `messages.service`.
use alloc::string::String;
use alloc::vec::Vec;
use crate::GuestSvcError;

pub const MSG_BODY_MAX: usize = 1024;
pub const MSG_SUBJECT_MAX: usize = 128;
pub const MSG_ID_MAX: usize = 96;
pub const MSG_PATH_MAX: usize = 200;
pub const MSG_MIME_MAX: usize = 64;
pub const MSG_RECIPIENTS_MAX: usize = 8;
pub const MSG_ATTACHMENTS_MAX: usize = 4;
pub const MSG_DRAFTS_MAX: usize = 32;

/// Recipient role in a composed guest message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GuestRecipientKind {
    To = 0,
    Cc = 1,
    Bcc = 2,
}

impl GuestRecipientKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::To),
            1 => Some(Self::Cc),
            2 => Some(Self::Bcc),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestRecipient {
    pub kind: GuestRecipientKind,
    pub addr: String,
}

/// CoralFS path reference for an attachment (never a raw file blob over IPC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestAttachmentRef {
    pub path: String,
    pub mime: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestDraft {
    pub id: u64,
    pub conv: Option<String>,
    pub to: Vec<GuestRecipient>,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<GuestAttachmentRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestConv {
    pub id: String,
    pub title: String,
    pub last_seq: u64,
    pub read_seq: u64,
}

/// One stored message of a conversation, as the service holds it (read-only to a guest).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestMsg {
    pub seq: u64,
    pub sender: String,
    pub ts_ms: u64,
    pub body: String,
}

/// Most messages one `messages` read returns (a guest pages by asking again; the service bounds its own pages).
pub const MSG_READ_MAX: usize = 32;

/// Validate an attachment reference path (relative CoralFS path, no `..`, no control chars).
pub fn attachment_ref_ok(a: &GuestAttachmentRef) -> bool {
    if a.path.is_empty() || a.path.len() > MSG_PATH_MAX || a.path.starts_with('/') || a.path.chars().any(char::is_control) {
        return false;
    }
    if a.path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return false;
    }
    a.mime.len() <= MSG_MIME_MAX
        && !a.mime.chars().any(char::is_control)
        && a.name.len() <= MSG_ID_MAX
        && !a.name.chars().any(char::is_control)
}

/// Messaging store and compose surface backed by CoralOS `messages.service` (reader lane).
pub trait GuestMessages {
    fn accounts(&mut self) -> Result<Vec<String>, GuestSvcError>;
    fn conversations(&mut self, account: &str) -> Result<Vec<GuestConv>, GuestSvcError>;
    /// The newest `limit` (<= [`MSG_READ_MAX`]) stored messages of `conv`, oldest first. Read-only; flags and bodies come as the
    /// service stores them (plain text, <= `MSG_BODY_MAX`).
    fn messages(&mut self, account: &str, conv: &str, limit: usize) -> Result<Vec<GuestMsg>, GuestSvcError>;
    fn save_draft(
        &mut self,
        account: &str,
        draft_id: Option<u64>,
        conv: Option<&str>,
        to: &[GuestRecipient],
        subject: &str,
        body: &str,
        attachments: &[GuestAttachmentRef],
    ) -> Result<u64, GuestSvcError>;
    fn drafts(&mut self, account: &str) -> Result<Vec<GuestDraft>, GuestSvcError>;
    fn delete_draft(&mut self, account: &str, draft_id: u64) -> Result<(), GuestSvcError>;
    fn send_draft(&mut self, account: &str, draft_id: u64) -> Result<u64, GuestSvcError>;
    fn delete_msg(&mut self, account: &str, conv: &str, seq: u64) -> Result<(), GuestSvcError>;
}

/// Deterministic in-memory [`GuestMessages`] implementor for host oracle tests.
#[derive(Clone, Debug, Default)]
pub struct FakeMessages {
    accounts: Vec<String>,
    convs: Vec<(String, GuestConv)>,
    msgs: Vec<(String, String, GuestMsg)>,
    drafts: Vec<(String, GuestDraft)>,
    sent: Vec<(String, u64, GuestDraft)>,
    deleted_msgs: Vec<(String, String, u64)>,
    next_draft: u64,
    next_out: u64,
    not_granted: bool,
    injected_error: Option<GuestSvcError>,
}

impl FakeMessages {
    pub fn new() -> Self {
        Self {
            accounts: alloc::vec![String::from("matrix:@user:coral.lan")],
            next_draft: 1,
            next_out: 1,
            ..Self::default()
        }
    }

    pub fn not_granted() -> Self {
        Self { not_granted: true, next_draft: 1, next_out: 1, ..Self::default() }
    }

    pub fn set_granted(&mut self, granted: bool) {
        self.not_granted = !granted;
    }

    pub fn set_injected_error(&mut self, err: Option<GuestSvcError>) {
        self.injected_error = err;
    }

    pub fn add_account(&mut self, account: &str) {
        if !self.accounts.iter().any(|a| a == account) {
            self.accounts.push(String::from(account));
        }
    }

    pub fn add_conv(&mut self, account: &str, conv: GuestConv) {
        self.add_account(account);
        self.convs.push((String::from(account), conv));
    }

    /// Seed a stored message into `conv` of `account` (a test of the read path; the real service owns the store).
    pub fn add_msg(&mut self, account: &str, conv: &str, msg: GuestMsg) {
        self.add_account(account);
        self.msgs.push((String::from(account), String::from(conv), msg));
    }

    pub fn stored_drafts(&self) -> &[(String, GuestDraft)] {
        &self.drafts
    }

    pub fn sent_drafts(&self) -> &[(String, u64, GuestDraft)] {
        &self.sent
    }

    pub fn deleted_msgs(&self) -> &[(String, String, u64)] {
        &self.deleted_msgs
    }

    fn check_access(&self) -> Result<(), GuestSvcError> {
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        if let Some(err) = self.injected_error {
            return Err(err);
        }
        Ok(())
    }
}

impl GuestMessages for FakeMessages {
    fn accounts(&mut self) -> Result<Vec<String>, GuestSvcError> {
        self.check_access()?;
        Ok(self.accounts.clone())
    }

    fn conversations(&mut self, account: &str) -> Result<Vec<GuestConv>, GuestSvcError> {
        self.check_access()?;
        if account.is_empty() || account.len() > MSG_ID_MAX {
            return Err(GuestSvcError::BadName);
        }
        Ok(self.convs.iter().filter(|(a, _)| a == account).map(|(_, c)| c.clone()).collect())
    }

    fn messages(&mut self, account: &str, conv: &str, limit: usize) -> Result<Vec<GuestMsg>, GuestSvcError> {
        self.check_access()?;
        if account.is_empty() || account.len() > MSG_ID_MAX || conv.is_empty() || conv.len() > MSG_ID_MAX {
            return Err(GuestSvcError::BadName);
        }
        let mut all: Vec<GuestMsg> = self
            .msgs
            .iter()
            .filter(|(a, c, m)| a == account && c == conv && !self.deleted_msgs.iter().any(|(da, dc, ds)| da == a && dc == c && *ds == m.seq))
            .map(|(_, _, m)| m.clone())
            .collect();
        all.sort_by_key(|m| m.seq);
        let keep = limit.clamp(1, MSG_READ_MAX);
        let start = all.len().saturating_sub(keep);
        Ok(all.split_off(start))
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
        self.check_access()?;
        if account.is_empty() || account.len() > MSG_ID_MAX {
            return Err(GuestSvcError::BadName);
        }
        if to.len() > MSG_RECIPIENTS_MAX || attachments.len() > MSG_ATTACHMENTS_MAX || body.len() > MSG_BODY_MAX {
            return Err(GuestSvcError::TooLarge);
        }
        if subject.len() > MSG_SUBJECT_MAX
            || subject.chars().any(char::is_control)
            || to.iter().any(|r| r.addr.is_empty() || r.addr.len() > MSG_ID_MAX || r.addr.chars().any(char::is_control))
            || attachments.iter().any(|a| !attachment_ref_ok(a))
        {
            return Err(GuestSvcError::Refused("bad-request"));
        }
        let id = match draft_id {
            Some(id) => {
                if !self.drafts.iter().any(|(a, d)| a == account && d.id == id) {
                    return Err(GuestSvcError::Refused("not-found"));
                }
                id
            }
            None => {
                if self.drafts.iter().filter(|(a, _)| a == account).count() >= MSG_DRAFTS_MAX {
                    return Err(GuestSvcError::TooLarge);
                }
                let id = self.next_draft;
                self.next_draft += 1;
                id
            }
        };
        let draft = GuestDraft {
            id,
            conv: conv.map(String::from),
            to: to.to_vec(),
            subject: String::from(subject),
            body: String::from(body),
            attachments: attachments.to_vec(),
        };
        if let Some((_, slot)) = self.drafts.iter_mut().find(|(a, d)| a == account && d.id == id) {
            *slot = draft;
        } else {
            self.drafts.push((String::from(account), draft));
        }
        Ok(id)
    }

    fn drafts(&mut self, account: &str) -> Result<Vec<GuestDraft>, GuestSvcError> {
        self.check_access()?;
        Ok(self.drafts.iter().filter(|(a, _)| a == account).map(|(_, d)| d.clone()).collect())
    }

    fn delete_draft(&mut self, account: &str, draft_id: u64) -> Result<(), GuestSvcError> {
        self.check_access()?;
        let Some(pos) = self.drafts.iter().position(|(a, d)| a == account && d.id == draft_id) else {
            return Err(GuestSvcError::Refused("not-found"));
        };
        self.drafts.remove(pos);
        Ok(())
    }

    fn send_draft(&mut self, account: &str, draft_id: u64) -> Result<u64, GuestSvcError> {
        self.check_access()?;
        let Some(pos) = self.drafts.iter().position(|(a, d)| a == account && d.id == draft_id) else {
            return Err(GuestSvcError::Refused("not-found"));
        };
        let d = &self.drafts[pos].1;
        if (d.to.is_empty() && d.conv.is_none()) || (d.body.is_empty() && d.attachments.is_empty()) {
            return Err(GuestSvcError::Refused("bad-request"));
        }
        let (_, draft) = self.drafts.remove(pos);
        let out_id = self.next_out;
        self.next_out += 1;
        self.sent.push((String::from(account), out_id, draft));
        Ok(out_id)
    }

    fn delete_msg(&mut self, account: &str, conv: &str, seq: u64) -> Result<(), GuestSvcError> {
        self.check_access()?;
        if seq == 0 || conv.is_empty() {
            return Err(GuestSvcError::Refused("not-found"));
        }
        self.deleted_msgs.push((String::from(account), String::from(conv), seq));
        Ok(())
    }
}
