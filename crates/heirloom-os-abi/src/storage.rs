//! Native CoralOS per-bundle CoralFS persistence bridge ([`crate::OsVTable::attach_storage`],
//! appended to OS ABI v3, REUSE-AUDIT U19).
use core::ffi::c_void;

pub const STORAGE_ATTACH_OK: i32 = 0;
pub const STORAGE_ATTACH_REFUSED: i32 = 1;

pub const STORAGE_OK: i32 = 0;
pub const STORAGE_NOT_FOUND: i32 = 1;
pub const STORAGE_NOT_GRANTED: i32 = -1;
pub const STORAGE_BAD_NAME: i32 = -2;
pub const STORAGE_TOO_LARGE: i32 = -3;
pub const STORAGE_LOST: i32 = -4;
pub const STORAGE_UNREADABLE: i32 = -6;
pub const STORAGE_REFUSED_OTHER: i32 = -13;

pub type BytesEmitFn = extern "C" fn(cb_ctx: *mut c_void, ptr: *const u8, len: usize);
pub type PathEmitFn = extern "C" fn(cb_ctx: *mut c_void, ptr: *const u8, len: usize);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StorageHost {
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    pub read_file: extern "C" fn(
        ctx: *mut c_void,
        path: *const u8,
        path_len: usize,
        emit: BytesEmitFn,
        cb_ctx: *mut c_void,
    ) -> i32,
    pub write_file: extern "C" fn(
        ctx: *mut c_void,
        path: *const u8,
        path_len: usize,
        data: *const u8,
        data_len: usize,
    ) -> i32,
    pub delete_file: extern "C" fn(ctx: *mut c_void, path: *const u8, path_len: usize) -> i32,
    pub list_files: extern "C" fn(
        ctx: *mut c_void,
        dir: *const u8,
        dir_len: usize,
        emit: PathEmitFn,
        cb_ctx: *mut c_void,
    ) -> i32,
}

unsafe impl Send for StorageHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;
    use guestsvc_abi::{GuestStorage, GuestSvcError};

    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => STORAGE_NOT_GRANTED,
            GuestSvcError::BadName => STORAGE_BAD_NAME,
            GuestSvcError::TooLarge => STORAGE_TOO_LARGE,
            GuestSvcError::Lost => STORAGE_LOST,
            GuestSvcError::Unreadable => STORAGE_UNREADABLE,
            _ => STORAGE_REFUSED_OTHER,
        }
    }

    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            STORAGE_NOT_GRANTED => GuestSvcError::NotGranted,
            STORAGE_BAD_NAME => GuestSvcError::BadName,
            STORAGE_TOO_LARGE => GuestSvcError::TooLarge,
            STORAGE_LOST => GuestSvcError::Lost,
            STORAGE_UNREADABLE => GuestSvcError::Unreadable,
            _ => GuestSvcError::Refused("refused"),
        }
    }

    extern "C" fn collect_bytes(cb_ctx: *mut c_void, ptr: *const u8, len: usize) {
        if cb_ctx.is_null() || (ptr.is_null() && len != 0) { return; }
        let slot = unsafe { &mut *(cb_ctx as *mut Vec<u8>) };
        let bytes = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(ptr, len) } };
        slot.extend_from_slice(bytes);
    }

    extern "C" fn collect_path(cb_ctx: *mut c_void, ptr: *const u8, len: usize) {
        if cb_ctx.is_null() || (ptr.is_null() && len != 0) { return; }
        let list = unsafe { &mut *(cb_ctx as *mut Vec<String>) };
        let bytes = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(ptr, len) } };
        if let Ok(s) = core::str::from_utf8(bytes) {
            list.push(String::from(s));
        }
    }

    impl GuestStorage for StorageHost {
        fn read_file(&mut self, rel_path: &str) -> Result<Option<Vec<u8>>, GuestSvcError> {
            let mut out = Vec::new();
            let rc = (self.read_file)(
                self.ctx,
                rel_path.as_ptr(),
                rel_path.len(),
                collect_bytes,
                (&mut out as *mut Vec<u8>).cast(),
            );
            match rc {
                STORAGE_OK => Ok(Some(out)),
                STORAGE_NOT_FOUND => Ok(None),
                other => Err(code_to_err(other)),
            }
        }

        fn write_file(&mut self, rel_path: &str, data: &[u8]) -> Result<(), GuestSvcError> {
            let rc = (self.write_file)(
                self.ctx,
                rel_path.as_ptr(),
                rel_path.len(),
                data.as_ptr(),
                data.len(),
            );
            if rc == STORAGE_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn delete_file(&mut self, rel_path: &str) -> Result<(), GuestSvcError> {
            let rc = (self.delete_file)(self.ctx, rel_path.as_ptr(), rel_path.len());
            if rc == STORAGE_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn list_files(&mut self, rel_dir: &str) -> Result<Vec<String>, GuestSvcError> {
            let mut list = Vec::new();
            let rc = (self.list_files)(
                self.ctx,
                rel_dir.as_ptr(),
                rel_dir.len(),
                collect_path,
                (&mut list as *mut Vec<String>).cast(),
            );
            if rc == STORAGE_OK { Ok(list) } else { Err(code_to_err(rc)) }
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, err_to_code};
