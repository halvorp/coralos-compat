//! Native CoralOS `messages.service` host bridge ([`crate::OsVTable::attach_messages`], appended to OS ABI v3).
//!
//! Lets an engine process (`coral-heirloom-engine-rt::messages_service::MessagesService`, backed by
//! CoralOS `messages.service` on the loader-admitted reader lane) attach its messaging session to a
//! dynamically loaded `os-*.library` without crossing the DLL boundary with Rust trait objects or
//! cross-allocator frees. Inside `os-*.library`, [`MessagesHost`] implements `guestsvc_abi::GuestMessages`.
use core::ffi::c_void;

pub const MSG_ATTACH_OK: i32 = 0;
pub const MSG_ATTACH_REFUSED: i32 = 1;

pub const MSG_OK: i32 = 0;
pub const MSG_NOT_GRANTED: i32 = -1;
pub const MSG_BAD_NAME: i32 = -2;
pub const MSG_TOO_LARGE: i32 = -3;
pub const MSG_LOST: i32 = -4;
pub const MSG_STALLED: i32 = -5;
pub const MSG_UNREADABLE: i32 = -6;
pub const MSG_REFUSED_NOT_FOUND: i32 = -10;
pub const MSG_REFUSED_BAD_REQUEST: i32 = -11;
pub const MSG_REFUSED_NOT_YOURS: i32 = -12;
pub const MSG_REFUSED_OTHER: i32 = -13;

/// Flat C-ABI recipient entry lent across `save_draft`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MsgRecipientC {
    pub kind: u8,
    pub _pad: [u8; 7],
    pub addr: *const u8,
    pub addr_len: usize,
}

/// Flat C-ABI attachment ref lent across `save_draft`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MsgAttachmentC {
    pub path: *const u8,
    pub path_len: usize,
    pub mime: *const u8,
    pub mime_len: usize,
    pub name: *const u8,
    pub name_len: usize,
}

/// Synchronous visitor callback invoked once per account id string.
pub type AccountEmitFn = extern "C" fn(cb_ctx: *mut c_void, acct: *const u8, acct_len: usize);

/// Synchronous visitor callback invoked once per conversation info.
pub type ConvEmitFn = extern "C" fn(
    cb_ctx: *mut c_void,
    id: *const u8,
    id_len: usize,
    title: *const u8,
    title_len: usize,
    last_seq: u64,
    read_seq: u64,
);

/// Synchronous visitor callback invoked once per draft.
pub type DraftEmitFn = extern "C" fn(
    cb_ctx: *mut c_void,
    id: u64,
    conv: *const u8,
    conv_len: usize,
    to: *const MsgRecipientC,
    to_len: usize,
    subject: *const u8,
    subject_len: usize,
    body: *const u8,
    body_len: usize,
    attachments: *const MsgAttachmentC,
    attachments_len: usize,
);

/// Host-provided `messages.service` callbacks ([`crate::OsVTable::attach_messages`], copied; `ctx` and fn
/// pointers must outlive the `OsInstance`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MessagesHost {
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    pub accounts: extern "C" fn(ctx: *mut c_void, emit: AccountEmitFn, cb_ctx: *mut c_void) -> i32,
    pub conversations: extern "C" fn(
        ctx: *mut c_void,
        account: *const u8,
        account_len: usize,
        emit: ConvEmitFn,
        cb_ctx: *mut c_void,
    ) -> i32,
    pub save_draft: extern "C" fn(
        ctx: *mut c_void,
        account: *const u8,
        account_len: usize,
        draft_id: u64, // 0 = new draft; nonzero = update existing
        conv: *const u8,
        conv_len: usize,
        to: *const MsgRecipientC,
        to_len: usize,
        subject: *const u8,
        subject_len: usize,
        body: *const u8,
        body_len: usize,
        attachments: *const MsgAttachmentC,
        attachments_len: usize,
        out_id: *mut u64,
    ) -> i32,
    pub drafts: extern "C" fn(
        ctx: *mut c_void,
        account: *const u8,
        account_len: usize,
        emit: DraftEmitFn,
        cb_ctx: *mut c_void,
    ) -> i32,
    pub delete_draft: extern "C" fn(ctx: *mut c_void, account: *const u8, account_len: usize, draft_id: u64) -> i32,
    pub send_draft: extern "C" fn(
        ctx: *mut c_void,
        account: *const u8,
        account_len: usize,
        draft_id: u64,
        out_id: *mut u64,
    ) -> i32,
    pub delete_msg: extern "C" fn(
        ctx: *mut c_void,
        account: *const u8,
        account_len: usize,
        conv: *const u8,
        conv_len: usize,
        seq: u64,
    ) -> i32,
}

// SAFETY: MessagesHost holds plain function pointers and an engine-owned ctx pointer accessed on the single engine thread.
unsafe impl Send for MessagesHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;
    use guestsvc_abi::{
        GuestAttachmentRef, GuestConv, GuestDraft, GuestMessages, GuestRecipient, GuestRecipientKind, GuestSvcError,
    };

    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => MSG_NOT_GRANTED,
            GuestSvcError::BadName => MSG_BAD_NAME,
            GuestSvcError::TooLarge => MSG_TOO_LARGE,
            GuestSvcError::Lost => MSG_LOST,
            GuestSvcError::Stalled => MSG_STALLED,
            GuestSvcError::Unreadable => MSG_UNREADABLE,
            GuestSvcError::Refused("not-found") => MSG_REFUSED_NOT_FOUND,
            GuestSvcError::Refused("bad-request") => MSG_REFUSED_BAD_REQUEST,
            GuestSvcError::Refused("not-yours") => MSG_REFUSED_NOT_YOURS,
            GuestSvcError::Refused(_) => MSG_REFUSED_OTHER,
        }
    }

    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            MSG_NOT_GRANTED => GuestSvcError::NotGranted,
            MSG_BAD_NAME => GuestSvcError::BadName,
            MSG_TOO_LARGE => GuestSvcError::TooLarge,
            MSG_LOST => GuestSvcError::Lost,
            MSG_STALLED => GuestSvcError::Stalled,
            MSG_UNREADABLE => GuestSvcError::Unreadable,
            MSG_REFUSED_NOT_FOUND => GuestSvcError::Refused("not-found"),
            MSG_REFUSED_BAD_REQUEST => GuestSvcError::Refused("bad-request"),
            MSG_REFUSED_NOT_YOURS => GuestSvcError::Refused("not-yours"),
            _ => GuestSvcError::Refused("refused"),
        }
    }

    unsafe fn slice_str<'a>(p: *const u8, len: usize) -> &'a str {
        if p.is_null() || len == 0 {
            return "";
        }
        core::str::from_utf8(unsafe { core::slice::from_raw_parts(p, len) }).unwrap_or("")
    }

    extern "C" fn collect_account(cb_ctx: *mut c_void, acct: *const u8, acct_len: usize) {
        if cb_ctx.is_null() {
            return;
        }
        let out = unsafe { &mut *(cb_ctx as *mut Vec<String>) };
        out.push(String::from(unsafe { slice_str(acct, acct_len) }));
    }

    extern "C" fn collect_conv(
        cb_ctx: *mut c_void,
        id: *const u8,
        id_len: usize,
        title: *const u8,
        title_len: usize,
        last_seq: u64,
        read_seq: u64,
    ) {
        if cb_ctx.is_null() {
            return;
        }
        let out = unsafe { &mut *(cb_ctx as *mut Vec<GuestConv>) };
        out.push(GuestConv {
            id: String::from(unsafe { slice_str(id, id_len) }),
            title: String::from(unsafe { slice_str(title, title_len) }),
            last_seq,
            read_seq,
        });
    }

    extern "C" fn collect_draft(
        cb_ctx: *mut c_void,
        id: u64,
        conv: *const u8,
        conv_len: usize,
        to: *const MsgRecipientC,
        to_len: usize,
        subject: *const u8,
        subject_len: usize,
        body: *const u8,
        body_len: usize,
        attachments: *const MsgAttachmentC,
        attachments_len: usize,
    ) {
        if cb_ctx.is_null() {
            return;
        }
        let out = unsafe { &mut *(cb_ctx as *mut Vec<GuestDraft>) };
        let conv_s = unsafe { slice_str(conv, conv_len) };
        let mut recips = Vec::with_capacity(to_len);
        if !to.is_null() && to_len > 0 {
            for r in unsafe { core::slice::from_raw_parts(to, to_len) } {
                recips.push(GuestRecipient {
                    kind: GuestRecipientKind::from_u8(r.kind).unwrap_or(GuestRecipientKind::To),
                    addr: String::from(unsafe { slice_str(r.addr, r.addr_len) }),
                });
            }
        }
        let mut atts = Vec::with_capacity(attachments_len);
        if !attachments.is_null() && attachments_len > 0 {
            for a in unsafe { core::slice::from_raw_parts(attachments, attachments_len) } {
                atts.push(GuestAttachmentRef {
                    path: String::from(unsafe { slice_str(a.path, a.path_len) }),
                    mime: String::from(unsafe { slice_str(a.mime, a.mime_len) }),
                    name: String::from(unsafe { slice_str(a.name, a.name_len) }),
                });
            }
        }
        out.push(GuestDraft {
            id,
            conv: (!conv_s.is_empty()).then(|| String::from(conv_s)),
            to: recips,
            subject: String::from(unsafe { slice_str(subject, subject_len) }),
            body: String::from(unsafe { slice_str(body, body_len) }),
            attachments: atts,
        });
    }

    impl GuestMessages for MessagesHost {
        fn accounts(&mut self) -> Result<Vec<String>, GuestSvcError> {
            let mut out = Vec::new();
            let rc = (self.accounts)(self.ctx, collect_account, (&mut out as *mut Vec<String>).cast());
            if rc == MSG_OK { Ok(out) } else { Err(code_to_err(rc)) }
        }

        fn conversations(&mut self, account: &str) -> Result<Vec<GuestConv>, GuestSvcError> {
            let mut out = Vec::new();
            let rc = (self.conversations)(
                self.ctx,
                account.as_ptr(),
                account.len(),
                collect_conv,
                (&mut out as *mut Vec<GuestConv>).cast(),
            );
            if rc == MSG_OK { Ok(out) } else { Err(code_to_err(rc)) }
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
            let to_c: Vec<MsgRecipientC> = to
                .iter()
                .map(|r| MsgRecipientC { kind: r.kind as u8, _pad: [0; 7], addr: r.addr.as_ptr(), addr_len: r.addr.len() })
                .collect();
            let att_c: Vec<MsgAttachmentC> = attachments
                .iter()
                .map(|a| MsgAttachmentC {
                    path: a.path.as_ptr(),
                    path_len: a.path.len(),
                    mime: a.mime.as_ptr(),
                    mime_len: a.mime.len(),
                    name: a.name.as_ptr(),
                    name_len: a.name.len(),
                })
                .collect();
            let (cp, cl) = conv.map_or((core::ptr::null(), 0), |c| (c.as_ptr(), c.len()));
            let mut out_id = 0u64;
            let rc = (self.save_draft)(
                self.ctx,
                account.as_ptr(),
                account.len(),
                draft_id.unwrap_or(0),
                cp,
                cl,
                to_c.as_ptr(),
                to_c.len(),
                subject.as_ptr(),
                subject.len(),
                body.as_ptr(),
                body.len(),
                att_c.as_ptr(),
                att_c.len(),
                &mut out_id,
            );
            if rc == MSG_OK { Ok(out_id) } else { Err(code_to_err(rc)) }
        }

        fn drafts(&mut self, account: &str) -> Result<Vec<GuestDraft>, GuestSvcError> {
            let mut out = Vec::new();
            let rc = (self.drafts)(
                self.ctx,
                account.as_ptr(),
                account.len(),
                collect_draft,
                (&mut out as *mut Vec<GuestDraft>).cast(),
            );
            if rc == MSG_OK { Ok(out) } else { Err(code_to_err(rc)) }
        }

        fn delete_draft(&mut self, account: &str, draft_id: u64) -> Result<(), GuestSvcError> {
            let rc = (self.delete_draft)(self.ctx, account.as_ptr(), account.len(), draft_id);
            if rc == MSG_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn send_draft(&mut self, account: &str, draft_id: u64) -> Result<u64, GuestSvcError> {
            let mut out_id = 0u64;
            let rc = (self.send_draft)(self.ctx, account.as_ptr(), account.len(), draft_id, &mut out_id);
            if rc == MSG_OK { Ok(out_id) } else { Err(code_to_err(rc)) }
        }

        fn delete_msg(&mut self, account: &str, conv: &str, seq: u64) -> Result<(), GuestSvcError> {
            let rc = (self.delete_msg)(self.ctx, account.as_ptr(), account.len(), conv.as_ptr(), conv.len(), seq);
            if rc == MSG_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, err_to_code};
