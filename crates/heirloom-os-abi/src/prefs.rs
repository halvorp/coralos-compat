//! Native CoralOS `prefs.service` host bridge ([`crate::OsVTable::attach_prefs`], appended to OS ABI v3).
//!
//! Lets an engine process (`coral-heirloom-engine-rt::services::Services`) attach its own `prefs.service`
//! session (`Space::Own`, admitted by the loader for the launched bundle) to a dynamically loaded
//! `os-*.library` without crossing the DLL boundary with Rust trait objects or cross-allocator frees.
//! Inside `os-*.library`, [`PrefsHost`] implements `guestsvc_abi::GuestPrefs` so guest adapters remain
//! pure sans-IO consumers of the host trait.
use core::ffi::c_void;

/// `attach_prefs` accepted the host.
pub const PREFS_ATTACH_OK: i32 = 0;
/// `attach_prefs` refused (null args or family does not consume host prefs).
pub const PREFS_ATTACH_REFUSED: i32 = 1;

/// Call status codes returned by [`PrefsHost`] callbacks.
pub const PREFS_OK: i32 = 0;
pub const PREFS_NONE: i32 = 1;
pub const PREFS_NOT_GRANTED: i32 = -1;
pub const PREFS_BAD_NAME: i32 = -2;
pub const PREFS_TOO_LARGE: i32 = -3;
pub const PREFS_LOST: i32 = -4;
pub const PREFS_STALLED: i32 = -5;
pub const PREFS_UNREADABLE: i32 = -6;
pub const PREFS_REFUSED_BAD_VALUE: i32 = -7;
pub const PREFS_REFUSED_STALE_INCARNATION: i32 = -8;
pub const PREFS_REFUSED_STORE_FAILED: i32 = -9;
pub const PREFS_REFUSED_CONFLICT: i32 = -10;
pub const PREFS_REFUSED_NO_MEMORY: i32 = -11;
pub const PREFS_REFUSED_BAD_REQUEST: i32 = -12;
pub const PREFS_REFUSED_OTHER: i32 = -13;

/// Value discriminant tags across the C ABI.
pub const VAL_BOOL: u8 = 1;
pub const VAL_INT: u8 = 2;
pub const VAL_REAL: u8 = 3;
pub const VAL_TEXT: u8 = 4;
pub const VAL_BYTES: u8 = 5;

/// Maximum payload length for `VAL_TEXT` / `VAL_BYTES` (matches `guestsvc_abi::VALUE_MAX`).
pub const PREFS_VALUE_MAX: usize = 1024;

/// Host-provided `prefs.service` callbacks ([`crate::OsVTable::attach_prefs`], copied; `ctx` and fn
/// pointers must outlive the `OsInstance`).
///
/// Every call operates in the engine's own loader-admitted namespace (`Space::Own`); the guest chooses
/// keys only, never a namespace. No heap allocation crosses the DLL boundary (`get` writes into caller's
/// buffer, `keys` invokes a synchronous visitor callback).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PrefsHost {
    /// `size_of::<PrefsHost>()` as built by the host.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Fetch `key[..key_len]`. Returns `PREFS_OK` (writing `*out_tag`, `out_buf[..*out_len]`),
    /// `PREFS_NONE` when absent, or a negative `PREFS_*` error code.
    pub get: extern "C" fn(
        ctx: *mut c_void,
        key: *const u8,
        key_len: usize,
        out_tag: *mut u8,
        out_buf: *mut u8,
        out_cap: usize,
        out_len: *mut usize,
    ) -> i32,
    /// Store `(tag, val[..val_len])` at `key[..key_len]`. Returns `PREFS_OK` or a negative `PREFS_*` error code.
    pub set: extern "C" fn(
        ctx: *mut c_void,
        key: *const u8,
        key_len: usize,
        tag: u8,
        val: *const u8,
        val_len: usize,
    ) -> i32,
    /// Delete `key[..key_len]`. Returns `PREFS_OK` (even when absent) or a negative `PREFS_*` error code.
    pub delete: extern "C" fn(ctx: *mut c_void, key: *const u8, key_len: usize) -> i32,
    /// Enumerate every key under `prefix[..prefix_len]` in ascending order, calling `emit_key` synchronously
    /// for each key. Returns `PREFS_OK` or a negative `PREFS_*` error code.
    pub keys: extern "C" fn(
        ctx: *mut c_void,
        prefix: *const u8,
        prefix_len: usize,
        emit_key: extern "C" fn(cb_ctx: *mut c_void, key: *const u8, key_len: usize),
        cb_ctx: *mut c_void,
    ) -> i32,
}

// SAFETY: PrefsHost holds plain function pointers and an engine-owned ctx pointer accessed on the single engine thread.
unsafe impl Send for PrefsHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;
    use guestsvc_abi::{GuestPrefs, GuestSvcError, GuestValue};

    /// Encode a [`GuestSvcError`] as a C-ABI `PREFS_*` status code.
    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => PREFS_NOT_GRANTED,
            GuestSvcError::BadName => PREFS_BAD_NAME,
            GuestSvcError::TooLarge => PREFS_TOO_LARGE,
            GuestSvcError::Lost => PREFS_LOST,
            GuestSvcError::Stalled => PREFS_STALLED,
            GuestSvcError::Unreadable => PREFS_UNREADABLE,
            GuestSvcError::Refused("bad-value") => PREFS_REFUSED_BAD_VALUE,
            GuestSvcError::Refused("stale-incarnation") => PREFS_REFUSED_STALE_INCARNATION,
            GuestSvcError::Refused("store-failed") => PREFS_REFUSED_STORE_FAILED,
            GuestSvcError::Refused("conflict") => PREFS_REFUSED_CONFLICT,
            GuestSvcError::Refused("no-memory") => PREFS_REFUSED_NO_MEMORY,
            GuestSvcError::Refused("bad-request") => PREFS_REFUSED_BAD_REQUEST,
            GuestSvcError::Refused(_) => PREFS_REFUSED_OTHER,
        }
    }

    /// Decode a C-ABI `PREFS_*` error code back to a [`GuestSvcError`].
    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            PREFS_NOT_GRANTED => GuestSvcError::NotGranted,
            PREFS_BAD_NAME => GuestSvcError::BadName,
            PREFS_TOO_LARGE => GuestSvcError::TooLarge,
            PREFS_LOST => GuestSvcError::Lost,
            PREFS_STALLED => GuestSvcError::Stalled,
            PREFS_UNREADABLE => GuestSvcError::Unreadable,
            PREFS_REFUSED_BAD_VALUE => GuestSvcError::Refused("bad-value"),
            PREFS_REFUSED_STALE_INCARNATION => GuestSvcError::Refused("stale-incarnation"),
            PREFS_REFUSED_STORE_FAILED => GuestSvcError::Refused("store-failed"),
            PREFS_REFUSED_CONFLICT => GuestSvcError::Refused("conflict"),
            PREFS_REFUSED_NO_MEMORY => GuestSvcError::Refused("no-memory"),
            PREFS_REFUSED_BAD_REQUEST => GuestSvcError::Refused("bad-request"),
            _ => GuestSvcError::Refused("refused"),
        }
    }

    /// Decode a `(tag, bytes)` pair into a [`GuestValue`].
    pub fn decode_value(tag: u8, data: &[u8]) -> Result<GuestValue, GuestSvcError> {
        match tag {
            VAL_BOOL if data.len() == 1 => Ok(GuestValue::Bool(data[0] != 0)),
            VAL_INT if data.len() == 8 => Ok(GuestValue::Int(i64::from_le_bytes(data.try_into().unwrap()))),
            VAL_REAL if data.len() == 8 => Ok(GuestValue::Real(f64::from_le_bytes(data.try_into().unwrap()))),
            VAL_TEXT => core::str::from_utf8(data)
                .map(|s| GuestValue::Text(String::from(s)))
                .map_err(|_| GuestSvcError::Unreadable),
            VAL_BYTES => Ok(GuestValue::Bytes(data.to_vec())),
            _ => Err(GuestSvcError::Unreadable),
        }
    }

    /// Encode `value` into `(tag, buf[..len])`.
    pub fn encode_value(value: &GuestValue, buf: &mut [u8; PREFS_VALUE_MAX]) -> Result<(u8, usize), GuestSvcError> {
        match value {
            GuestValue::Bool(b) => {
                buf[0] = u8::from(*b);
                Ok((VAL_BOOL, 1))
            }
            GuestValue::Int(n) => {
                buf[..8].copy_from_slice(&n.to_le_bytes());
                Ok((VAL_INT, 8))
            }
            GuestValue::Real(r) => {
                buf[..8].copy_from_slice(&r.to_le_bytes());
                Ok((VAL_REAL, 8))
            }
            GuestValue::Text(s) => {
                if s.len() > PREFS_VALUE_MAX {
                    return Err(GuestSvcError::TooLarge);
                }
                buf[..s.len()].copy_from_slice(s.as_bytes());
                Ok((VAL_TEXT, s.len()))
            }
            GuestValue::Bytes(b) => {
                if b.len() > PREFS_VALUE_MAX {
                    return Err(GuestSvcError::TooLarge);
                }
                buf[..b.len()].copy_from_slice(b);
                Ok((VAL_BYTES, b.len()))
            }
        }
    }

    extern "C" fn collect_key(cb_ctx: *mut c_void, key: *const u8, key_len: usize) {
        if cb_ctx.is_null() || (key.is_null() && key_len != 0) {
            return;
        }
        // SAFETY: `cb_ctx` points to the caller's `Vec<String>` for the duration of `keys`.
        let out = unsafe { &mut *(cb_ctx as *mut Vec<String>) };
        // SAFETY: host lends `key_len` readable UTF-8 bytes for the duration of the callback.
        let bytes = if key_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(key, key_len) } };
        if let Ok(s) = core::str::from_utf8(bytes) {
            out.push(String::from(s));
        }
    }

    impl GuestPrefs for PrefsHost {
        fn get(&mut self, key: &str) -> Result<Option<GuestValue>, GuestSvcError> {
            let mut tag = 0u8;
            let mut buf = [0u8; PREFS_VALUE_MAX];
            let mut len = 0usize;
            let rc = (self.get)(self.ctx, key.as_ptr(), key.len(), &mut tag, buf.as_mut_ptr(), buf.len(), &mut len);
            match rc {
                PREFS_OK => {
                    if len > buf.len() {
                        return Err(GuestSvcError::Unreadable);
                    }
                    decode_value(tag, &buf[..len]).map(Some)
                }
                PREFS_NONE => Ok(None),
                other => Err(code_to_err(other)),
            }
        }

        fn set(&mut self, key: &str, value: &GuestValue) -> Result<(), GuestSvcError> {
            let mut buf = [0u8; PREFS_VALUE_MAX];
            let (tag, len) = encode_value(value, &mut buf)?;
            let rc = (self.set)(self.ctx, key.as_ptr(), key.len(), tag, buf.as_ptr(), len);
            if rc == PREFS_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn delete(&mut self, key: &str) -> Result<(), GuestSvcError> {
            let rc = (self.delete)(self.ctx, key.as_ptr(), key.len());
            if rc == PREFS_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn keys(&mut self, prefix: &str) -> Result<Vec<String>, GuestSvcError> {
            let mut out = Vec::new();
            let rc = (self.keys)(
                self.ctx,
                prefix.as_ptr(),
                prefix.len(),
                collect_key,
                (&mut out as *mut Vec<String>).cast(),
            );
            if rc == PREFS_OK { Ok(out) } else { Err(code_to_err(rc)) }
        }
    }

    /// Build a [`PrefsHost`] whose callbacks dispatch to `*mut T` (any `GuestPrefs` implementor).
    ///
    /// # Safety
    /// `ctx` must remain valid, unmoved, and exclusively borrowed by the engine thread for the entire
    /// lifetime of the `OsInstance` that receives this `PrefsHost`.
    pub unsafe fn host_for<T: GuestPrefs>(ctx: *mut T) -> PrefsHost {
        extern "C" fn tramp_get<T: GuestPrefs>(
            ctx: *mut c_void,
            key: *const u8,
            key_len: usize,
            out_tag: *mut u8,
            out_buf: *mut u8,
            out_cap: usize,
            out_len: *mut usize,
        ) -> i32 {
            if ctx.is_null() || out_tag.is_null() || out_len.is_null() || (key.is_null() && key_len != 0) {
                return PREFS_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let prefs = unsafe { &mut *(ctx as *mut T) };
            let key_bytes = if key_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(key, key_len) } };
            let Ok(key_str) = core::str::from_utf8(key_bytes) else { return PREFS_BAD_NAME };
            match prefs.get(key_str) {
                Ok(None) => PREFS_NONE,
                Ok(Some(val)) => {
                    let mut tmp = [0u8; PREFS_VALUE_MAX];
                    let Ok((tag, len)) = encode_value(&val, &mut tmp) else { return PREFS_TOO_LARGE };
                    if len > out_cap || (len > 0 && out_buf.is_null()) {
                        return PREFS_TOO_LARGE;
                    }
                    // SAFETY: output pointers validated above.
                    unsafe {
                        *out_tag = tag;
                        if len > 0 {
                            core::ptr::copy_nonoverlapping(tmp.as_ptr(), out_buf, len);
                        }
                        *out_len = len;
                    }
                    PREFS_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_set<T: GuestPrefs>(
            ctx: *mut c_void,
            key: *const u8,
            key_len: usize,
            tag: u8,
            val: *const u8,
            val_len: usize,
        ) -> i32 {
            if ctx.is_null() || (key.is_null() && key_len != 0) || (val.is_null() && val_len != 0) {
                return PREFS_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let prefs = unsafe { &mut *(ctx as *mut T) };
            let key_bytes = if key_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(key, key_len) } };
            let Ok(key_str) = core::str::from_utf8(key_bytes) else { return PREFS_BAD_NAME };
            let val_bytes = if val_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(val, val_len) } };
            let value = match decode_value(tag, val_bytes) {
                Ok(v) => v,
                Err(e) => return err_to_code(e),
            };
            match prefs.set(key_str, &value) {
                Ok(()) => PREFS_OK,
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_delete<T: GuestPrefs>(ctx: *mut c_void, key: *const u8, key_len: usize) -> i32 {
            if ctx.is_null() || (key.is_null() && key_len != 0) {
                return PREFS_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let prefs = unsafe { &mut *(ctx as *mut T) };
            let key_bytes = if key_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(key, key_len) } };
            let Ok(key_str) = core::str::from_utf8(key_bytes) else { return PREFS_BAD_NAME };
            match prefs.delete(key_str) {
                Ok(()) => PREFS_OK,
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_keys<T: GuestPrefs>(
            ctx: *mut c_void,
            prefix: *const u8,
            prefix_len: usize,
            emit_key: extern "C" fn(cb_ctx: *mut c_void, key: *const u8, key_len: usize),
            cb_ctx: *mut c_void,
        ) -> i32 {
            if ctx.is_null() || (prefix.is_null() && prefix_len != 0) {
                return PREFS_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let prefs = unsafe { &mut *(ctx as *mut T) };
            let pfx_bytes = if prefix_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(prefix, prefix_len) } };
            let Ok(pfx_str) = core::str::from_utf8(pfx_bytes) else { return PREFS_BAD_NAME };
            match prefs.keys(pfx_str) {
                Ok(list) => {
                    for k in list {
                        emit_key(cb_ctx, k.as_ptr(), k.len());
                    }
                    PREFS_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        PrefsHost {
            size: core::mem::size_of::<PrefsHost>() as u32,
            flags: 0,
            ctx: ctx.cast(),
            get: tramp_get::<T>,
            set: tramp_set::<T>,
            delete: tramp_delete::<T>,
            keys: tramp_keys::<T>,
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, decode_value, encode_value, err_to_code, host_for};

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;
    use guestsvc_abi::{FakePrefs, GuestPrefs, GuestSvcError, GuestValue};

    #[test]
    fn prefs_host_round_trips_all_value_shapes_and_errors_across_c_abi() {
        let mut fake = FakePrefs::new();
        // SAFETY: `fake` outlives `host` in this test.
        let mut host = unsafe { host_for(&mut fake) };

        assert_eq!(host.get("cenrep/101f876d/00000001"), Ok(None));
        assert_eq!(host.set("cenrep/101f876d/00000001", &GuestValue::Bool(true)), Ok(()));
        assert_eq!(host.set("cenrep/101f876d/00000002", &GuestValue::Int(-77)), Ok(()));
        assert_eq!(host.set("cenrep/101f876d/00000003", &GuestValue::Real(2.75)), Ok(()));
        assert_eq!(host.set("cenrep/101f876d/00000004", &GuestValue::Text("symbian".into())), Ok(()));
        assert_eq!(host.set("dict/102828d6/0000abcd", &GuestValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])), Ok(()));

        assert_eq!(host.get("cenrep/101f876d/00000001"), Ok(Some(GuestValue::Bool(true))));
        assert_eq!(host.get("cenrep/101f876d/00000002"), Ok(Some(GuestValue::Int(-77))));
        assert_eq!(host.get("cenrep/101f876d/00000003"), Ok(Some(GuestValue::Real(2.75))));
        assert_eq!(host.get("cenrep/101f876d/00000004"), Ok(Some(GuestValue::Text("symbian".into()))));
        assert_eq!(host.get("dict/102828d6/0000abcd"), Ok(Some(GuestValue::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]))));

        assert_eq!(
            host.keys("cenrep/101f876d/").unwrap(),
            vec![
                String::from("cenrep/101f876d/00000001"),
                String::from("cenrep/101f876d/00000002"),
                String::from("cenrep/101f876d/00000003"),
                String::from("cenrep/101f876d/00000004"),
            ]
        );
        assert_eq!(host.delete("cenrep/101f876d/00000002"), Ok(()));
        assert_eq!(host.get("cenrep/101f876d/00000002"), Ok(None));

        // Negative control: NotGranted and non-finite Real cross the C ABI intact.
        assert_eq!(host.set("k", &GuestValue::Real(f64::NAN)), Err(GuestSvcError::Refused("bad-value")));
        let mut denied = FakePrefs::not_granted();
        let mut denied_host = unsafe { host_for(&mut denied) };
        assert_eq!(denied_host.get("k"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied_host.set("k", &GuestValue::Int(1)), Err(GuestSvcError::NotGranted));
        assert_eq!(denied_host.delete("k"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied_host.keys(""), Err(GuestSvcError::NotGranted));
    }
}
