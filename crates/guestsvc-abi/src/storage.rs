//! Host-trait ABI for per-bundle guest persistence on CoralFS (REUSE-AUDIT U19, `G9`).
//!
//! Guest adapters (Amiga writable volume delta, Mac prefs/saves, Symbian private data dir) never
//! pick a CoralFS path outside their own bundle's admitted data root (`Users/Default/AppData/<bundle>/`):
//! every relative path is validated by [`rel_path_ok`] (rejecting `..`, leading `/`, `:`, `\`, `\0`)
//! before crossing the seam, and `StorageService` in `coral-heirloom-engine-rt` scopes every I/O
//! strictly inside the bundle's CoralFS data directory.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::GuestSvcError;

/// Maximum UTF-8 byte length of a relative path inside the bundle's data directory.
pub const STORAGE_PATH_MAX: usize = 256;

/// Maximum byte size of one persisted file written across [`GuestStorage::write_file`] (1 MiB).
pub const STORAGE_FILE_MAX: usize = 1024 * 1024;

/// Whether `path` is a safe strictly-relative path inside a bundle's CoralFS data directory.
pub fn rel_path_ok(path: &str) -> bool {
    if path.is_empty()
        || path.len() > STORAGE_PATH_MAX
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
    {
        return false;
    }
    path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Whether `dir` is either `""` (the bundle data root) or a safe relative directory path.
pub fn rel_dir_ok(dir: &str) -> bool {
    dir.is_empty() || rel_path_ok(dir.trim_end_matches('/'))
}

/// Per-bundle CoralFS persistence owned by the engine process (`Users/Default/AppData/<bundle>/`).
pub trait GuestStorage {
    /// Read a persisted file at `rel_path`; `Ok(None)` when absent.
    fn read_file(&mut self, rel_path: &str) -> Result<Option<Vec<u8>>, GuestSvcError>;
    /// Atomically store `data` at `rel_path` (bounded by [`STORAGE_FILE_MAX`]).
    fn write_file(&mut self, rel_path: &str, data: &[u8]) -> Result<(), GuestSvcError>;
    /// Remove the file at `rel_path` (idempotent `Ok(())` when absent).
    fn delete_file(&mut self, rel_path: &str) -> Result<(), GuestSvcError>;
    /// List relative file paths under `rel_dir` (`""` = all files in the bundle data root).
    fn list_files(&mut self, rel_dir: &str) -> Result<Vec<String>, GuestSvcError>;
}

/// Deterministic in-memory [`GuestStorage`] implementor for host oracle tests.
#[derive(Clone, Debug, Default)]
pub struct FakeStorage {
    files: BTreeMap<String, Vec<u8>>,
    not_granted: bool,
}

impl FakeStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn not_granted() -> Self {
        Self { files: BTreeMap::new(), not_granted: true }
    }

    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }
}

impl GuestStorage for FakeStorage {
    fn read_file(&mut self, rel_path: &str) -> Result<Option<Vec<u8>>, GuestSvcError> {
        if !rel_path_ok(rel_path) {
            return Err(GuestSvcError::BadName);
        }
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        Ok(self.files.get(rel_path).cloned())
    }

    fn write_file(&mut self, rel_path: &str, data: &[u8]) -> Result<(), GuestSvcError> {
        if !rel_path_ok(rel_path) {
            return Err(GuestSvcError::BadName);
        }
        if data.len() > STORAGE_FILE_MAX {
            return Err(GuestSvcError::TooLarge);
        }
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        self.files.insert(String::from(rel_path), data.to_vec());
        Ok(())
    }

    fn delete_file(&mut self, rel_path: &str) -> Result<(), GuestSvcError> {
        if !rel_path_ok(rel_path) {
            return Err(GuestSvcError::BadName);
        }
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        self.files.remove(rel_path);
        Ok(())
    }

    fn list_files(&mut self, rel_dir: &str) -> Result<Vec<String>, GuestSvcError> {
        if !rel_dir_ok(rel_dir) {
            return Err(GuestSvcError::BadName);
        }
        if self.not_granted {
            return Err(GuestSvcError::NotGranted);
        }
        let prefix = if rel_dir.is_empty() {
            String::new()
        } else {
            let mut p = String::from(rel_dir.trim_end_matches('/'));
            p.push('/');
            p
        };
        Ok(self.files.keys().filter(|k| k.starts_with(&prefix)).cloned().collect())
    }
}
