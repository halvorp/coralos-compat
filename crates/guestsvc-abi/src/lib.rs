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

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Maximum UTF-8 byte length of a preference key (matches `coral_prefs_abi::KEY_MAX`).
pub const KEY_MAX: usize = 128;

/// Maximum payload byte length of a `Text` or `Bytes` value (matches `coral_prefs_abi::VALUE_MAX`).
pub const VALUE_MAX: usize = 1024;

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
}
