//! Native CoralOS `coral-descriptor` + `coral-opener` host bridge ([`crate::OsVTable::attach_apparc`],
//! appended to OS ABI v3, REUSE-AUDIT U19).
use core::ffi::c_void;

pub const APPARC_ATTACH_OK: i32 = 0;
pub const APPARC_ATTACH_REFUSED: i32 = 1;

pub const APPARC_OK: i32 = 0;
pub const APPARC_NONE: i32 = 1;
pub const APPARC_NOT_GRANTED: i32 = -1;
pub const APPARC_BAD_NAME: i32 = -2;
pub const APPARC_LOST: i32 = -4;
pub const APPARC_UNREADABLE: i32 = -6;
pub const APPARC_REFUSED_OTHER: i32 = -13;

/// Synchronous UTF-8 string visitor callback (zero cross-DLL heap frees).
pub type StrEmitFn = extern "C" fn(cb_ctx: *mut c_void, ptr: *const u8, len: usize);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AppArcHost {
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Recognize `(name, head)`: emits MIME via `emit_mime`, writes `*out_conf`, returns `APPARC_OK`.
    pub recognize_data: extern "C" fn(
        ctx: *mut c_void,
        name: *const u8,
        name_len: usize,
        head: *const u8,
        head_len: usize,
        emit_mime: StrEmitFn,
        cb_ctx: *mut c_void,
        out_conf: *mut i32,
    ) -> i32,
    /// Resolve default handler bundle stem for `(mime, name)`: emits stem + returns `APPARC_OK`,
    /// or `APPARC_NONE` when no tool claims it.
    pub handler_for: extern "C" fn(
        ctx: *mut c_void,
        mime: *const u8,
        mime_len: usize,
        name: *const u8,
        name_len: usize,
        emit_stem: StrEmitFn,
        cb_ctx: *mut c_void,
    ) -> i32,
}

unsafe impl Send for AppArcHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use alloc::string::String;
    use guestsvc_abi::{GuestAppArc, GuestSvcError, GuestTypeInfo};

    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => APPARC_NOT_GRANTED,
            GuestSvcError::BadName => APPARC_BAD_NAME,
            GuestSvcError::Lost => APPARC_LOST,
            GuestSvcError::Unreadable => APPARC_UNREADABLE,
            _ => APPARC_REFUSED_OTHER,
        }
    }

    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            APPARC_NOT_GRANTED => GuestSvcError::NotGranted,
            APPARC_BAD_NAME => GuestSvcError::BadName,
            APPARC_LOST => GuestSvcError::Lost,
            APPARC_UNREADABLE => GuestSvcError::Unreadable,
            _ => GuestSvcError::Refused("refused"),
        }
    }

    extern "C" fn collect_str(cb_ctx: *mut c_void, ptr: *const u8, len: usize) {
        if cb_ctx.is_null() || (ptr.is_null() && len != 0) {
            return;
        }
        let slot = unsafe { &mut *(cb_ctx as *mut String) };
        let bytes = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(ptr, len) } };
        if let Ok(s) = core::str::from_utf8(bytes) {
            *slot = String::from(s);
        }
    }

    impl GuestAppArc for AppArcHost {
        fn recognize_data(&mut self, name: &str, head: &[u8]) -> Result<GuestTypeInfo, GuestSvcError> {
            let mut mime = String::new();
            let mut conf = guestsvc_abi::CONF_NOT_RECOGNIZED;
            let rc = (self.recognize_data)(
                self.ctx,
                name.as_ptr(),
                name.len(),
                head.as_ptr(),
                head.len(),
                collect_str,
                (&mut mime as *mut String).cast(),
                &mut conf,
            );
            if rc == APPARC_OK { Ok(GuestTypeInfo { mime, confidence: conf }) } else { Err(code_to_err(rc)) }
        }

        fn handler_for(&mut self, mime: &str, name: &str) -> Result<Option<String>, GuestSvcError> {
            let mut stem = String::new();
            let rc = (self.handler_for)(
                self.ctx,
                mime.as_ptr(),
                mime.len(),
                name.as_ptr(),
                name.len(),
                collect_str,
                (&mut stem as *mut String).cast(),
            );
            match rc {
                APPARC_OK => Ok(Some(stem)),
                APPARC_NONE => Ok(None),
                other => Err(code_to_err(other)),
            }
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, err_to_code};
