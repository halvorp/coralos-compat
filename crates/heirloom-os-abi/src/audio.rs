//! Native CoralOS `Aura.engine` host bridge ([`crate::OsVTable::attach_audio`], appended to OS ABI v3).
//!
//! Lets an engine process (`coral-heirloom-engine-rt::audio_service::AudioService`, backed by CoralOS
//! `Aura.engine` over `EP_AURA_LISTEN`) attach its audio playback session to a dynamically loaded
//! `os-*.library` without crossing the DLL boundary with Rust trait objects or cross-allocator frees.
//! Inside `os-*.library`, [`AudioHost`] implements `guestsvc_abi::GuestAudio`.
use core::ffi::c_void;

/// `attach_audio` accepted the host.
pub const AUDIO_ATTACH_OK: i32 = 0;
/// `attach_audio` refused (null args or family does not consume host audio).
pub const AUDIO_ATTACH_REFUSED: i32 = 1;

/// Call status codes returned by [`AudioHost`] callbacks.
pub const AUDIO_OK: i32 = 0;
pub const AUDIO_NOT_GRANTED: i32 = -1;
pub const AUDIO_BAD_NAME: i32 = -2;
pub const AUDIO_TOO_LARGE: i32 = -3;
pub const AUDIO_LOST: i32 = -4;
pub const AUDIO_STALLED: i32 = -5;
pub const AUDIO_UNREADABLE: i32 = -6;
pub const AUDIO_REFUSED_BAD_STREAM: i32 = -10;
pub const AUDIO_REFUSED_BAD_FORMAT: i32 = -11;
pub const AUDIO_REFUSED_NO_SINK: i32 = -12;
pub const AUDIO_REFUSED_OTHER: i32 = -13;

/// Host-provided `Aura.engine` callbacks ([`crate::OsVTable::attach_audio`], copied; `ctx` and fn
/// pointers must outlive the `OsInstance`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioHost {
    /// `size_of::<AudioHost>()` as built by the host.
    pub size: u32,
    pub flags: u32,
    pub ctx: *mut c_void,
    /// Open a playback stream; writes non-zero `*out_id` on `AUDIO_OK`.
    pub open_stream: extern "C" fn(
        ctx: *mut c_void,
        rate_hz: u32,
        channels: u8,
        format: u8,
        label: *const u8,
        label_len: usize,
        out_id: *mut u32,
    ) -> i32,
    /// Append uncompressed guest PCM bytes to `stream`.
    pub queue_pcm: extern "C" fn(
        ctx: *mut c_void,
        stream: u32,
        pcm: *const u8,
        pcm_len: usize,
    ) -> i32,
    /// Append looping guest PCM bytes to `stream` (`loop_start..loop_end` in frames).
    pub queue_looped: extern "C" fn(
        ctx: *mut c_void,
        stream: u32,
        pcm: *const u8,
        pcm_len: usize,
        loop_start: u32,
        loop_end: u32,
    ) -> i32,
    /// Drop all queued/looping samples on `stream`.
    pub flush: extern "C" fn(ctx: *mut c_void, stream: u32) -> i32,
    /// Pause (`paused != 0`) or resume (`paused == 0`) `stream`.
    pub pause: extern "C" fn(ctx: *mut c_void, stream: u32, paused: u8) -> i32,
    /// Set per-side gain (`0..=AUDIO_UNITY`, 8.8 fixed-point) on `stream`.
    pub set_volume: extern "C" fn(ctx: *mut c_void, stream: u32, left: u16, right: u16) -> i32,
    /// Write unplayed guest PCM bytes remaining on `stream` into `*out_bytes`.
    pub queued_bytes: extern "C" fn(ctx: *mut c_void, stream: u32, out_bytes: *mut u32) -> i32,
    /// Close `stream`.
    pub close_stream: extern "C" fn(ctx: *mut c_void, stream: u32) -> i32,
}

// SAFETY: AudioHost holds plain function pointers and an engine-owned ctx pointer accessed on the single engine thread.
unsafe impl Send for AudioHost {}

#[cfg(any(feature = "serve", test))]
mod bridge {
    use super::*;
    use alloc::string::String;
    use guestsvc_abi::{GuestAudio, GuestAudioFormat, GuestAudioModel, GuestSvcError};

    /// Encode a [`GuestSvcError`] as a C-ABI `AUDIO_*` status code.
    pub fn err_to_code(e: GuestSvcError) -> i32 {
        match e {
            GuestSvcError::NotGranted => AUDIO_NOT_GRANTED,
            GuestSvcError::BadName => AUDIO_BAD_NAME,
            GuestSvcError::TooLarge => AUDIO_TOO_LARGE,
            GuestSvcError::Lost => AUDIO_LOST,
            GuestSvcError::Stalled => AUDIO_STALLED,
            GuestSvcError::Unreadable => AUDIO_UNREADABLE,
            GuestSvcError::Refused("bad-stream") => AUDIO_REFUSED_BAD_STREAM,
            GuestSvcError::Refused("bad-format") => AUDIO_REFUSED_BAD_FORMAT,
            GuestSvcError::Refused("no-sink") => AUDIO_REFUSED_NO_SINK,
            GuestSvcError::Refused(_) => AUDIO_REFUSED_OTHER,
        }
    }

    /// Decode a C-ABI `AUDIO_*` error code back to a [`GuestSvcError`].
    pub fn code_to_err(rc: i32) -> GuestSvcError {
        match rc {
            AUDIO_NOT_GRANTED => GuestSvcError::NotGranted,
            AUDIO_BAD_NAME => GuestSvcError::BadName,
            AUDIO_TOO_LARGE => GuestSvcError::TooLarge,
            AUDIO_LOST => GuestSvcError::Lost,
            AUDIO_STALLED => GuestSvcError::Stalled,
            AUDIO_UNREADABLE => GuestSvcError::Unreadable,
            AUDIO_REFUSED_BAD_STREAM => GuestSvcError::Refused("bad-stream"),
            AUDIO_REFUSED_BAD_FORMAT => GuestSvcError::Refused("bad-format"),
            AUDIO_REFUSED_NO_SINK => GuestSvcError::Refused("no-sink"),
            _ => GuestSvcError::Refused("refused"),
        }
    }

    impl GuestAudio for AudioHost {
        fn open_stream(&mut self, model: &GuestAudioModel) -> Result<u32, GuestSvcError> {
            let mut id = 0u32;
            let rc = (self.open_stream)(
                self.ctx,
                model.rate_hz,
                model.channels,
                model.format as u8,
                model.label.as_ptr(),
                model.label.len(),
                &mut id,
            );
            if rc == AUDIO_OK { Ok(id) } else { Err(code_to_err(rc)) }
        }

        fn queue_pcm(&mut self, stream: u32, pcm: &[u8]) -> Result<(), GuestSvcError> {
            let rc = (self.queue_pcm)(self.ctx, stream, pcm.as_ptr(), pcm.len());
            if rc == AUDIO_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn queue_looped(
            &mut self,
            stream: u32,
            pcm: &[u8],
            loop_start: u32,
            loop_end: u32,
        ) -> Result<(), GuestSvcError> {
            let rc = (self.queue_looped)(self.ctx, stream, pcm.as_ptr(), pcm.len(), loop_start, loop_end);
            if rc == AUDIO_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn flush(&mut self, stream: u32) -> Result<(), GuestSvcError> {
            let rc = (self.flush)(self.ctx, stream);
            if rc == AUDIO_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn pause(&mut self, stream: u32, paused: bool) -> Result<(), GuestSvcError> {
            let rc = (self.pause)(self.ctx, stream, u8::from(paused));
            if rc == AUDIO_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn set_volume(&mut self, stream: u32, left: u16, right: u16) -> Result<(), GuestSvcError> {
            let rc = (self.set_volume)(self.ctx, stream, left, right);
            if rc == AUDIO_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }

        fn queued_bytes(&mut self, stream: u32) -> Result<u32, GuestSvcError> {
            let mut bytes = 0u32;
            let rc = (self.queued_bytes)(self.ctx, stream, &mut bytes);
            if rc == AUDIO_OK { Ok(bytes) } else { Err(code_to_err(rc)) }
        }

        fn close_stream(&mut self, stream: u32) -> Result<(), GuestSvcError> {
            let rc = (self.close_stream)(self.ctx, stream);
            if rc == AUDIO_OK { Ok(()) } else { Err(code_to_err(rc)) }
        }
    }

    /// Build an [`AudioHost`] whose callbacks dispatch to `*mut T` (any `GuestAudio` implementor).
    ///
    /// # Safety
    /// `ctx` must remain valid, unmoved, and exclusively borrowed by the engine thread for the entire
    /// lifetime of the `OsInstance` that receives this `AudioHost`.
    pub unsafe fn host_for<T: GuestAudio>(ctx: *mut T) -> AudioHost {
        extern "C" fn tramp_open<T: GuestAudio>(
            ctx: *mut c_void,
            rate_hz: u32,
            channels: u8,
            format: u8,
            label: *const u8,
            label_len: usize,
            out_id: *mut u32,
        ) -> i32 {
            if ctx.is_null() || out_id.is_null() || (label.is_null() && label_len != 0) {
                return AUDIO_UNREADABLE;
            }
            let Some(fmt) = GuestAudioFormat::from_u8(format) else { return AUDIO_REFUSED_BAD_FORMAT };
            // SAFETY: caller lends `label_len` readable bytes at `label`.
            let label_bytes = if label_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(label, label_len) } };
            let Ok(label_str) = core::str::from_utf8(label_bytes) else { return AUDIO_BAD_NAME };
            // SAFETY: caller guarantees `ctx` points to live `T` on the single engine thread.
            let aud = unsafe { &mut *(ctx as *mut T) };
            let model = GuestAudioModel { rate_hz, channels, format: fmt, label: String::from(label_str) };
            match aud.open_stream(&model) {
                Ok(id) => {
                    // SAFETY: `out_id` validated non-null above.
                    unsafe { *out_id = id };
                    AUDIO_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_queue<T: GuestAudio>(
            ctx: *mut c_void,
            stream: u32,
            pcm: *const u8,
            pcm_len: usize,
        ) -> i32 {
            if ctx.is_null() || (pcm.is_null() && pcm_len != 0) {
                return AUDIO_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` and lends `pcm_len` readable bytes.
            let aud = unsafe { &mut *(ctx as *mut T) };
            let bytes = if pcm_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(pcm, pcm_len) } };
            match aud.queue_pcm(stream, bytes) {
                Ok(()) => AUDIO_OK,
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_queue_looped<T: GuestAudio>(
            ctx: *mut c_void,
            stream: u32,
            pcm: *const u8,
            pcm_len: usize,
            loop_start: u32,
            loop_end: u32,
        ) -> i32 {
            if ctx.is_null() || (pcm.is_null() && pcm_len != 0) {
                return AUDIO_UNREADABLE;
            }
            // SAFETY: caller guarantees `ctx` points to live `T` and lends `pcm_len` readable bytes.
            let aud = unsafe { &mut *(ctx as *mut T) };
            let bytes = if pcm_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(pcm, pcm_len) } };
            match aud.queue_looped(stream, bytes, loop_start, loop_end) {
                Ok(()) => AUDIO_OK,
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_flush<T: GuestAudio>(ctx: *mut c_void, stream: u32) -> i32 {
            if ctx.is_null() { return AUDIO_UNREADABLE; }
            let aud = unsafe { &mut *(ctx as *mut T) };
            match aud.flush(stream) { Ok(()) => AUDIO_OK, Err(e) => err_to_code(e) }
        }

        extern "C" fn tramp_pause<T: GuestAudio>(ctx: *mut c_void, stream: u32, paused: u8) -> i32 {
            if ctx.is_null() { return AUDIO_UNREADABLE; }
            let aud = unsafe { &mut *(ctx as *mut T) };
            match aud.pause(stream, paused != 0) { Ok(()) => AUDIO_OK, Err(e) => err_to_code(e) }
        }

        extern "C" fn tramp_volume<T: GuestAudio>(ctx: *mut c_void, stream: u32, left: u16, right: u16) -> i32 {
            if ctx.is_null() { return AUDIO_UNREADABLE; }
            let aud = unsafe { &mut *(ctx as *mut T) };
            match aud.set_volume(stream, left, right) { Ok(()) => AUDIO_OK, Err(e) => err_to_code(e) }
        }

        extern "C" fn tramp_queued<T: GuestAudio>(ctx: *mut c_void, stream: u32, out_bytes: *mut u32) -> i32 {
            if ctx.is_null() || out_bytes.is_null() { return AUDIO_UNREADABLE; }
            let aud = unsafe { &mut *(ctx as *mut T) };
            match aud.queued_bytes(stream) {
                Ok(n) => {
                    unsafe { *out_bytes = n };
                    AUDIO_OK
                }
                Err(e) => err_to_code(e),
            }
        }

        extern "C" fn tramp_close<T: GuestAudio>(ctx: *mut c_void, stream: u32) -> i32 {
            if ctx.is_null() { return AUDIO_UNREADABLE; }
            let aud = unsafe { &mut *(ctx as *mut T) };
            match aud.close_stream(stream) { Ok(()) => AUDIO_OK, Err(e) => err_to_code(e) }
        }

        AudioHost {
            size: core::mem::size_of::<AudioHost>() as u32,
            flags: 0,
            ctx: ctx.cast(),
            open_stream: tramp_open::<T>,
            queue_pcm: tramp_queue::<T>,
            queue_looped: tramp_queue_looped::<T>,
            flush: tramp_flush::<T>,
            pause: tramp_pause::<T>,
            set_volume: tramp_volume::<T>,
            queued_bytes: tramp_queued::<T>,
            close_stream: tramp_close::<T>,
        }
    }
}

#[cfg(any(feature = "serve", test))]
pub use bridge::{code_to_err, err_to_code, host_for};

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use guestsvc_abi::{FakeAudio, GuestAudio, GuestAudioFormat, GuestAudioModel, GuestSvcError};

    #[test]
    fn audio_host_round_trips_playback_and_not_granted() {
        let mut fake = FakeAudio::new();
        // SAFETY: `fake` outlives `host` in this test.
        let mut host = unsafe { host_for(&mut fake) };
        let model = GuestAudioModel {
            rate_hz: 22_254,
            channels: 1,
            format: GuestAudioFormat::U8,
            label: String::from("Sound Manager"),
        };
        let id = host.open_stream(&model).unwrap();
        host.queue_pcm(id, &[0x80, 0xA0, 0xC0, 0xA0]).unwrap();
        assert_eq!(host.queued_bytes(id), Ok(4));
        host.set_volume(id, 0x0080, 0x00C0).unwrap();
        host.pause(id, true).unwrap();
        host.flush(id).unwrap();
        assert_eq!(host.queued_bytes(id), Ok(0));
        host.queue_looped(id, &[0x80, 0xA0, 0xC0, 0xA0], 1, 3).unwrap();
        assert_eq!(host.queued_bytes(id), Ok(4));
        host.close_stream(id).unwrap();

        // Negative control: NotGranted crosses the C ABI intact.
        let mut denied = FakeAudio::not_granted();
        let mut denied_host = unsafe { host_for(&mut denied) };
        assert_eq!(denied_host.open_stream(&model), Err(GuestSvcError::NotGranted));
        assert_eq!(denied_host.queued_bytes(1), Err(GuestSvcError::NotGranted));
    }
}
