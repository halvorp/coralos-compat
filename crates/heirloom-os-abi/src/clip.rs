//! Native CoralOS `clipboardd` host bridge ([`crate::OsVTable::attach_clipboard`], appended to OS ABI v3).
//!
//! Lets an engine process (`coral-heirloom-engine-rt::clip_service::ClipService`, backed by CoralOS
//! `clipboardd` + C12 `Watch`/doorbell) attach its clipboard session to a dynamically loaded `os-*.library`
//! without crossing the DLL boundary with Rust trait objects or cross-allocator frees. Inside
//! `os-*.library`, [`ClipHost`] implements `guestsvc_abi::GuestClipboard`.
use core::ffi::c_void;

/// `attach_clipboard` accepted the host.
pub const CLIP_ATTACH_OK: i32 = 0;
/// `attach_clipboard` refused (null args or family does not consume host clipboard).
pub const CLIP_ATTACH_REFUSED: i32 = 1;

/// Call status codes returned by [`ClipHost`] callbacks.
pub const CLIP_OK: i32 = 0;
pub const CLIP_EMPTY: i32 = 1;
pub const CLIP_NOT_GRANTED: i32 = -1;
pub const CLIP_BAD_NAME: i32 = -2;
pub const CLIP_TOO_LARGE: i32 = -3;
pub const CLIP_LOST: i32 = -4;
pub const CLIP_STALLED: i32 = -5;
pub const CLIP_UNREADABLE: i32 = -6;
pub const CLIP_REFUSED_NO_MEMORY: i32 = -11;
pub const CLIP_REFUSED_BAD_REQUEST: i32 = -12;
pub const CLIP_REFUSED_OTHER: i32 = -13;

/// Synchronous paste visitor callback invoked by [`ClipHost::paste`] with the host-lent `(mime, data)`.
pub type PasteEmitFn = extern "C" fn(
    cb_ctx: *mut c_void,
    mime: *const u8,
    mime_len: usize,
    data: *const u8,
    data_len: usize,
);

/// Host-provided `clipboardd` callbacks ([`crate::OsVTable::attach_clipboard`], copied; `ctx` and fn
/// pointers must outlive the `OsInstance`).
///
/// No heap allocation crosses the DLL boundary (`paste` invokes a synchronous visitor callback).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClipHost {
    /// `size_of::<ClipHost>()` as built by the host.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Publish `(mime[..mime_len], data[..data_len])` to the clipboard; writes `*out_gen` on `CLIP_OK`.
    pub copy: extern "C" fn(
        ctx: *mut c_void,
        mime: *const u8,
        mime_len: usize,
        data: *const u8,
        data_len: usize,
        out_gen: *mut u64,
    ) -> i32,
    /// Read the current clipboard entry; invokes `emit` synchronously once and returns `CLIP_OK`,
    /// returns `CLIP_EMPTY` if empty, or returns a negative `CLIP_*` error code.
    pub paste: extern "C" fn(ctx: *mut c_void, emit: PasteEmitFn, cb_ctx: *mut c_void) -> i32,
    /// Return the current clipboard generation in `*out_gen` (`CLIP_OK`), or a negative `CLIP_*` error code.
    pub generation: extern "C" fn(ctx: *mut c_void, out_gen: *mut u64) -> i32,
}

// SAFETY: ClipHost holds plain function pointers and an engine-owned ctx pointer accessed on the single engine thread.
unsafe impl Send for ClipHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;
    use guestsvc_abi::{GuestClipboard, GuestSvcError};

    /// Encode a [`GuestSvcError`] as a C-ABI `CLIP_*` status code.
    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => CLIP_NOT_GRANTED,
            GuestSvcError::BadName => CLIP_BAD_NAME,
            GuestSvcError::TooLarge => CLIP_TOO_LARGE,
            GuestSvcError::Lost => CLIP_LOST,
            GuestSvcError::Stalled => CLIP_STALLED,
            GuestSvcError::Unreadable => CLIP_UNREADABLE,
            GuestSvcError::Refused("no-memory") => CLIP_REFUSED_NO_MEMORY,
            GuestSvcError::Refused("bad-request") => CLIP_REFUSED_BAD_REQUEST,
            GuestSvcError::Refused(_) => CLIP_REFUSED_OTHER,
        }
    }

    /// Decode a C-ABI `CLIP_*` error code back to a [`GuestSvcError`].
    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            CLIP_NOT_GRANTED => GuestSvcError::NotGranted,
            CLIP_BAD_NAME => GuestSvcError::BadName,
            CLIP_TOO_LARGE => GuestSvcError::TooLarge,
            CLIP_LOST => GuestSvcError::Lost,
            CLIP_STALLED => GuestSvcError::Stalled,
            CLIP_UNREADABLE => GuestSvcError::Unreadable,
            CLIP_REFUSED_NO_MEMORY => GuestSvcError::Refused("no-memory"),
            CLIP_REFUSED_BAD_REQUEST => GuestSvcError::Refused("bad-request"),
            _ => GuestSvcError::Refused("refused"),
        }
    }

    extern "C" fn collect_paste(
        cb_ctx: *mut c_void,
        mime: *const u8,
        mime_len: usize,
        data: *const u8,
        data_len: usize,
    ) {
        if cb_ctx.is_null() || (mime.is_null() && mime_len != 0) || (data.is_null() && data_len != 0) {
            return;
        }
        // SAFETY: `cb_ctx` points to the caller's `Option<(String, Vec<u8>)>` for the duration of `paste`.
        let slot = unsafe { &mut *(cb_ctx as *mut Option<(String, Vec<u8>)>) };
        // SAFETY: host lends `mime_len` and `data_len` readable bytes for the duration of this synchronous callback.
        let mime_bytes = if mime_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(mime, mime_len) } };
        let data_bytes = if data_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(data, data_len) } };
        if let Ok(m) = core::str::from_utf8(mime_bytes) {
            *slot = Some((String::from(m), data_bytes.to_vec()));
        }
    }

    impl GuestClipboard for ClipHost {
        fn copy(&mut self, mime: &str, payload: &[u8]) -> Result<u64, GuestSvcError> {
            let mut gen = 0u64;
            let rc = (self.copy)(
                self.ctx,
                mime.as_ptr(),
                mime.len(),
                payload.as_ptr(),
                payload.len(),
                &mut gen,
            );
            if rc == CLIP_OK { Ok(gen) } else { Err(code_to_err(rc)) }
        }

        fn paste(&mut self) -> Result<Option<(String, Vec<u8>)>, GuestSvcError> {
            let mut slot: Option<(String, Vec<u8>)> = None;
            let rc = (self.paste)(
                self.ctx,
                collect_paste,
                (&mut slot as *mut Option<(String, Vec<u8>)>).cast(),
            );
            match rc {
                CLIP_OK => Ok(slot),
                CLIP_EMPTY => Ok(None),
                other => Err(code_to_err(other)),
            }
        }

        fn generation(&mut self) -> Result<u64, GuestSvcError> {
            let mut gen = 0u64;
            let rc = (self.generation)(self.ctx, &mut gen);
            if rc == CLIP_OK { Ok(gen) } else { Err(code_to_err(rc)) }
        }
    }

    /// Build a [`ClipHost`] whose callbacks dispatch to `*mut T` (any `GuestClipboard` implementor).
    ///
    /// # Safety
    /// `ctx` must remain valid, unmoved, and exclusively borrowed by the engine thread for the entire
    /// lifetime of the `OsInstance` that receives this `ClipHost`.
    pub unsafe fn host_for<T: GuestClipboard>(ctx: *mut T) -> ClipHost {
        extern "C" fn tramp_copy<T: GuestClipboard>(
            ctx: *mut c_void,
            mime: *const u8,
            mime_len: usize,
            data: *const u8,
            data_len: usize,
            out_gen: *mut u64,
        ) -> i32 {
            if ctx.is_null()
                || out_gen.is_null()
                || (mime.is_null() && mime_len != 0)
                || (data.is_null() && data_len != 0)
            {
                return CLIP_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let clip = unsafe { &mut *(ctx as *mut T) };
            let mime_bytes = if mime_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(mime, mime_len) } };
            let Ok(mime_str) = core::str::from_utf8(mime_bytes) else { return CLIP_BAD_NAME };
            let data_bytes = if data_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(data, data_len) } };
            match clip.copy(mime_str, data_bytes) {
                Ok(gen) => {
                    // SAFETY: `out_gen` validated non-null above.
                    unsafe { *out_gen = gen };
                    CLIP_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_paste<T: GuestClipboard>(
            ctx: *mut c_void,
            emit: PasteEmitFn,
            cb_ctx: *mut c_void,
        ) -> i32 {
            if ctx.is_null() {
                return CLIP_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let clip = unsafe { &mut *(ctx as *mut T) };
            match clip.paste() {
                Ok(None) => CLIP_EMPTY,
                Ok(Some((mime, data))) => {
                    emit(cb_ctx, mime.as_ptr(), mime.len(), data.as_ptr(), data.len());
                    CLIP_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_generation<T: GuestClipboard>(ctx: *mut c_void, out_gen: *mut u64) -> i32 {
            if ctx.is_null() || out_gen.is_null() {
                return CLIP_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let clip = unsafe { &mut *(ctx as *mut T) };
            match clip.generation() {
                Ok(gen) => {
                    // SAFETY: `out_gen` validated non-null above.
                    unsafe { *out_gen = gen };
                    CLIP_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        ClipHost {
            size: core::mem::size_of::<ClipHost>() as u32,
            flags: 0,
            ctx: ctx.cast(),
            copy: tramp_copy::<T>,
            paste: tramp_paste::<T>,
            generation: tramp_generation::<T>,
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, err_to_code, host_for};

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use guestsvc_abi::{FakeClipboard, GuestClipboard, GuestSvcError};

    #[test]
    fn clip_host_round_trips_copy_paste_generation_and_not_granted() {
        let mut fake = FakeClipboard::new();
        // SAFETY: `fake` outlives `host` in this test.
        let mut host = unsafe { host_for(&mut fake) };

        assert_eq!(host.generation(), Ok(0));
        assert_eq!(host.paste(), Ok(None));

        let g1 = host.copy("text/plain;charset=utf-8", b"mac->coral").unwrap();
        assert_eq!(g1, 1);
        assert_eq!(host.generation(), Ok(1));
        assert_eq!(
            host.paste(),
            Ok(Some((String::from("text/plain;charset=utf-8"), b"mac->coral".to_vec())))
        );

        let g2 = host.copy("application/x-mac-pict", &[0x00, 0x11, 0x02, 0xFF]).unwrap();
        assert_eq!(g2, 2);
        assert_eq!(
            host.paste(),
            Ok(Some((String::from("application/x-mac-pict"), alloc::vec![0x00, 0x11, 0x02, 0xFF])))
        );

        // Negative control: NotGranted crosses the C ABI intact.
        let mut denied = FakeClipboard::not_granted();
        let mut denied_host = unsafe { host_for(&mut denied) };
        assert_eq!(denied_host.copy("text/plain", b"x"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied_host.paste(), Err(GuestSvcError::NotGranted));
        assert_eq!(denied_host.generation(), Err(GuestSvcError::NotGranted));
    }
}
