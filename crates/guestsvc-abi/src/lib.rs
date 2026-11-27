//! Host-trait ABI between Heirloom guest-OS adapters and the engine process's native service sessions
//! (CoralOS `docs/heirloom/23-ENGINE-SPLIT.md` §2.4, component-model unit C5).
//!
//! Guest adapters remain sans-IO state machines: they call [`GuestPrefs`] (and future per-service traits)
//! with plain host types (`&str`, [`GuestValue`], `Vec<String>`), zero CoralOS wire types, and NEVER pick
//! a namespace. On CoralOS, `coral-heirloom-engine-rt::services::Services` implements [`GuestPrefs`] over
//! `prefs.service` in `Space::Own` (the namespace the loader admitted for the launched bundle). In host
//! oracle tests, [`FakePrefs`] provides a deterministic map-backed implementation with a `NotGranted` switch.
#![no_std]
extern crate alloc;

pub mod apparc;
pub use apparc::{
    FakeAppArc, GuestAppArc, GuestTypeInfo, APPARC_MIME_MAX, APPARC_NAME_MAX, APPARC_SNIFF_MAX,
    CONF_CERTAIN, CONF_NOT_RECOGNIZED, CONF_POSSIBLE, CONF_PROBABLE, CONF_UNLIKELY,
};
pub mod audio;
pub use audio::{
    FakeAudio, FakeAudioStream, GuestAudio, GuestAudioFormat, GuestAudioModel, AUDIO_LABEL_MAX,
    AUDIO_MAX_PCM, AUDIO_UNITY,
};
pub mod messages;
pub use messages::{
    attachment_ref_ok, FakeMessages, GuestAttachmentRef, GuestConv, GuestDraft, GuestMessages,
    GuestRecipient, GuestRecipientKind, MSG_ATTACHMENTS_MAX, MSG_BODY_MAX, MSG_DRAFTS_MAX,
    MSG_ID_MAX, MSG_MIME_MAX, MSG_PATH_MAX, MSG_RECIPIENTS_MAX, MSG_SUBJECT_MAX,
};
pub mod net;
pub use net::{
    host_ok, FakeConn, FakeNet, GuestNet, NET_EVT_EOF, NET_EVT_READABLE, NET_EVT_RESET,
    NET_EVT_WRITABLE, NET_HOST_MAX, NET_IO_MAX, NET_MAX_CONNS,
};
pub mod storage;
pub use storage::{
    rel_dir_ok, rel_path_ok, FakeStorage, GuestStorage, STORAGE_FILE_MAX, STORAGE_PATH_MAX,
};

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Maximum UTF-8 byte length of a preference key (matches `coral_prefs_abi::KEY_MAX`).
pub const KEY_MAX: usize = 128;

/// Maximum payload byte length of a `Text` or `Bytes` value (matches `coral_prefs_abi::VALUE_MAX`).
pub const VALUE_MAX: usize = 1024;

/// Maximum UTF-8 byte length of a clipboard MIME type (matches `coral_clip_abi::MIME_MAX`).
pub const CLIP_MIME_MAX: usize = 64;

/// Maximum payload byte length of a clipboard entry across the guest service seam.
pub const CLIP_PAYLOAD_MAX: usize = 64 * 1024;

/// A preference value as a guest adapter sees it: the five shapes `prefs.service` stores,
/// with no wire type in the signature.
#[derive(Clone, Debug, PartialEq)]
pub enum GuestValue {
    Bool(bool),
    Int(i64),
    /// Finite only: NaN and infinities are refused (`GuestSvcError::Refused("bad-value")`).
    Real(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl GuestValue {
    /// Whether the value is storable: finite real, and payload within [`VALUE_MAX`].
    pub fn ok(&self) -> bool {
        match self {
            GuestValue::Bool(_) | GuestValue::Int(_) => true,
            GuestValue::Real(r) => r.is_finite(),
            GuestValue::Text(t) => t.len() <= VALUE_MAX,
            GuestValue::Bytes(b) => b.len() <= VALUE_MAX,
        }
    }
}

/// Why a service call produced no answer. A closed, named set: a guest adapter maps each to its
/// own guest error, and a log line names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestSvcError {
    /// This engine has no authority for the service (no grant, service not running, or launch not admitted).
    NotGranted,
    /// The service refused by name (e.g. `bad-value`, `too-large`, `stale-incarnation`).
    Refused(&'static str),
    /// The key or prefix is outside the allowed grammar; nothing was sent.
    BadName,
    /// The value or request does not fit one message; nothing was sent.
    TooLarge,
    /// The service died or restarted while the request was in flight and the retry did not answer.
    Lost,
    /// The service did not answer within the bounded wait.
    Stalled,
    /// The reply did not decode or was not the shape the request expects.
    Unreadable,
}

impl GuestSvcError {
    /// The canonical diagnostic name for log lines.
    pub const fn name(self) -> &'static str {
        match self {
            GuestSvcError::NotGranted => "not-granted",
            GuestSvcError::Refused(why) => why,
            GuestSvcError::BadName => "bad-name",
            GuestSvcError::TooLarge => "too-large",
            GuestSvcError::Lost => "lost",
            GuestSvcError::Stalled => "stalled",
            GuestSvcError::Unreadable => "unreadable",
        }
    }
}

fn word_ok(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// Whether `s` is a valid preference key: `/`-separated `[A-Za-z0-9._-]` words, no `.` or `..`, 1..=[`KEY_MAX`] bytes.
pub fn key_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= KEY_MAX && s.split('/').all(word_ok)
}

/// Whether `s` is a valid key prefix: empty (all keys) or a valid key with optional trailing `/`.
pub fn prefix_ok(s: &str) -> bool {
    s.is_empty() || key_ok(s.strip_suffix('/').unwrap_or(s))
}

/// Whether `s` is a valid clipboard MIME type: `type/subtype[;param=value]`, ASCII printable, no whitespace, 1..=[`CLIP_MIME_MAX`].
pub fn mime_ok(s: &str) -> bool {
    if s.is_empty() || s.len() > CLIP_MIME_MAX {
        return false;
    }
    let Some((ty, rest)) = s.split_once('/') else { return false };
    !ty.is_empty()
        && !rest.is_empty()
        && !rest.starts_with(';')
        && s.bytes().all(|b| b.is_ascii_graphic())
}

/// System clipboard for a guest, backed by CoralOS `clipboardd` (with C12 `Watch` + doorbell, zero polling).
///
/// Adapters translate guest clipboard formats (Mac `'TEXT'` / `'PICT'`, Amiga IFF `FORM FTXT`) to/from
/// MIME types + raw bytes and NEVER keep a private guest-only fallback when the service refuses (`NotGranted`).
pub trait GuestClipboard {
    /// Publish `(mime, payload)` to the system clipboard; returns the resulting clipboard generation.
    fn copy(&mut self, mime: &str, payload: &[u8]) -> Result<u64, GuestSvcError>;
    /// Read the current system clipboard entry as `(mime, payload)`, or `None` when empty.
    fn paste(&mut self) -> Result<Option<(String, Vec<u8>)>, GuestSvcError>;
    /// Current clipboard generation (monotonically advances whenever the clipboard changes; non-polling on CoralOS).
    fn generation(&mut self) -> Result<u64, GuestSvcError>;
}

/// Deterministic in-memory [`GuestClipboard`] implementor for host oracle tests.
///
/// Mirrors CoralOS `clipboardd` MIME/size validation, generation increments on copy (including native copies
/// via [`FakeClipboard::simulate_native_copy`]), and `(clip)` grant enforcement (`not_granted`).
#[derive(Clone, Debug, Default)]
pub struct FakeClipboard {
    current: Option<(String, Vec<u8>)>,
    generation: u64,
    not_granted: bool,
    injected_error: Option<GuestSvcError>,
}

impl FakeClipboard {
    /// An empty granted fake clipboard at generation 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake clipboard whose engine has no `(clip)` grant: every call returns `Err(GuestSvcError::NotGranted)`.
    pub fn not_granted() -> Self {
        Self { current: None, generation: 0, not_granted: true, injected_error: None }
    }

    /// Toggle whether the engine holds the `(clip)` grant.
    pub fn set_granted(&mut self, granted: bool) {
        self.not_granted = !granted;
    }

    /// Inject a specific error (`Lost`, `Stalled`, etc.) for negative-control tests; `None` clears it.
    pub fn set_injected_error(&mut self, err: Option<GuestSvcError>) {
        self.injected_error = err;
    }

    /// Simulate a native CoralOS app copying `(mime, payload)` to `clipboardd`: replaces the entry and advances generation.
    pub fn simulate_native_copy(&mut self, mime: &str, payload: &[u8]) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.current = Some((String::from(mime), payload.to_vec()));
        self.generation
    }

    /// Inspect the stored `(mime, payload)` directly in tests.
    pub fn current(&self) -> Option<(&str, &[u8])> {
        self.current.as_ref().map(|(m, d)| (m.as_str(), d.as_slice()))
    }

    /// Inspect the current generation directly in tests.
    pub fn current_generation(&self) -> u64 {
        self.generation
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

impl GuestClipboard for FakeClipboard {
    fn copy(&mut self, mime: &str, payload: &[u8]) -> Result<u64, GuestSvcError> {
        if !mime_ok(mime) {
            return Err(GuestSvcError::BadName);
        }
        if payload.len() > CLIP_PAYLOAD_MAX {
            return Err(GuestSvcError::TooLarge);
        }
        self.check_access()?;
        self.generation = self.generation.wrapping_add(1);
        self.current = Some((String::from(mime), payload.to_vec()));
        Ok(self.generation)
    }

    fn paste(&mut self) -> Result<Option<(String, Vec<u8>)>, GuestSvcError> {
        self.check_access()?;
        Ok(self.current.clone())
    }

    fn generation(&mut self) -> Result<u64, GuestSvcError> {
        self.check_access()?;
        Ok(self.generation)
    }
}

/// Preferences for a guest, in the engine's own admitted namespace.
///
/// Adapters choose keys only (e.g. `cenrep/<uid_hex>/<key_hex>`, `dict/<uid3_hex>/<stream_hex>`),
/// NEVER a namespace: the host forces `Space::Own`.
pub trait GuestPrefs {
    /// The value at `key`; `None` when absent.
    fn get(&mut self, key: &str) -> Result<Option<GuestValue>, GuestSvcError>;
    /// Store `value` at `key` (last writer wins).
    fn set(&mut self, key: &str, value: &GuestValue) -> Result<(), GuestSvcError>;
    /// Remove `key`; removing an absent key is `Ok(())`.
    fn delete(&mut self, key: &str) -> Result<(), GuestSvcError>;
    /// Every key under `prefix`, in ascending key order.
    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, GuestSvcError>;
}

/// Deterministic in-memory [`GuestPrefs`] implementor for host oracle tests.
///
/// Enforces the exact same key grammar (`key_ok` / `prefix_ok`), value bounds (`VALUE_MAX`, finite `f64`),
/// and grant enforcement (`not_granted`) as CoralOS `prefs.service`.
#[derive(Clone, Debug, Default)]
pub struct FakePrefs {
    entries: BTreeMap<String, GuestValue>,
    not_granted: bool,
    injected_error: Option<GuestSvcError>,
}

impl FakePrefs {
    /// An empty granted fake preferences store.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake preferences store whose engine has no `(prefs)` grant: every call returns `Err(GuestSvcError::NotGranted)`.
    pub fn not_granted() -> Self {
        Self { entries: BTreeMap::new(), not_granted: true, injected_error: None }
    }

    /// Toggle whether the engine holds the `(prefs)` grant.
    pub fn set_granted(&mut self, granted: bool) {
        self.not_granted = !granted;
    }

    /// Inject a specific error (`Lost`, `Stalled`, `Refused(...)`, etc.) for negative-control tests; `None` clears it.
    pub fn set_injected_error(&mut self, err: Option<GuestSvcError>) {
        self.injected_error = err;
    }

    /// Inspect stored entries directly in tests.
    pub fn entries(&self) -> &BTreeMap<String, GuestValue> {
        &self.entries
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

impl GuestPrefs for FakePrefs {
    fn get(&mut self, key: &str) -> Result<Option<GuestValue>, GuestSvcError> {
        if !key_ok(key) {
            return Err(GuestSvcError::BadName);
        }
        self.check_access()?;
        Ok(self.entries.get(key).cloned())
    }

    fn set(&mut self, key: &str, value: &GuestValue) -> Result<(), GuestSvcError> {
        if !key_ok(key) {
            return Err(GuestSvcError::BadName);
        }
        match value {
            GuestValue::Text(t) if t.len() > VALUE_MAX => return Err(GuestSvcError::TooLarge),
            GuestValue::Bytes(b) if b.len() > VALUE_MAX => return Err(GuestSvcError::TooLarge),
            GuestValue::Real(r) if !r.is_finite() => return Err(GuestSvcError::Refused("bad-value")),
            _ => {}
        }
        self.check_access()?;
        self.entries.insert(String::from(key), value.clone());
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<(), GuestSvcError> {
        if !key_ok(key) {
            return Err(GuestSvcError::BadName);
        }
        self.check_access()?;
        self.entries.remove(key);
        Ok(())
    }

    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, GuestSvcError> {
        if !prefix_ok(prefix) {
            return Err(GuestSvcError::BadName);
        }
        self.check_access()?;
        Ok(self
            .entries
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn crud_and_prefix_listing_round_trip() {
        let mut prefs = FakePrefs::new();
        assert_eq!(prefs.get("cenrep/101f876d/00000001"), Ok(None));
        assert_eq!(prefs.set("cenrep/101f876d/00000001", &GuestValue::Int(42)), Ok(()));
        assert_eq!(prefs.set("cenrep/101f876d/00000002", &GuestValue::Text("hello".into())), Ok(()));
        assert_eq!(prefs.set("cenrep/101f876d/00000003", &GuestValue::Real(3.5)), Ok(()));
        assert_eq!(prefs.set("cenrep/101f876d/00000004", &GuestValue::Bool(true)), Ok(()));
        assert_eq!(prefs.set("dict/102828d6/00001234", &GuestValue::Bytes(vec![1, 2, 3])), Ok(()));

        assert_eq!(prefs.get("cenrep/101f876d/00000001"), Ok(Some(GuestValue::Int(42))));
        assert_eq!(prefs.get("dict/102828d6/00001234"), Ok(Some(GuestValue::Bytes(vec![1, 2, 3]))));
        assert_eq!(
            prefs.keys("cenrep/101f876d/").unwrap(),
            vec![
                String::from("cenrep/101f876d/00000001"),
                String::from("cenrep/101f876d/00000002"),
                String::from("cenrep/101f876d/00000003"),
                String::from("cenrep/101f876d/00000004"),
            ]
        );
        assert_eq!(prefs.delete("cenrep/101f876d/00000001"), Ok(()));
        assert_eq!(prefs.get("cenrep/101f876d/00000001"), Ok(None));
        // Deleting an absent key is idempotent Ok(()).
        assert_eq!(prefs.delete("cenrep/101f876d/00000001"), Ok(()));
    }

    #[test]
    fn negative_controls_enforce_grant_grammar_and_value_bounds() {
        let mut denied = FakePrefs::not_granted();
        assert_eq!(denied.get("cenrep/101f876d/00000001"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.set("cenrep/101f876d/00000001", &GuestValue::Int(1)), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.delete("cenrep/101f876d/00000001"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.keys(""), Err(GuestSvcError::NotGranted));
        assert!(denied.entries().is_empty());

        let mut prefs = FakePrefs::new();
        let long_key = "k".repeat(KEY_MAX + 1);
        for bad in ["", ".", "..", "a/../b", "a//b", "/leading", "trailing/", "a b", long_key.as_str()] {
            assert_eq!(prefs.get(bad), Err(GuestSvcError::BadName), "{bad:?}");
            assert_eq!(prefs.set(bad, &GuestValue::Bool(true)), Err(GuestSvcError::BadName), "{bad:?}");
            assert_eq!(prefs.delete(bad), Err(GuestSvcError::BadName), "{bad:?}");
        }
        assert_eq!(prefs.keys("a b"), Err(GuestSvcError::BadName));
        assert_eq!(prefs.keys("a/../b"), Err(GuestSvcError::BadName));

        // Oversized values and non-finite floats are refused cleanly.
        assert_eq!(prefs.set("k", &GuestValue::Text("x".repeat(VALUE_MAX + 1))), Err(GuestSvcError::TooLarge));
        assert_eq!(prefs.set("k", &GuestValue::Bytes(vec![0u8; VALUE_MAX + 1])), Err(GuestSvcError::TooLarge));
        assert_eq!(prefs.set("k", &GuestValue::Real(f64::NAN)), Err(GuestSvcError::Refused("bad-value")));
        assert_eq!(prefs.set("k", &GuestValue::Real(f64::INFINITY)), Err(GuestSvcError::Refused("bad-value")));
        assert_eq!(GuestSvcError::NotGranted.name(), "not-granted");
        assert_eq!(GuestSvcError::Lost.name(), "lost");
        assert_eq!(GuestSvcError::Stalled.name(), "stalled");
        assert_eq!(GuestSvcError::Unreadable.name(), "unreadable");
        assert_eq!(GuestSvcError::Refused("bad-value").name(), "bad-value");
    }

    #[test]
    fn fake_clipboard_round_trip_and_negative_controls() {
        let mut clip = FakeClipboard::new();
        assert_eq!(clip.generation(), Ok(0));
        assert_eq!(clip.paste(), Ok(None));

        let g1 = clip.copy("text/plain;charset=utf-8", b"hello coral").unwrap();
        assert_eq!(g1, 1);
        assert_eq!(clip.generation(), Ok(1));
        assert_eq!(
            clip.paste(),
            Ok(Some((String::from("text/plain;charset=utf-8"), b"hello coral".to_vec())))
        );

        let g2 = clip.simulate_native_copy("text/plain;charset=utf-8", b"from native");
        assert_eq!(g2, 2);
        assert_eq!(clip.generation(), Ok(2));
        assert_eq!(
            clip.paste(),
            Ok(Some((String::from("text/plain;charset=utf-8"), b"from native".to_vec())))
        );

        // Negative controls: bad MIME, oversized payload, NotGranted.
        for bad in ["", "text", "/plain", "text/", "text/;a=1", "text/plain with space"] {
            assert_eq!(clip.copy(bad, b"x"), Err(GuestSvcError::BadName), "{bad:?}");
        }
        assert_eq!(clip.copy("text/plain", &vec![0u8; CLIP_PAYLOAD_MAX + 1]), Err(GuestSvcError::TooLarge));

        let mut denied = FakeClipboard::not_granted();
        assert_eq!(denied.copy("text/plain", b"x"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.paste(), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.generation(), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.current(), None);
    }

    #[test]
    fn fake_messages_round_trip_and_negative_controls() {
        let mut msg = FakeMessages::new();
        let accts = msg.accounts().unwrap();
        assert_eq!(accts, vec![String::from("matrix:@user:coral.lan")]);
        let acct = &accts[0];
        let to = vec![GuestRecipient { kind: GuestRecipientKind::To, addr: String::from("@bob:coral.lan") }];
        let att = vec![GuestAttachmentRef {
            path: String::from("Users/Default/Documents/notes.txt"),
            mime: String::from("text/plain"),
            name: String::from("notes.txt"),
        }];
        let id = msg.save_draft(acct, None, None, &to, "Hello", "World", &att).unwrap();
        assert_eq!(id, 1);
        assert_eq!(msg.drafts(acct).unwrap().len(), 1);
        let out_id = msg.send_draft(acct, id).unwrap();
        assert_eq!(out_id, 1);
        assert!(msg.drafts(acct).unwrap().is_empty());
        assert_eq!(msg.sent_drafts().len(), 1);

        // Negative controls: path traversal refused; NotGranted refuses every verb.
        let bad_att = vec![GuestAttachmentRef { path: String::from("../secret"), mime: String::new(), name: String::new() }];
        assert_eq!(msg.save_draft(acct, None, None, &to, "S", "B", &bad_att), Err(GuestSvcError::Refused("bad-request")));

        let mut denied = FakeMessages::not_granted();
        assert_eq!(denied.accounts(), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.save_draft(acct, None, None, &to, "S", "B", &[]), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.send_draft(acct, 1), Err(GuestSvcError::NotGranted));
    }

    #[test]
    fn fake_audio_round_trip_and_negative_controls() {
        let mut aud = FakeAudio::new();
        let model = GuestAudioModel {
            rate_hz: 22_254,
            channels: 1,
            format: GuestAudioFormat::U8,
            label: String::from("Sound Manager"),
        };
        let id = aud.open_stream(&model).unwrap();
        assert!(id > 0);
        aud.queue_pcm(id, &[0x80, 0x90, 0xA0, 0x90]).unwrap();
        assert_eq!(aud.queued_bytes(id), Ok(4));
        aud.set_volume(id, 0x0080, 0x0100).unwrap();
        assert_eq!(aud.streams()[&id].volume, (0x0080, 0x0100));
        aud.pause(id, true).unwrap();
        assert!(aud.streams()[&id].paused);
        assert_eq!(aud.play(id, 2), 2);
        assert_eq!(aud.queued_bytes(id), Ok(2));
        aud.flush(id).unwrap();
        assert_eq!(aud.queued_bytes(id), Ok(0));
        aud.queue_looped(id, &[0x80, 0x90, 0xA0, 0x90], 1, 3).unwrap();
        assert_eq!(aud.streams()[&id].looped, Some((1, 3)));
        aud.close_stream(id).unwrap();
        assert!(aud.streams().is_empty());

        // Negative controls: bad format, unknown stream, bad loop bounds, NotGranted.
        let bad_model = GuestAudioModel { rate_hz: 0, channels: 1, format: GuestAudioFormat::U8, label: String::new() };
        assert_eq!(aud.open_stream(&bad_model), Err(GuestSvcError::Refused("bad-format")));
        assert_eq!(aud.queued_bytes(id), Err(GuestSvcError::Refused("bad-stream")));

        let mut denied = FakeAudio::not_granted();
        assert_eq!(denied.open_stream(&model), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.queue_pcm(1, &[0x80]), Err(GuestSvcError::NotGranted));
        assert!(denied.streams().is_empty());
    }

    #[test]
    fn fake_net_round_trip_and_negative_controls() {
        let mut net = FakeNet::new();
        net.add_host("coral.lan", [10, 0, 2, 15]);
        assert_eq!(net.resolve("coral.lan"), Ok([10, 0, 2, 15]));
        assert_eq!(net.resolve("127.0.0.1"), Ok([127, 0, 0, 1]));
        let c = net.connect([10, 0, 2, 15], 22).unwrap();
        assert_eq!(net.send(c, b"SSH-2.0\r\n"), Ok(9));
        assert_eq!(net.conns()[&c].tx.as_slice(), b"SSH-2.0\r\n");
        let mut buf = [0u8; 16];
        assert_eq!(net.recv(c, &mut buf), Ok(0));
        net.inject_rx(c, b"OK");
        assert_eq!(net.poll_events(c).unwrap() & NET_EVT_READABLE, NET_EVT_READABLE);
        assert_eq!(net.recv(c, &mut buf), Ok(2));
        assert_eq!(&buf[..2], b"OK");
        net.inject_eof(c);
        assert_eq!(net.recv(c, &mut buf), Err(GuestSvcError::Refused("eof")));
        net.shutdown_write(c).unwrap();
        net.close(c).unwrap();
        assert!(net.conns().is_empty());

        // Negative controls: unknown host, bad name, port 0, NotGranted.
        assert_eq!(net.resolve("missing.invalid"), Err(GuestSvcError::Refused("dns")));
        assert_eq!(net.resolve("bad host"), Err(GuestSvcError::BadName));
        assert_eq!(net.connect([10, 0, 0, 1], 0), Err(GuestSvcError::Refused("conn-refused")));
        let mut denied = FakeNet::not_granted();
        assert_eq!(denied.resolve("coral.lan"), Err(GuestSvcError::NotGranted));
        assert_eq!(denied.connect([10, 0, 2, 15], 22), Err(GuestSvcError::NotGranted));
    }

    #[test]
    fn fake_apparc_and_storage_round_trip_and_negative_controls() {
        let mut aa = FakeAppArc::new();
        aa.add_magic(b"\x89PNG", "image/png", CONF_CERTAIN);
        aa.add_handler("image/png", "Preview");
        let r = aa.recognize_data("pic.png", b"\x89PNG\r\n").unwrap();
        assert_eq!((r.mime.as_str(), r.confidence), ("image/png", CONF_CERTAIN));
        assert_eq!(aa.handler_for("image/png", "pic.png"), Ok(Some(String::from("Preview"))));
        assert_eq!(aa.handler_for("application/x-unknown", "x.bin"), Ok(None));
        let mut denied_aa = FakeAppArc::not_granted();
        assert_eq!(denied_aa.recognize_data("a", b""), Err(GuestSvcError::NotGranted));

        let mut st = FakeStorage::new();
        st.write_file("saves/slot1.dat", b"gold=42").unwrap();
        assert_eq!(st.read_file("saves/slot1.dat"), Ok(Some(b"gold=42".to_vec())));
        assert_eq!(st.list_files("saves"), Ok(vec![String::from("saves/slot1.dat")]));
        st.delete_file("saves/slot1.dat").unwrap();
        assert_eq!(st.read_file("saves/slot1.dat"), Ok(None));

        for bad in ["", "/abs", "../escape", "a/../b", "SYS:foo", "a\\b", "dir/"] {
            assert_eq!(st.write_file(bad, b"x"), Err(GuestSvcError::BadName), "{bad:?}");
        }
        assert_eq!(st.write_file("big.bin", &vec![0u8; STORAGE_FILE_MAX + 1]), Err(GuestSvcError::TooLarge));
        let mut denied_st = FakeStorage::not_granted();
        assert_eq!(denied_st.write_file("a.dat", b"x"), Err(GuestSvcError::NotGranted));
    }
}
