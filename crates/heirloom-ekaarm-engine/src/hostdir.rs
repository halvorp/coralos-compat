//! Host side (std, feature `host`): read a directory tree into a [`Drive`] for the runner and the tests; the engine itself stays
//! `no_std`. A file name is put as found when it is UTF-8; a name that is not is refused by name (never mangled into a different
//! name). The drive's lookup is case-insensitive.
use crate::drive::Drive;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

pub fn load_dir(dir: &Path) -> std::io::Result<(Drive, usize)> {
    fn walk(drive: &mut Drive, guest: &str, host: &Path, files: &mut usize) -> std::io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(host)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().into_string()
                .map_err(|raw| std::io::Error::other(format!("{}: a file name that is not UTF-8 ({raw:?})", host.display())))?;
            let path = format!("{guest}\\{name}");
            let ty = e.file_type()?;
            if ty.is_dir() {
                walk(drive, &path, &e.path(), files)?;
            } else if ty.is_file() {
                drive.put_file(&path, std::fs::read(e.path())?).map_err(|err| std::io::Error::other(format!("{path}: {err:?}")))?;
                *files += 1;
            }
        }
        Ok(())
    }
    let mut drive = Drive::new();
    let mut files = 0;
    walk(&mut drive, "", dir, &mut files)?;
    Ok((drive, files))
}

/// A host directory as the device ROM: files are read on demand, names matched case-insensitively component by component.
pub struct DirRom {
    root: std::path::PathBuf,
    accessed: RomAccessLog,
}

/// The ROM paths a run touched (shared handle: the runner keeps one after the drive owns the `DirRom`). Each entry is the
/// resolved path relative to the ROM root, `/` separated, in the spelling found on the host; a directory that was listed or
/// tested carries a trailing `/`. A path that does not exist in the ROM is not recorded (there is nothing to ship for it).
#[derive(Clone, Default)]
pub struct RomAccessLog(std::rc::Rc<core::cell::RefCell<std::collections::BTreeSet<String>>>);

impl RomAccessLog {
    /// Sorted, unique.
    pub fn paths(&self) -> Vec<String> {
        self.0.borrow().iter().cloned().collect()
    }
}

impl DirRom {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        std::fs::read_dir(root)?;
        Ok(DirRom { root: root.to_path_buf(), accessed: RomAccessLog::default() })
    }

    /// The handle onto this ROM's access record (`--rom-manifest`).
    pub fn access_log(&self) -> RomAccessLog {
        self.accessed.clone()
    }

    fn note(&self, p: &Path) {
        if let Ok(rel) = p.strip_prefix(&self.root) {
            let mut s = rel.to_string_lossy().replace('\\', "/");
            if p.is_dir() { s.push('/'); }
            self.accessed.0.borrow_mut().insert(s);
        }
    }

    fn resolve(&self, path: &str) -> Option<std::path::PathBuf> {
        let rest = if path.len() >= 2 && path.as_bytes()[1] == b':' { &path[2..] } else { path };
        let mut cur = self.root.clone();
        for comp in rest.split(['\\', '/']).filter(|c| !c.is_empty()) {
            // A guest path names a file INSIDE the ROM: `..` (or `.`) would let it walk out of the root into the host's tree.
            if comp == ".." || comp == "." {
                return None;
            }
            let exact = cur.join(comp);
            if exact.exists() { cur = exact; continue; }
            let found = std::fs::read_dir(&cur).ok()?.filter_map(Result::ok).find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp))?;
            cur = found.path();
        }
        Some(cur)
    }
}

impl crate::drive::RomSource for DirRom {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let p = self.resolve(path)?;
        let bytes = std::fs::read(&p).ok()?;
        self.note(&p);
        Some(bytes)
    }
    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_some_and(|p| { let e = p.exists(); if e { self.note(&p); } e })
    }
    fn list(&self, path: &str) -> Option<Vec<alloc::string::String>> {
        let p = self.resolve(path)?;
        let rd = std::fs::read_dir(&p).ok()?;
        self.note(&p);
        Some(rd.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::RomSource;

    #[test]
    fn rom_access_log_records_resolved_existing_paths_sorted_unique() {
        let root = std::env::temp_dir().join(format!("heirloom-romlog-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Resource/Apps")).unwrap();
        std::fs::write(root.join("Resource/Apps/Avkon.RSC"), b"abc").unwrap();
        std::fs::write(root.join("Resource/Other.txt"), b"x").unwrap();
        let rom = DirRom::new(&root).unwrap();
        let log = rom.access_log();
        assert!(rom.read("z:\\resource\\apps\\avkon.rsc").is_some());
        assert!(rom.read("\\resource\\apps\\avkon.rsc").is_some()); // twice: one entry
        assert!(rom.exists("\\Resource\\Other.txt"));
        assert!(!rom.exists("\\resource\\missing.bin"));
        assert!(rom.read("\\resource\\missing2.bin").is_none());
        assert!(rom.list("\\resource\\apps").is_some());
        assert_eq!(log.paths(), ["Resource/Apps/", "Resource/Apps/Avkon.RSC", "Resource/Other.txt"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
