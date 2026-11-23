//! Native CoralOS `datatype.library` bridge ([`crate::OsVTable::attach_datatype`], appended to OS ABI v3).
//!
//! Lets any guest OS family (Amiga `datatypes.library` / MUI picture specs, Mac QuickTime `GraphicsImporter`
//! and compressed `PICT`, Symbian `ImageConversion` / `AknIconUtils`) delegate format identification and
//! image decoding straight to CoralOS's native amd64 `.datatype` classes (`datatype.library`, `jpeg.datatype`,
//! `gif.datatype`, `webp.datatype`, `svg.datatype`, `pdf.datatype`) instead of interpreting guest codecs.
#[cfg(any(feature = "serve", test))]
use alloc::vec::Vec;
use core::ffi::c_void;

/// `attach_datatype` accepted the host.
pub const DT_ATTACH_OK: i32 = 0;
/// `attach_datatype` refused (null args or family does not consume native datatypes).
pub const DT_ATTACH_REFUSED: i32 = 1;

/// `decode_image` succeeded; `*out_w`, `*out_h`, `*out_pixels`, `*out_len` are populated (`ARGB32`: `0xAARRGGBB`).
pub const DT_DECODE_OK: i32 = 0;
/// Format not handled by the host datatype set (the caller may fall back to a guest-format decoder such as ILBM).
pub const DT_DECODE_UNSUPPORTED: i32 = 1;
/// Decoding failed (corrupt input, OOM, or host decode worker refused).
pub const DT_DECODE_FAILED: i32 = 2;

/// Content kind identifiers matching CoralOS `coral_datatype_abi::DT_*`.
pub const DT_UNKNOWN: u32 = 0;
pub const DT_PNG: u32 = 1;
pub const DT_WEBP: u32 = 2;
pub const DT_GIF: u32 = 3;
pub const DT_JPEG: u32 = 4;
pub const DT_SVG: u32 = 6;
pub const DT_BMP: u32 = 7;
pub const DT_PDF: u32 = 17;

/// Identify common image/document magic bytes without allocating (matches CoralOS `coral-descriptor` magic rules).
pub fn identify_magic(data: &[u8]) -> u32 {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        return DT_PNG;
    }
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return DT_JPEG;
    }
    if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        return DT_GIF;
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return DT_WEBP;
    }
    if data.len() >= 2 && &data[..2] == b"BM" {
        return DT_BMP;
    }
    if data.len() >= 5 && &data[..5] == b"%PDF-" {
        return DT_PDF;
    }
    let trimmed = skip_ws_and_bom(data);
    if trimmed.starts_with(b"<svg") || (trimmed.starts_with(b"<?xml") && contains_bytes(&trimmed[..trimmed.len().min(512)], b"<svg")) {
        return DT_SVG;
    }
    DT_UNKNOWN
}

fn skip_ws_and_bom(mut s: &[u8]) -> &[u8] {
    if s.starts_with(&[0xEF, 0xBB, 0xBF]) {
        s = &s[3..];
    }
    while let Some(&b) = s.first() {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            s = &s[1..];
        } else {
            break;
        }
    }
    s
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Decoded image pixels in `0xAARRGGBB` (ARGB32) row-major order, `width * height` entries.
#[cfg(any(feature = "serve", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// Host-provided native datatype callbacks ([`crate::OsVTable::attach_datatype`], copied; `ctx` and fn pointers must outlive the instance).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DatatypeHost {
    /// `size_of::<DatatypeHost>()` as built by the host.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Return a `DT_*` kind for `data[..len]` (`DT_UNKNOWN` if unrecognized).
    pub identify: extern "C" fn(ctx: *mut c_void, data: *const u8, len: usize) -> u32,
    /// Decode `data[..len]` into `0xAARRGGBB` pixels, optionally fitting to `(fit_w, fit_h)` (`0, 0` = natural size).
    /// On `DT_DECODE_OK`, writes `*out_w`, `*out_h`, `*out_pixels`, `*out_len`; caller frees via `free_pixels`.
    pub decode_image: extern "C" fn(
        ctx: *mut c_void,
        data: *const u8,
        len: usize,
        fit_w: u32,
        fit_h: u32,
        out_w: *mut u32,
        out_h: *mut u32,
        out_pixels: *mut *mut u32,
        out_len: *mut usize,
    ) -> i32,
    /// Release a pixel buffer returned by `decode_image`.
    pub free_pixels: extern "C" fn(ctx: *mut c_void, pixels: *mut u32, len: usize),
}

// SAFETY: DatatypeHost holds plain function pointers and an engine-owned ctx pointer accessed on the single engine thread.
unsafe impl Send for DatatypeHost {}

impl DatatypeHost {
    /// Identify `data` through the host datatype system (falling back to [`identify_magic`] if host returns `DT_UNKNOWN`).
    pub fn identify_bytes(&self, data: &[u8]) -> u32 {
        let k = (self.identify)(self.ctx, data.as_ptr(), data.len());
        if k != DT_UNKNOWN { k } else { identify_magic(data) }
    }

    /// Decode `data` into ARGB32 pixels through the native host datatype system.
    #[cfg(any(feature = "serve", test))]
    pub fn decode_image_bytes(&self, data: &[u8], fit_w: u32, fit_h: u32) -> Option<DecodedImage> {
        if data.is_empty() {
            return None;
        }
        let mut w = 0u32;
        let mut h = 0u32;
        let mut ptr: *mut u32 = core::ptr::null_mut();
        let mut len = 0usize;
        let rc = (self.decode_image)(self.ctx, data.as_ptr(), data.len(), fit_w, fit_h, &mut w, &mut h, &mut ptr, &mut len);
        if rc != DT_DECODE_OK || ptr.is_null() || w == 0 || h == 0 {
            if !ptr.is_null() {
                (self.free_pixels)(self.ctx, ptr, len);
            }
            return None;
        }
        let expected = (w as usize).checked_mul(h as usize);
        let out = if expected == Some(len) {
            // SAFETY: host contract guarantees `ptr` points to `len` initialized `u32` pixels until `free_pixels` is called.
            let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
            Some(DecodedImage { width: w, height: h, pixels: slice.to_vec() })
        } else {
            None
        };
        (self.free_pixels)(self.ctx, ptr, len);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    extern "C" fn test_identify(_ctx: *mut c_void, data: *const u8, len: usize) -> u32 {
        // SAFETY: test passes valid slice pointer + len.
        let s = unsafe { core::slice::from_raw_parts(data, len) };
        identify_magic(s)
    }

    extern "C" fn test_decode(
        _ctx: *mut c_void,
        data: *const u8,
        len: usize,
        _fw: u32,
        _fh: u32,
        out_w: *mut u32,
        out_h: *mut u32,
        out_px: *mut *mut u32,
        out_len: *mut usize,
    ) -> i32 {
        // SAFETY: test passes valid slice + output pointers.
        let s = unsafe { core::slice::from_raw_parts(data, len) };
        if identify_magic(s) != DT_PNG {
            return DT_DECODE_UNSUPPORTED;
        }
        let mut buf: Box<[u32]> = alloc::vec![0xFF11_2233, 0xFF44_5566, 0xFF77_8899, 0xFFAA_BBCC].into_boxed_slice();
        let p = buf.as_mut_ptr();
        let n = buf.len();
        let _ = Box::into_raw(buf);
        // SAFETY: test passes valid non-null out pointers.
        unsafe {
            *out_w = 2;
            *out_h = 2;
            *out_px = p;
            *out_len = n;
        }
        DT_DECODE_OK
    }

    extern "C" fn test_free(_ctx: *mut c_void, ptr: *mut u32, len: usize) {
        if !ptr.is_null() {
            // SAFETY: matches Box::into_raw from `test_decode` with exact `(ptr, len)`.
            unsafe { drop(Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len))) };
        }
    }

    #[test]
    fn magic_identification_and_host_decode_round_trip() {
        assert_eq!(identify_magic(b"\x89PNG\r\n\x1a\nrest"), DT_PNG);
        assert_eq!(identify_magic(b"\xFF\xD8\xFF\xE0jfif"), DT_JPEG);
        assert_eq!(identify_magic(b"GIF89a..."), DT_GIF);
        assert_eq!(identify_magic(b"RIFF\x00\x00\x00\x00WEBP"), DT_WEBP);
        assert_eq!(identify_magic(b"BM\x36\x00"), DT_BMP);
        assert_eq!(identify_magic(b"%PDF-1.7"), DT_PDF);
        assert_eq!(identify_magic(b"  <svg xmlns=\"x\"/>"), DT_SVG);
        assert_eq!(identify_magic(b"plain text"), DT_UNKNOWN);

        let host = DatatypeHost {
            size: core::mem::size_of::<DatatypeHost>() as u32,
            flags: 0,
            ctx: core::ptr::null_mut(),
            identify: test_identify,
            decode_image: test_decode,
            free_pixels: test_free,
        };
        let img = host.decode_image_bytes(b"\x89PNG\r\n\x1a\n", 0, 0).expect("PNG decoded");
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(img.pixels, alloc::vec![0xFF11_2233, 0xFF44_5566, 0xFF77_8899, 0xFFAA_BBCC]);
        assert!(host.decode_image_bytes(b"unknown", 0, 0).is_none());
    }
}
