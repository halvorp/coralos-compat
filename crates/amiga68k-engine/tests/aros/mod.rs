//! The trimmed AROS volume of /data/build/corpus-bin (docs/CORPUS.md "the mini volume") and a REAL AROS program started on it. Every
//! test using it SKIPS loudly when the corpus is absent. Shared by the tests that drive Zune programs with classes on disk.
#![allow(dead_code)]
use amiga68k_engine::{Amiga, Step};
use guestui_abi::RecordingGuestUi;

pub const ROOT: &str = "/data/build/corpus-bin";

/// Copy a host directory into the guest volume under `prefix` (files and sub-directories), leaving out every path that starts with a `skip` entry.
pub fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path, skip: &[&str]) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        if skip.iter().any(|s| path.to_ascii_lowercase().starts_with(&s.to_ascii_lowercase())) { continue; }
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path(), skip); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

/// A machine with the mini volume on SYS: (and `LIBS:` = `SYS:Libs`), and AROS program `rel` (under `sys/`) loaded but not yet run.
/// `without` names volume paths (prefixes) never put on the volume: a negative control, the same program with a file missing.
pub fn machine_for(rel: &str, without: &[&str]) -> Option<Amiga<RecordingGuestUi>> {
    let file = std::fs::read(format!("{ROOT}/aros-m68k/sys/{rel}")).ok().or_else(|| { eprintln!("SKIPPED: {ROOT}/aros-m68k/sys/{rel} not present (see docs/CORPUS.md)"); None })?;
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    load(&mut a, "SYS:", std::path::Path::new(&format!("{ROOT}/aros-m68k/mini")), without);
    a.fs.vol.assign("LIBS", "SYS:Libs");
    a.stop_on_unimplemented = true;
    a.start_hunk_as(&file, rel.rsplit('/').next().unwrap()).unwrap();
    Some(a)
}

/// [`machine_for`], run until the program blocks (it is waiting for input) or exits.
pub fn start(rel: &str, without: &[&str]) -> Option<(Amiga<RecordingGuestUi>, Step)> {
    let mut a = machine_for(rel, without)?;
    let (s, _) = a.step(40_000_000);
    Some((a, s))
}
