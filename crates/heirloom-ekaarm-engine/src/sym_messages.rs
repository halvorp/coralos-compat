//! Stateless Symbian messaging adapter (`msgs`, `sendas2`, `sendui`) over `guestsvc_abi::GuestMessages`
//! (CoralOS `messages.service`, REUSE-AUDIT U15 / `docs/heirloom/21-SHARED-SERVICES.md`).
//!
//! - Holds ZERO persistent message store inside the engine: only transient in-flight compose handles
//!   (`SendAsDraftState` for open `RSendAsMessage`, `MessageDataState` for `CMessageData`) before
//!   `SaveMessageAndCloseL` / `SendMessageAndCloseL` / `CreateAndSendMessageL` flushes to `messages.service`.
//! - Radio/device bearers (SMS `0x1000102C`, MMS `0x100058E1`, WAP push `0x10005522`, BT `0x10009ED5`,
//!   IrDA `0x100053A4`) are REFUSED BY NAME at the send step (`KErrNotSupported` = `-5`) while preserving
//!   the saved draft in `messages.service`.
//! - Without a `(messages)` grant / loader admission (`GuestSvcError::NotGranted` or unattached host),
//!   calls fail cleanly with `KErrPermissionDenied` (`-46`) and NEVER fall back to private engine RAM.
use alloc::format;
use alloc::string::String;
use guestsvc_abi::{GuestAttachmentRef, GuestRecipient, GuestRecipientKind, GuestSvcError};
use crate::cleanup::CleanupItem;
use crate::desc;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{HostObj, MessageDataState, SendAsDraftState};

pub const KERR_NONE: i32 = 0;
pub const KERR_NOT_FOUND: i32 = -1;
pub const KERR_NOT_SUPPORTED: i32 = -5;
pub const KERR_ARGUMENT: i32 = -6;
pub const KERR_SERVER_TERMINATED: i32 = -15;
pub const KERR_OVERFLOW: i32 = -22;
pub const KERR_PERMISSION_DENIED: i32 = -46;

/// Standard Symbian S60 MTM / SendUI service UIDs for radio / device bearers (refused at send).
pub const KUID_MSG_TYPE_SMS: u32 = 0x1000_102C;
pub const KUID_MSG_TYPE_MMS: u32 = 0x1000_58E1;
pub const KUID_MSG_TYPE_WAP_PUSH: u32 = 0x1000_5522;
pub const KUID_MSG_TYPE_BT: u32 = 0x1000_9ED5;
pub const KUID_MSG_TYPE_IR: u32 = 0x1000_53A4;

/// Standard Symbian S60 SMTP email MTM UID (non-radio: routed through `messages.service`).
pub const KUID_MSG_TYPE_SMTP: u32 = 0x1000_1028;

/// If `mtm_uid` is a radio or device bearer, return its canonical refusal name.
pub const fn radio_bearer_name(mtm_uid: u32) -> Option<&'static str> {
    match mtm_uid {
        KUID_MSG_TYPE_SMS => Some("SMS radio bearer (KUidMsgTypeSMS)"),
        KUID_MSG_TYPE_MMS => Some("MMS radio bearer (KUidMsgTypeMultimedia)"),
        KUID_MSG_TYPE_WAP_PUSH => Some("WAP push radio bearer (KUidMsgTypeWapPush)"),
        KUID_MSG_TYPE_BT => Some("Bluetooth bearer (KUidMsgTypeBt)"),
        KUID_MSG_TYPE_IR => Some("IrDA bearer (KUidMsgTypeIr)"),
        _ => None,
    }
}

/// The guest's error code for a service refusal (shared with `sym_msv.rs`).
pub(crate) fn map_svc_err(err: GuestSvcError) -> i32 {
    match err {
        GuestSvcError::NotGranted => KERR_PERMISSION_DENIED,
        GuestSvcError::TooLarge => KERR_OVERFLOW,
        GuestSvcError::Refused("not-found") => KERR_NOT_FOUND,
        GuestSvcError::Refused("not-yours") => KERR_PERMISSION_DENIED,
        GuestSvcError::BadName | GuestSvcError::Refused(_) => KERR_ARGUMENT,
        GuestSvcError::Lost | GuestSvcError::Stalled | GuestSvcError::Unreadable => KERR_SERVER_TERMINATED,
    }
}

fn leave_or_return(proc: &mut Process, name: &str, code: i32, detail: String) -> NativeOutcome {
    proc.log.push(format!("[messages] {name}: {detail} -> {code}"));
    if !proc.exception_descriptors.is_empty() {
        crate::ehabi::leave(proc, code)
    } else {
        proc.cpu.regs[0] = code as u32;
        NativeOutcome::Return
    }
}

/// Normalize a Symbian file path (e.g. `C:\Data\notes.txt` or `Users/Default/Documents/a.txt`) into a
/// CoralFS path ref without silently rewriting `..` traversal (which stays rejected by `messages.service`).
fn symbian_path_to_attachment(path: &str, mime: &str) -> GuestAttachmentRef {
    let slash = path.replace('\\', "/");
    let trimmed = if slash.len() >= 2 && slash.as_bytes()[1] == b':' {
        slash[2..].trim_start_matches('/')
    } else {
        slash.as_str()
    };
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    GuestAttachmentRef {
        path: String::from(trimmed),
        mime: String::from(mime),
        name: String::from(name),
    }
}

fn recipient_kind_from_symbian(code: u32) -> GuestRecipientKind {
    match code {
        1 => GuestRecipientKind::Cc,
        2 => GuestRecipientKind::Bcc,
        _ => GuestRecipientKind::To,
    }
}

fn resolve_account(proc: &mut Process) -> Result<String, i32> {
    let Some(host) = proc.messages_host.as_mut() else {
        return Err(KERR_PERMISSION_DENIED);
    };
    let accts = host.accounts().map_err(map_svc_err)?;
    accts.into_iter().next().ok_or(KERR_NOT_FOUND)
}

fn complete_request_status(proc: &mut Process, status_ptr: u32, code: i32) {
    if status_ptr != 0 {
        let _ = proc.wr(status_ptr, code as u32);
    }
}

fn persist_draft(proc: &mut Process, draft: &mut SendAsDraftState) -> Result<(String, u64), i32> {
    let account = resolve_account(proc)?;
    let Some(host) = proc.messages_host.as_mut() else {
        return Err(KERR_PERMISSION_DENIED);
    };
    let id = host
        .save_draft(
            &account,
            draft.saved_id,
            None,
            &draft.to,
            &draft.subject,
            &draft.body,
            &draft.attachments,
        )
        .map_err(map_svc_err)?;
    draft.saved_id = Some(id);
    Ok((account, id))
}

fn send_draft_with_bearer_check(proc: &mut Process, draft: &mut SendAsDraftState) -> Result<u64, (i32, String)> {
    let (account, id) = persist_draft(proc, draft)
        .map_err(|code| (code, String::from("save_draft refused by messages.service")))?;
    if let Some(bearer) = radio_bearer_name(draft.mtm_uid) {
        return Err((
            KERR_NOT_SUPPORTED,
            format!("radio bearer refused by name: {bearer} (draft {id} preserved in messages.service)"),
        ));
    }
    let Some(host) = proc.messages_host.as_mut() else {
        return Err((KERR_PERMISSION_DENIED, String::from("no messages.service host")));
    };
    host.send_draft(&account, id)
        .map_err(|e| (map_svc_err(e), format!("send_draft refused ({})", e.name())))
}

const MSGDATA_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN12CMessageDataD1Ev"),
    (1, "_ZN12CMessageDataD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
];

const SENDUI_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN7CSendUiD1Ev"),
    (1, "_ZN7CSendUiD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
];

pub fn native(proc: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2, r3) = (proc.cpu.regs[0], proc.cpu.regs[1], proc.cpu.regs[2], proc.cpu.regs[3]);
    match name {
        // ---- msgs.dll: CMsvSession ----
        "_ZN11CMsvSession9OpenSyncLER19MMsvSessionObserver"
        | "_ZN11CMsvSession10OpenAsyncLER19MMsvSessionObserver"
        | "_ZN11CMsvSession15OpenAsObserverLER19MMsvSessionObserver" => {
            if let Err(code) = resolve_account(proc) {
                return Some(leave_or_return(proc, name, code, String::from("messages.service not granted or no account")));
            }
            match proc.plain_object("CMsvSession", 0x20) {
                Ok(obj) => {
                    proc.cpu.regs[0] = obj;
                    Some(NativeOutcome::Return)
                }
                Err(m) => Some(NativeOutcome::Refused(format!("NEXT-NEEDED msgs {name}: {m}"))),
            }
        }
        "_ZN11CMsvSession18CloseMessageServerEv" => Some(NativeOutcome::Return),
        "_ZN11CMsvSession11RemoveEntryEl" => {
            // r0 = this, r1 = entry_id (maps to draft_id in messages.service)
            let draft_id = u64::from(r1);
            let res = resolve_account(proc).and_then(|acct| {
                let Some(host) = proc.messages_host.as_mut() else {
                    return Err(KERR_PERMISSION_DENIED);
                };
                host.delete_draft(&acct, draft_id).map_err(map_svc_err)
            });
            proc.cpu.regs[0] = res.err().unwrap_or(KERR_NONE) as u32;
            Some(NativeOutcome::Return)
        }

        // ---- sendas2.dll: RSendAs ----
        "_ZN7RSendAs7ConnectEv" | "_ZN7RSendAs7ConnectEi" => {
            match resolve_account(proc) {
                Ok(acct) => {
                    let h = proc.nat.objs.next_sendas_handle.wrapping_add(1).max(1);
                    proc.nat.objs.next_sendas_handle = h;
                    proc.nat.objs.sendas_sessions.insert(h, acct);
                    let _ = proc.wr(r0, h);
                    proc.cpu.regs[0] = KERR_NONE as u32;
                    Some(NativeOutcome::Return)
                }
                Err(code) => {
                    let _ = proc.wr(r0, 0);
                    proc.log.push(format!("[messages] {name}: refused ({code})"));
                    proc.cpu.regs[0] = code as u32;
                    Some(NativeOutcome::Return)
                }
            }
        }

        // ---- sendas2.dll: RSendAsMessage ----
        "_ZN14RSendAsMessage7CreateLER7RSendAs4TUid" | "_ZN14RSendAsMessage7CreateLER7RSendAsl" => {
            let sess_h = proc.rd(r1).unwrap_or(0);
            if !proc.nat.objs.sendas_sessions.contains_key(&sess_h) {
                return Some(leave_or_return(proc, name, KERR_PERMISSION_DENIED, String::from("RSendAs handle not connected")));
            }
            let h = proc.nat.objs.next_sendas_handle.wrapping_add(1).max(1);
            proc.nat.objs.next_sendas_handle = h;
            proc.nat.objs.sendas_drafts.insert(h, SendAsDraftState { mtm_uid: r2, ..Default::default() });
            let _ = proc.wr(r0, h);
            proc.cpu.regs[0] = KERR_NONE as u32;
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage13AddRecipientLERK7TDesC16NS_20TSendAsRecipientTypeE" => {
            let h = proc.rd(r0).unwrap_or(0);
            let addr = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
            };
            let kind = recipient_kind_from_symbian(r2);
            let Some(d) = proc.nat.objs.sendas_drafts.get_mut(&h) else {
                return Some(leave_or_return(proc, name, KERR_NOT_FOUND, String::from("no open RSendAsMessage")));
            };
            d.to.push(GuestRecipient { kind, addr });
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage13AddRecipientLERK7TDesC16S2_NS_20TSendAsRecipientTypeE" => {
            let h = proc.rd(r0).unwrap_or(0);
            let addr = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
            };
            let kind = recipient_kind_from_symbian(r3);
            let Some(d) = proc.nat.objs.sendas_drafts.get_mut(&h) else {
                return Some(leave_or_return(proc, name, KERR_NOT_FOUND, String::from("no open RSendAsMessage")));
            };
            d.to.push(GuestRecipient { kind, addr });
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage11SetSubjectLERK7TDesC16" => {
            let h = proc.rd(r0).unwrap_or(0);
            let subj = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
            };
            let Some(d) = proc.nat.objs.sendas_drafts.get_mut(&h) else {
                return Some(leave_or_return(proc, name, KERR_NOT_FOUND, String::from("no open RSendAsMessage")));
            };
            d.subject = subj;
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage12SetBodyTextLERK7TDesC16" => {
            let h = proc.rd(r0).unwrap_or(0);
            let body = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
            };
            let Some(d) = proc.nat.objs.sendas_drafts.get_mut(&h) else {
                return Some(leave_or_return(proc, name, KERR_NOT_FOUND, String::from("no open RSendAsMessage")));
            };
            d.body = body;
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage13AddAttachmentERK7TDesC16R14TRequestStatus"
        | "_ZN14RSendAsMessage19AddLinkedAttachmentERK7TDesC16R14TRequestStatus" => {
            let h = proc.rd(r0).unwrap_or(0);
            let path = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(_) => {
                    complete_request_status(proc, r2, KERR_ARGUMENT);
                    return Some(NativeOutcome::Return);
                }
            };
            if let Some(d) = proc.nat.objs.sendas_drafts.get_mut(&h) {
                d.attachments.push(symbian_path_to_attachment(&path, ""));
                complete_request_status(proc, r2, KERR_NONE);
            } else {
                complete_request_status(proc, r2, KERR_NOT_FOUND);
            }
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage13AddAttachmentERK7TDesC16RK6TDesC8R14TRequestStatus"
        | "_ZN14RSendAsMessage19AddLinkedAttachmentERK7TDesC16RK6TDesC8R14TRequestStatus" => {
            let h = proc.rd(r0).unwrap_or(0);
            let path = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(_) => {
                    complete_request_status(proc, r3, KERR_ARGUMENT);
                    return Some(NativeOutcome::Return);
                }
            };
            let mime = desc::read8(proc, r2)
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            if let Some(d) = proc.nat.objs.sendas_drafts.get_mut(&h) {
                d.attachments.push(symbian_path_to_attachment(&path, &mime));
                complete_request_status(proc, r3, KERR_NONE);
            } else {
                complete_request_status(proc, r3, KERR_NOT_FOUND);
            }
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage20SaveMessageAndCloseLEv" => {
            let h = proc.rd(r0).unwrap_or(0);
            let Some(mut d) = proc.nat.objs.sendas_drafts.remove(&h) else {
                return Some(leave_or_return(proc, name, KERR_NOT_FOUND, String::from("no open RSendAsMessage")));
            };
            let _ = proc.wr(r0, 0);
            match persist_draft(proc, &mut d) {
                Ok((_, id)) => {
                    proc.log.push(format!("[messages] SaveMessageAndCloseL -> draft {id}"));
                    proc.cpu.regs[0] = KERR_NONE as u32;
                    Some(NativeOutcome::Return)
                }
                Err(code) => Some(leave_or_return(proc, name, code, String::from("SaveMessageAndCloseL failed"))),
            }
        }
        "_ZN14RSendAsMessage20SendMessageAndCloseLEv"
        | "_ZN14RSendAsMessage29SendMessageConfirmedAndCloseLEv" => {
            let h = proc.rd(r0).unwrap_or(0);
            let Some(mut d) = proc.nat.objs.sendas_drafts.remove(&h) else {
                return Some(leave_or_return(proc, name, KERR_NOT_FOUND, String::from("no open RSendAsMessage")));
            };
            let _ = proc.wr(r0, 0);
            match send_draft_with_bearer_check(proc, &mut d) {
                Ok(out_id) => {
                    proc.log.push(format!("[messages] {name} -> queued {out_id}"));
                    proc.cpu.regs[0] = KERR_NONE as u32;
                    Some(NativeOutcome::Return)
                }
                Err((code, why)) => Some(leave_or_return(proc, name, code, why)),
            }
        }
        "_ZN14RSendAsMessage11SendMessageER14TRequestStatus"
        | "_ZN14RSendAsMessage20SendMessageConfirmedER14TRequestStatus" => {
            let h = proc.rd(r0).unwrap_or(0);
            let Some(mut d) = proc.nat.objs.sendas_drafts.remove(&h) else {
                complete_request_status(proc, r1, KERR_NOT_FOUND);
                return Some(NativeOutcome::Return);
            };
            match send_draft_with_bearer_check(proc, &mut d) {
                Ok(out_id) => {
                    proc.log.push(format!("[messages] {name} -> queued {out_id}"));
                    complete_request_status(proc, r1, KERR_NONE);
                }
                Err((code, why)) => {
                    proc.log.push(format!("[messages] {name}: {why} -> {code}"));
                    complete_request_status(proc, r1, code);
                }
            }
            Some(NativeOutcome::Return)
        }
        "_ZN14RSendAsMessage5CloseEv" | "_ZN14RSendAsMessage6CancelEv" => {
            let h = proc.rd(r0).unwrap_or(0);
            proc.nat.objs.sendas_drafts.remove(&h);
            let _ = proc.wr(r0, 0);
            Some(NativeOutcome::Return)
        }

        // ---- sendui.dll: CMessageData + CSendUi ----
        "_ZN12CMessageData4NewLEv" | "_ZN12CMessageData5NewLCEv" => {
            let obj = match proc.alloc_zeroed(16) {
                Some(o) => o,
                None => return Some(leave_or_return(proc, name, -4, String::from("no heap for CMessageData"))),
            };
            let vptr = match proc.build_vtable("CMessageData", &slots_with_gaps(3, MSGDATA_VTABLE)) {
                Ok(v) => v,
                Err(e) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED sendui {name}: {e}"))),
            };
            let _ = proc.wr(obj, vptr);
            let _ = proc.obj_set(obj, HostObj::MessageData(MessageDataState::default()));
            if name.ends_with("5NewLCEv") {
                proc.nat.cleanup.push(CleanupItem::Base(obj));
            }
            proc.cpu.regs[0] = obj;
            Some(NativeOutcome::Return)
        }
        "_ZN12CMessageDataD0Ev" | "_ZN12CMessageDataD1Ev" | "_ZN12CMessageDataD2Ev" => {
            proc.obj_remove(r0);
            if name.ends_with("D0Ev") {
                if let Some(h) = proc.heap.as_mut() {
                    let _ = h.free(r0);
                }
            }
            proc.cpu.regs[0] = r0;
            Some(NativeOutcome::Return)
        }
        "_ZN12CMessageData11SetSubjectLEPK7TDesC16" => {
            if r1 != 0 {
                let subj = match desc::read16(proc, r1) {
                    Ok(u) => desc::to_utf8(&u),
                    Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
                };
                if let Some(HostObj::MessageData(md)) = proc.obj_get(r0) {
                    md.subject = subj;
                }
            }
            Some(NativeOutcome::Return)
        }
        "_ZN12CMessageData16AppendToAddressLERK7TDesC16S2_"
        | "_ZN12CMessageData16AppendCcAddressLERK7TDesC16S2_"
        | "_ZN12CMessageData17AppendBccAddressLERK7TDesC16S2_" => {
            let addr = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
            };
            let kind = if name.contains("CcAddress") {
                GuestRecipientKind::Cc
            } else if name.contains("BccAddress") {
                GuestRecipientKind::Bcc
            } else {
                GuestRecipientKind::To
            };
            if let Some(HostObj::MessageData(md)) = proc.obj_get(r0) {
                md.to.push(GuestRecipient { kind, addr });
            }
            Some(NativeOutcome::Return)
        }
        "_ZN12CMessageData17AppendAttachmentLERK7TDesC16" => {
            let path = match desc::read16(proc, r1) {
                Ok(u) => desc::to_utf8(&u),
                Err(e) => return Some(leave_or_return(proc, name, KERR_ARGUMENT, e)),
            };
            if let Some(HostObj::MessageData(md)) = proc.obj_get(r0) {
                md.attachments.push(symbian_path_to_attachment(&path, ""));
            }
            Some(NativeOutcome::Return)
        }
        "_ZN12CMessageData20ClearAttachmentArrayEv" => {
            if let Some(HostObj::MessageData(md)) = proc.obj_get(r0) {
                md.attachments.clear();
            }
            Some(NativeOutcome::Return)
        }
        "_ZN7CSendUi4NewLEv" | "_ZN7CSendUi5NewLCEv" => {
            let obj = match proc.alloc_zeroed(16) {
                Some(o) => o,
                None => return Some(leave_or_return(proc, name, -4, String::from("no heap for CSendUi"))),
            };
            let vptr = match proc.build_vtable("CSendUi", &slots_with_gaps(3, SENDUI_VTABLE)) {
                Ok(v) => v,
                Err(e) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED sendui {name}: {e}"))),
            };
            let _ = proc.wr(obj, vptr);
            if name.ends_with("5NewLCEv") {
                proc.nat.cleanup.push(CleanupItem::Base(obj));
            }
            proc.cpu.regs[0] = obj;
            Some(NativeOutcome::Return)
        }
        "_ZN7CSendUiD0Ev" | "_ZN7CSendUiD1Ev" | "_ZN7CSendUiD2Ev" => {
            if name.ends_with("D0Ev") {
                if let Some(h) = proc.heap.as_mut() {
                    let _ = h.free(r0);
                }
            }
            proc.cpu.regs[0] = r0;
            Some(NativeOutcome::Return)
        }
        "_ZN7CSendUi21CreateAndSendMessageLE4TUidPK12CMessageDataS0_i" => {
            // r0 = this, r1 = aServiceUid, r2 = aMessageData
            let md = match proc.obj_get(r2) {
                Some(HostObj::MessageData(m)) => m.clone(),
                _ => MessageDataState::default(),
            };
            let mut draft = SendAsDraftState {
                mtm_uid: r1,
                to: md.to,
                subject: md.subject,
                body: md.body,
                attachments: md.attachments,
                saved_id: None,
            };
            match send_draft_with_bearer_check(proc, &mut draft) {
                Ok(out_id) => {
                    proc.log.push(format!("[messages] CSendUi::CreateAndSendMessageL uid={r1:#x} -> queued {out_id}"));
                    proc.cpu.regs[0] = KERR_NONE as u32;
                    Some(NativeOutcome::Return)
                }
                Err((code, why)) => Some(leave_or_return(proc, name, code, why)),
            }
        }
        "_ZNK7CSendUi17CanSendBioMessageE4TUid" => {
            proc.cpu.regs[0] = 0; // EFalse: BIO/SMS radio messages are not supported
            Some(NativeOutcome::Return)
        }
        _ => None,
    }
}
