//! std-only: fill a [`Volume`] from a host directory. NOT part of the no_std library: the runner binary and the
//! tests include this file with `#[path]`, so the CoralOS build never sees `std`.
use heirloom_ami68k_engine::volume::Volume;
use std::path::Path;

/// What a load did.
#[derive(Debug, Default)]
pub struct Loaded { pub files: usize, pub bytes: usize, pub skipped: Vec<String> }

/// Copy the host directory `host` into the volume under `guest` (`Dev:dir`, created if missing). Names the guest
/// cannot hold (containing ':', too long) are skipped and listed, never silently dropped.
pub fn load_dir(vol: &mut Volume, guest: &str, host: &Path) -> std::io::Result<Loaded> {
    let mut out = Loaded::default();
    vol.mkdirs(guest).map_err(|e| std::io::Error::other(format!("{guest}: AmigaDOS error {}", e.0)))?;
    walk(vol, guest, host, &mut out)?;
    Ok(out)
}

fn walk(vol: &mut Volume, guest: &str, host: &Path, out: &mut Loaded) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(host)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if guest.ends_with(':') || guest.ends_with('/') { format!("{guest}{name}") } else { format!("{guest}/{name}") };
        let ty = e.file_type()?;
        let r = if ty.is_dir() {
            vol.mkdirs(&path).map(|_| ()).and_then(|_| { walk(vol, &path, &e.path(), out).map_err(|_| heirloom_ami68k_engine::dos_err::ERROR_OBJECT_NOT_FOUND) })
        } else if ty.is_file() {
            let data = std::fs::read(e.path())?;
            out.bytes += data.len();
            out.files += 1;
            vol.put_file(&path, data).map(|_| ())
        } else { Ok(()) };
        if let Err(code) = r { out.skipped.push(format!("{path}: AmigaDOS error {}", code.0)); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_tree_appears_under_its_guest_path() {
        let d = std::env::temp_dir().join(format!("a68k-vol-{}", std::process::id()));
        std::fs::create_dir_all(d.join("Libs/mui")).unwrap();
        std::fs::write(d.join("Libs/Foo.library"), b"abc").unwrap();
        std::fs::write(d.join("Libs/mui/X.mcc"), b"x").unwrap();
        std::fs::write(d.join("bad:name"), b"y").unwrap();
        let mut v = Volume::new();
        v.add_device("Prog5", false);
        let l = load_dir(&mut v, "Prog5:", &d).unwrap();
        assert_eq!((l.files, l.bytes), (3, 5));
        assert_eq!(l.skipped.len(), 1, "the name with a colon is reported: {:?}", l.skipped);
        let n = v.resolve(heirloom_ami68k_engine::volume::ROOT, "prog5:libs/mui/x.mcc").unwrap();
        assert_eq!(v.file_data(n).unwrap(), b"x");
        assert!(v.resolve(heirloom_ami68k_engine::volume::ROOT, "prog5:bad").is_err());
        std::fs::remove_dir_all(d).unwrap();
    }
}
