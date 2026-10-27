//! Host side (std, feature `host`): read a directory tree into a [`Drive`] for the runner and the tests; the engine itself stays
//! `no_std`. A file name is put as found when it is UTF-8; a name that is not is refused by name (never mangled into a different
//! name). The drive's lookup is case-insensitive.
use crate::drive::Drive;
use alloc::format;
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
}

impl DirRom {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        std::fs::read_dir(root)?;
        Ok(DirRom { root: root.to_path_buf() })
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
        std::fs::read(self.resolve(path)?).ok()
    }
    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_some_and(|p| p.exists())
    }
    fn list(&self, path: &str) -> Option<Vec<alloc::string::String>> {
        let rd = std::fs::read_dir(self.resolve(path)?).ok()?;
        Some(rd.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).collect())
    }
}
