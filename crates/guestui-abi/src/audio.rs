//! AUDIO OUT: the one way a retro engine makes sound. Generic, like every other part of this seam: no engine's types cross it
//! (no Sound Manager channel, no `audio.device` IORequest), and no guest address does either. The engine decodes whatever the
//! guest handed its OS (a sampled-sound resource, an 8SVX body, a synthesizer command) into interleaved PCM and queues it on a
//! STREAM; the host plays the stream and mixes it with the others. The engine never opens a sound device and never links an
//! audio crate (ruling 17: engines link only this seam and `retro-*`).
//!
//! # What the host must provide (a requirement on the CoralOS side; nothing here pretends to meet it)
//! A CoralOS audio service (a mixer in front of the HDA / USB audio driver) reachable from the process hosting the engine, able to
//! open N independent PCM output streams, queue bytes on each, report how many are still unplayed, pause, flush and set a
//! per-stream gain, and close. Until that service exists a host implements none of it: every method below refuses by name
//! (`UiRefusal::Unsupported`), and the engine answers its guest as its OS does when the hardware is absent
//! (`notEnoughHardware`), never as though the sound played.
//!
//! # Timing
//! The engine is not real-time. `queue_audio` hands over bytes; `audio_queued_bytes` says how many the host has not yet played, so
//! the engine can answer a channel-status or "is it done" question truthfully from the host's own clock. Playback pacing is the
//! host's.
use alloc::string::String;
use crate::model::{UiObjectRef, UiRefusal};

/// A live audio stream (same identity-plus-generation rule as every other seam object).
pub type AudioStreamRef = UiObjectRef;

/// How samples are laid out in the bytes of `queue_audio`. Interleaved by channel in every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Unsigned 8-bit, silence = 0x80 (what a Macintosh sampled sound and an 8SVX-after-conversion are).
    U8,
    /// Signed 16-bit, little-endian, silence = 0.
    S16Le,
}

impl AudioFormat {
    pub const fn bytes_per_sample(self) -> u32 { match self { AudioFormat::U8 => 1, AudioFormat::S16Le => 2 } }
}

/// What to open: one PCM output stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamModel {
    /// Frames per second.
    pub rate_hz: u32,
    /// 1 (mono) or 2 (stereo, left then right).
    pub channels: u8,
    pub format: AudioFormat,
    /// Where it comes from, for a mixer panel ("Sound Manager channel", "audio.device unit 0"). Free text, never parsed.
    pub label: String,
}

impl AudioStreamModel {
    /// Bytes of one frame (all channels).
    pub const fn frame_bytes(&self) -> u32 { self.channels as u32 * self.format.bytes_per_sample() }
    pub fn validate(&self) -> Result<(), UiRefusal> {
        if self.rate_hz == 0 || self.rate_hz > 192_000 { return Err(UiRefusal::InvalidGeometry("audio stream rate outside 1..=192000 Hz")); }
        if !(1..=2).contains(&self.channels) { return Err(UiRefusal::InvalidGeometry("audio stream channels must be 1 or 2")); }
        Ok(())
    }
}

/// Gain of a stream per channel, 8.8 fixed point: `0` silence, [`AUDIO_UNITY`] unchanged. Above unity is refused.
pub const AUDIO_UNITY: u16 = 0x100;
