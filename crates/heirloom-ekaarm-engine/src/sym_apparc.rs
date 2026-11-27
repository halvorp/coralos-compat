//! Stateless Symbian `RApaLsSession` (Application Architecture / MIME recognizer & document handler)
//! adapter over `guestsvc_abi::GuestAppArc` (CoralOS `coral-descriptor` + `coral-opener`, REUSE-AUDIT U19).
//!
//! Replaces the old `RApaLsSession` placeholders with real CoralOS content sniffing and opener lookup:
//! * `RecognizeData(name, buf, TDataRecognitionResult&)` -> `GuestAppArc::recognize_data` (`coral-descriptor`)
//! * `AppForDataType(TDataType, TUid&)` -> `GuestAppArc::handler_for` (`coral-opener::choose`)
//! * `AppForDocument(fileName, TUid&, TDataType&)` -> `recognize_data` + `handler_for`
//! * `StartDocument(fileName, ...)` -> resolves handler through `coral-opener` (refuses `KErrNotFound` if none)
use alloc::format;
use alloc::string::String;

use guestsvc_abi::{GuestAppArc, GuestSvcError, CONF_NOT_RECOGNIZED};

use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const KERR_NONE: i32 = 0;
pub const KERR_NOT_FOUND: i32 = -1;
pub const KERR_ARGUMENT: i32 = -6;
pub const KERR_PERMISSION_DENIED: i32 = -46;

/// `TDataRecognitionResult` ARM EABI layout:
/// `0..264`: `TDataType iDataType` (`TBuf8<256>` = `EBuf` header 8 bytes + 256 bytes data)
/// `264..268`: `TInt iConfidence`
pub const RECOG_CONFIDENCE_OFF: u32 = 264;

/// Deterministic non-zero Symbian `TUid` derived from a CoralOS bundle stem (FNV-1a 32-bit, forced non-zero).
pub fn stem_to_uid(stem: &str) -> u32 {
    let mut h = 0x811C_9DC5u32;
    for b in stem.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    if h == 0 { 1 } else { h }
}

fn map_err(e: GuestSvcError) -> i32 {
    match e {
        GuestSvcError::NotGranted => KERR_PERMISSION_DENIED,
        GuestSvcError::BadName => KERR_ARGUMENT,
        _ => KERR_NOT_FOUND,
    }
}

fn host_mut(p: &mut Process) -> Result<&mut (dyn GuestAppArc + 'static), i32> {
    match p.apparc_host.as_mut() {
        Some(b) => Ok(&mut **b),
        None => Err(KERR_PERMISSION_DENIED),
    }
}

pub fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if !name.starts_with("_ZN13RApaLsSession") && !name.starts_with("_ZNK13RApaLsSession") {
        return None;
    }

    let r0 = p.cpu.regs[0];
    let r1 = p.cpu.regs[1];
    let r2 = p.cpu.regs[2];
    let r3 = p.cpu.regs[3];

    if name == "_ZN13RApaLsSession7ConnectEv" {
        let _ = p.wr(r0, 0x5241_5041); // 'RAPA'
        p.cpu.regs[0] = KERR_NONE as u32;
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN13RApaLsSession5CloseEv" {
        let _ = p.wr(r0, 0);
        return Some(NativeOutcome::Return);
    }
    if name == "_ZNK13RApaLsSession19ApplicationLanguageE4TUidR9TLanguage" {
        let _ = p.wr(r2, 1); // ELangEnglish
        p.cpu.regs[0] = KERR_NONE as u32;
        return Some(NativeOutcome::Return);
    }

    // TInt RApaLsSession::RecognizeData(const TDesC16& aName, const TDesC8& aBuffer, TDataRecognitionResult& aDataType) const
    if name.starts_with("_ZN13RApaLsSession13RecognizeData")
        || name.starts_with("_ZNK13RApaLsSession13RecognizeData")
    {
        let doc_name = match crate::desc::read16(p, r1) {
            Ok(u) => crate::desc::to_utf8(&u),
            Err(_) => {
                p.cpu.regs[0] = KERR_ARGUMENT as u32;
                return Some(NativeOutcome::Return);
            }
        };
        let head = match crate::desc::read8(p, r2) {
            Ok(b) => b,
            Err(_) => {
                p.cpu.regs[0] = KERR_ARGUMENT as u32;
                return Some(NativeOutcome::Return);
            }
        };
        let res = match host_mut(p) {
            Ok(h) => h.recognize_data(&doc_name, &head).map_err(map_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(info) => {
                let _ = crate::desc::write8(p, r3, info.mime.as_bytes());
                let _ = p.wr(r3 + RECOG_CONFIDENCE_OFF, info.confidence as u32);
                p.log.push(format!("[apparc] RecognizeData {doc_name:?} -> {:?} conf={}", info.mime, info.confidence));
                p.cpu.regs[0] = KERR_NONE as u32;
            }
            Err(code) => {
                let _ = p.wr(r3 + RECOG_CONFIDENCE_OFF, CONF_NOT_RECOGNIZED as u32);
                p.log.push(format!("[apparc] RecognizeData {doc_name:?} -> {code}"));
                p.cpu.regs[0] = code as u32;
            }
        }
        return Some(NativeOutcome::Return);
    }

    // TInt RApaLsSession::AppForDataType(const TDataType& aDataType, TUid& aAppUid) const
    if name.starts_with("_ZN13RApaLsSession14AppForDataType")
        || name.starts_with("_ZNK13RApaLsSession14AppForDataType")
    {
        let mime_bytes = match crate::desc::read8(p, r1) {
            Ok(b) => b,
            Err(_) => {
                p.cpu.regs[0] = KERR_ARGUMENT as u32;
                return Some(NativeOutcome::Return);
            }
        };
        let mime = String::from_utf8_lossy(&mime_bytes).into_owned();
        let res = match host_mut(p) {
            Ok(h) => h.handler_for(&mime, "").map_err(map_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(Some(stem)) => {
                let uid = stem_to_uid(&stem);
                let _ = p.wr(r2, uid);
                p.log.push(format!("[apparc] AppForDataType {mime:?} -> {stem:?} (uid={uid:#010x})"));
                p.cpu.regs[0] = KERR_NONE as u32;
            }
            Ok(None) => {
                let _ = p.wr(r2, 0);
                p.log.push(format!("[apparc] AppForDataType {mime:?} -> KErrNotFound"));
                p.cpu.regs[0] = KERR_NOT_FOUND as u32;
            }
            Err(code) => {
                let _ = p.wr(r2, 0);
                p.cpu.regs[0] = code as u32;
            }
        }
        return Some(NativeOutcome::Return);
    }

    // TInt RApaLsSession::AppForDocument(const TDesC16& aFileName, TUid& aAppUid, TDataType& aDataType) const
    if name.starts_with("_ZN13RApaLsSession14AppForDocument")
        || name.starts_with("_ZNK13RApaLsSession14AppForDocument")
    {
        let doc_name = match crate::desc::read16(p, r1) {
            Ok(u) => crate::desc::to_utf8(&u),
            Err(_) => {
                p.cpu.regs[0] = KERR_ARGUMENT as u32;
                return Some(NativeOutcome::Return);
            }
        };
        let head = p
            .host
            .drive
            .as_ref()
            .and_then(|d| d.read_owned(&doc_name).ok())
            .map(|mut v| {
                v.truncate(512);
                v
            })
            .unwrap_or_default();
        let recog = match host_mut(p) {
            Ok(h) => h.recognize_data(&doc_name, &head).map_err(map_err),
            Err(e) => Err(e),
        };
        let info = match recog {
            Ok(i) => i,
            Err(code) => {
                let _ = p.wr(r2, 0);
                p.cpu.regs[0] = code as u32;
                return Some(NativeOutcome::Return);
            }
        };
        let _ = crate::desc::write8(p, r3, info.mime.as_bytes());
        let handler = match host_mut(p) {
            Ok(h) => h.handler_for(&info.mime, &doc_name).map_err(map_err),
            Err(e) => Err(e),
        };
        match handler {
            Ok(Some(stem)) => {
                let uid = stem_to_uid(&stem);
                let _ = p.wr(r2, uid);
                p.log.push(format!("[apparc] AppForDocument {doc_name:?} ({:?}) -> {stem:?} (uid={uid:#010x})", info.mime));
                p.cpu.regs[0] = KERR_NONE as u32;
            }
            Ok(None) => {
                let _ = p.wr(r2, 0);
                p.cpu.regs[0] = KERR_NOT_FOUND as u32;
            }
            Err(code) => {
                let _ = p.wr(r2, 0);
                p.cpu.regs[0] = code as u32;
            }
        }
        return Some(NativeOutcome::Return);
    }

    // TInt RApaLsSession::StartDocument(const TDesC16& aFileName, ...)
    if name.starts_with("_ZN13RApaLsSession13StartDocument") {
        let doc_name = match crate::desc::read16(p, r1) {
            Ok(u) => crate::desc::to_utf8(&u),
            Err(_) => {
                p.cpu.regs[0] = KERR_ARGUMENT as u32;
                return Some(NativeOutcome::Return);
            }
        };
        let head = p
            .host
            .drive
            .as_ref()
            .and_then(|d| d.read_owned(&doc_name).ok())
            .map(|mut v| {
                v.truncate(512);
                v
            })
            .unwrap_or_default();
        let res = match host_mut(p) {
            Ok(h) => h.recognize_data(&doc_name, &head).and_then(|info| h.handler_for(&info.mime, &doc_name)).map_err(map_err),
            Err(e) => Err(e),
        };
        match res {
            Ok(Some(stem)) => {
                p.log.push(format!("[apparc] StartDocument {doc_name:?} -> resolved CoralOS handler {stem:?}"));
                p.cpu.regs[0] = KERR_NONE as u32;
            }
            Ok(None) => {
                p.log.push(format!("[apparc] StartDocument {doc_name:?} -> KErrNotFound (no CoralOS handler)"));
                p.cpu.regs[0] = KERR_NOT_FOUND as u32;
            }
            Err(code) => {
                p.cpu.regs[0] = code as u32;
            }
        }
        return Some(NativeOutcome::Return);
    }

    None
}
