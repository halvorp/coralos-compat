//! Host-trait ABI for guest MIME/type recognition and default-tool handler lookup over CoralOS's
//! `coral-descriptor` and `coral-opener` (REUSE-AUDIT U19, `G11`).
//!
//! Guest adapters (`Symbian RApaLsSession`, etc.) never ship their own magic-byte tables or app
//! registries: they ask [`GuestAppArc`] to sniff bytes via `coral-descriptor` and pick the native
//! CoralOS opener via `coral-opener`.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::GuestSvcError;

/// Maximum UTF-8 byte length of a document name passed to [`GuestAppArc::recognize_data`].
pub const APPARC_NAME_MAX: usize = 256;
/// Maximum leading sample bytes sniffed for content recognition (matches `coral_descriptor::SNIFF_BYTES` / 4 KiB).
pub const APPARC_SNIFF_MAX: usize = 4096;
/// Maximum UTF-8 byte length of a recognized MIME type string.
pub const APPARC_MIME_MAX: usize = 128;

/// Symbian `CApaDataRecognizerType::TRecognitionConfidence` levels.
pub const CONF_NOT_RECOGNIZED: i32 = i32::MIN;
pub const CONF_UNLIKELY: i32 = -10_000;
pub const CONF_POSSIBLE: i32 = 0;
pub const CONF_PROBABLE: i32 = 10_000;
pub const CONF_CERTAIN: i32 = i32::MAX;

/// Recognition verdict returned by [`GuestAppArc::recognize_data`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestTypeInfo {
    /// Canonical MIME type (e.g. `"image/png"`, `"text/plain"`, empty when not recognized).
    pub mime: String,
    /// Confidence level (`CONF_NOT_RECOGNIZED`..=`CONF_CERTAIN`).
    pub confidence: i32,
}

/// Content recognition and handler resolution owned by the engine process (`coral-descriptor` + `coral-opener`).
pub trait GuestAppArc {
    /// Sniff `(name, head)` via CoralOS `coral-descriptor` and return `(mime, confidence)`.
    fn recognize_data(&mut self, name: &str, head: &[u8]) -> Result<GuestTypeInfo, GuestSvcError>;
    /// Resolve which installed CoralOS bundle opens `(mime, name)` via `coral-opener::choose`.
    /// Returns `Ok(Some(stem))` when a tool claims it, `Ok(None)` for `Choice::NoTool`.
    fn handler_for(&mut self, mime: &str, name: &str) -> Result<Option<String>, GuestSvcError>;
}

/// Deterministic in-memory [`GuestAppArc`] implementor for host tests.
#[derive(Clone, Debug, Default)]
pub struct FakeAppArc {
    rules: Vec<(Vec<u8>, String, i32)>,
    ext_rules: BTreeMap<String, String>,
    handlers: BTreeMap<String, String>,
    not_granted: bool,
}

impl FakeAppArc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn not_granted() -> Self {
        Self { not_granted: true, ..Self::default() }
    }

    pub fn add_magic(&mut self, prefix: &[u8], mime: &str, confidence: i32) {
        self.rules.push((prefix.to_vec(), String::from(mime), confidence));
    }

    pub fn add_ext(&mut self, ext: &str, mime: &str) {
        self.ext_rules.insert(ext.to_ascii_lowercase(), String::from(mime));
    }

    pub fn add_handler(&mut self, mime: &str, stem: &str) {
        self.handlers.insert(mime.to_ascii_lowercase(), String::from(stem));
    }
}

impl GuestAppArc for FakeAppArc {
    fn recognize_data(&mut self, name: &str, head: &[u8]) -> Result<GuestTypeInfo, GuestSvcError> {
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        for (prefix, mime, conf) in &self.rules {
            if head.starts_with(prefix) {
                return Ok(GuestTypeInfo { mime: mime.clone(), confidence: *conf });
            }
        }
        if let Some((_, ext)) = name.rsplit_once('.') {
            if let Some(mime) = self.ext_rules.get(&ext.to_ascii_lowercase()) {
                return Ok(GuestTypeInfo { mime: mime.clone(), confidence: CONF_POSSIBLE });
            }
        }
        Ok(GuestTypeInfo { mime: String::new(), confidence: CONF_NOT_RECOGNIZED })
    }

    fn handler_for(&mut self, mime: &str, _name: &str) -> Result<Option<String>, GuestSvcError> {
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        Ok(self.handlers.get(&mime.to_ascii_lowercase()).cloned())
    }
}
