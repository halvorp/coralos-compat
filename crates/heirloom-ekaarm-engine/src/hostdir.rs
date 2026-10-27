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
