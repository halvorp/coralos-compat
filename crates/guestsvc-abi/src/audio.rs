//! Host-trait ABI for guest audio playback over CoralOS `Aura.engine` (`EP_AURA_LISTEN`, `(audio)` grant).
//!
//! Guest adapters (Mac 68k/PPC Sound Manager, Amiga `audio.device`, Symbian MMF) remain sans-IO state
//! machines: they open streams, queue raw guest PCM (`U8` unsigned 8-bit or `S16Le` signed 16-bit,
//! mono or stereo), adjust per-stream gain/pause/flush, and query unplayed queue depth through
//! [`GuestAudio`]. On CoralOS, `coral-heirloom-engine-rt::audio_service::AudioService` implements
//! [`GuestAudio`] over `libcoral::audio::AuraLink`, resampling/converting guest PCM into Aura's
//! negotiated `S16Le` stereo ring. Audio NEVER crosses the UI wire (`docs/heirloom/23-ENGINE-SPLIT.md` §2.4).
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::GuestSvcError;

/// Unity gain for per-side volume (8.8 fixed-point `0x0100`, matching Sound Manager & `heirloom-ui-abi`).
pub const AUDIO_UNITY: u16 = 0x0100;

/// Maximum UTF-8 byte length of a stream label.
pub const AUDIO_LABEL_MAX: usize = 128;

/// Maximum PCM byte length accepted in a single queue call (4 MiB).
pub const AUDIO_MAX_PCM: usize = 4 * 1024 * 1024;

/// Guest PCM sample formats crossed over [`GuestAudio`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestAudioFormat {
    /// Unsigned 8-bit PCM (0x80 = silence; classic Mac `'snd '` / Amiga 8-bit).
    U8 = 0,
    /// Signed 16-bit little-endian PCM.
    S16Le = 1,
}

impl GuestAudioFormat {
    pub const fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(GuestAudioFormat::U8),
            1 => Some(GuestAudioFormat::S16Le),
            _ => None,
        }
    }

    pub const fn bytes_per_sample(self) -> usize {
        match self {
            GuestAudioFormat::U8 => 1,
            GuestAudioFormat::S16Le => 2,
        }
    }
}

/// Parameters of one guest audio stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestAudioModel {
    pub rate_hz: u32,
    pub channels: u8,
    pub format: GuestAudioFormat,
    pub label: String,
}

impl GuestAudioModel {
    pub fn ok(&self) -> bool {
        self.rate_hz > 0
            && (1..=2).contains(&self.channels)
            && self.label.len() <= AUDIO_LABEL_MAX
    }

    pub const fn frame_bytes(&self) -> usize {
        (self.channels as usize) * self.format.bytes_per_sample()
    }
}

/// Guest audio playback session owned by the engine process (`Aura.engine` on CoralOS).
pub trait GuestAudio {
    /// Open a playback stream with `model`; returns a non-zero stream id.
    fn open_stream(&mut self, model: &GuestAudioModel) -> Result<u32, GuestSvcError>;
    /// Append uncompressed guest PCM bytes to `stream`.
    fn queue_pcm(&mut self, stream: u32, pcm: &[u8]) -> Result<(), GuestSvcError>;
    /// Append uncompressed guest PCM bytes to `stream` that repeat frames `loop_start..loop_end`
    /// until flushed or closed.
    fn queue_looped(
        &mut self,
        stream: u32,
        pcm: &[u8],
        loop_start: u32,
        loop_end: u32,
    ) -> Result<(), GuestSvcError>;
    /// Drop all queued/looping samples on `stream` immediately.
    fn flush(&mut self, stream: u32) -> Result<(), GuestSvcError>;
    /// Pause (`true`) or resume (`false`) playback on `stream`.
    fn pause(&mut self, stream: u32, paused: bool) -> Result<(), GuestSvcError>;
    /// Set per-side gain in 8.8 fixed-point (`0..=AUDIO_UNITY`).
    fn set_volume(&mut self, stream: u32, left: u16, right: u16) -> Result<(), GuestSvcError>;
    /// Number of guest PCM bytes queued on `stream` that have not finished playing yet.
    fn queued_bytes(&mut self, stream: u32) -> Result<u32, GuestSvcError>;
    /// Close `stream` and release its resources.
    fn close_stream(&mut self, stream: u32) -> Result<(), GuestSvcError>;
}

/// State of one stream inside [`FakeAudio`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeAudioStream {
    pub model: GuestAudioModel,
    pub queued: Vec<u8>,
    pub looped: Option<(u32, u32)>,
    pub paused: bool,
    pub volume: (u16, u16),
}

/// Deterministic in-memory [`GuestAudio`] implementor for host oracle tests.
///
/// Mirrors CoralOS `Aura.engine` grant enforcement (`not_granted`), format checks, per-stream
/// volume/pause/flush/loop state, and host-driven playback advancement via [`FakeAudio::play`].
#[derive(Clone, Debug, Default)]
pub struct FakeAudio {
    next_id: u32,
    streams: BTreeMap<u32, FakeAudioStream>,
    not_granted: bool,
    injected_error: Option<GuestSvcError>,
}

impl FakeAudio {
    /// A granted fake audio service with no open streams.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake audio service whose engine has no `(audio)` grant: every call returns `Err(GuestSvcError::NotGranted)`.
    pub fn not_granted() -> Self {
        Self { next_id: 0, streams: BTreeMap::new(), not_granted: true, injected_error: None }
    }

    /// Toggle whether the engine holds the `(audio)` grant.
    pub fn set_granted(&mut self, granted: bool) {
        self.not_granted = !granted;
    }

    /// Inject a specific error (`Lost`, `Refused("no-sink")`, etc.) for negative-control tests; `None` clears it.
    pub fn set_injected_error(&mut self, err: Option<GuestSvcError>) {
        self.injected_error = err;
    }

    /// Inspect all currently open streams.
    pub fn streams(&self) -> &BTreeMap<u32, FakeAudioStream> {
        &self.streams
    }

    /// Simulate the host playing (draining) up to `bytes` from `stream`; returns bytes actually drained.
    pub fn play(&mut self, stream: u32, bytes: usize) -> usize {
        let Some(s) = self.streams.get_mut(&stream) else { return 0 };
        let n = bytes.min(s.queued.len());
        s.queued.drain(..n);
        n
    }

    /// All currently queued PCM bytes across open streams (in stream-id order).
    pub fn all_queued(&self) -> Vec<u8> {
        self.streams.values().flat_map(|s| s.queued.iter().copied()).collect()
    }

    fn check_access(&self) -> Result<(), GuestSvcError> {
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        if let Some(err) = self.injected_error {
            return Err(err);
        }
        Ok(())
    }
}

impl GuestAudio for FakeAudio {
    fn open_stream(&mut self, model: &GuestAudioModel) -> Result<u32, GuestSvcError> {
        self.check_access()?;
        if !model.ok() {
            return Err(GuestSvcError::Refused("bad-format"));
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.streams.insert(
            id,
            FakeAudioStream {
                model: model.clone(),
                queued: Vec::new(),
                looped: None,
                paused: false,
                volume: (AUDIO_UNITY, AUDIO_UNITY),
            },
        );
        Ok(id)
    }

    fn queue_pcm(&mut self, stream: u32, pcm: &[u8]) -> Result<(), GuestSvcError> {
        self.check_access()?;
        if pcm.len() > AUDIO_MAX_PCM {
            return Err(GuestSvcError::TooLarge);
        }
        let s = self.streams.get_mut(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        s.queued.extend_from_slice(pcm);
        s.looped = None;
        Ok(())
    }

    fn queue_looped(
        &mut self,
        stream: u32,
        pcm: &[u8],
        loop_start: u32,
        loop_end: u32,
    ) -> Result<(), GuestSvcError> {
        self.check_access()?;
        if pcm.len() > AUDIO_MAX_PCM {
            return Err(GuestSvcError::TooLarge);
        }
        if loop_start >= loop_end {
            return Err(GuestSvcError::Refused("bad-format"));
        }
        let s = self.streams.get_mut(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        let fb = s.model.frame_bytes();
        if (loop_end as usize).saturating_mul(fb) > pcm.len() {
            return Err(GuestSvcError::Refused("bad-format"));
        }
        s.queued.extend_from_slice(pcm);
        s.looped = Some((loop_start, loop_end));
        Ok(())
    }

    fn flush(&mut self, stream: u32) -> Result<(), GuestSvcError> {
        self.check_access()?;
        let s = self.streams.get_mut(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        s.queued.clear();
        s.looped = None;
        Ok(())
    }

    fn pause(&mut self, stream: u32, paused: bool) -> Result<(), GuestSvcError> {
        self.check_access()?;
        let s = self.streams.get_mut(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        s.paused = paused;
        Ok(())
    }

    fn set_volume(&mut self, stream: u32, left: u16, right: u16) -> Result<(), GuestSvcError> {
        self.check_access()?;
        let s = self.streams.get_mut(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        s.volume = (left.min(AUDIO_UNITY), right.min(AUDIO_UNITY));
        Ok(())
    }

    fn queued_bytes(&mut self, stream: u32) -> Result<u32, GuestSvcError> {
        self.check_access()?;
        let s = self.streams.get(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        Ok(s.queued.len().min(u32::MAX as usize) as u32)
    }

    fn close_stream(&mut self, stream: u32) -> Result<(), GuestSvcError> {
        self.check_access()?;
        self.streams.remove(&stream).ok_or(GuestSvcError::Refused("bad-stream"))?;
        Ok(())
    }
}
