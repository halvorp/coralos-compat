//! The dos2-lane census on real, unmodified Directory Opus 5.102 binaries (not committed: /data/build/corpus-bin; each
//! test says so and skips when the tree is absent). Nothing is stubbed: the run stops at the first function no lane has implemented.
use amiga68k_engine::Amiga;
use guestui_abi::RecordingGuestUi;

const DOPUS: &str = "/data/build/corpus-bin/dopus5/DOpus5";

fn census(prog: &str, progdir: &str) -> Option<(Vec<String>, Vec<String>)> {
    let file = std::fs::read(format!("{DOPUS}/{prog}")).ok().or_else(|| { eprintln!("skipped {prog}: corpus not present"); None })?;
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.fs.vol.add_device("DOpus5", false);
    load(&mut a, "DOpus5:", std::path::Path::new(DOPUS));
    a.fs.vol.assign_add("LIBS", "DOpus5:Libs");
    a.fs.vol.assign("PROGDIR", progdir);
    a.stop_on_unimplemented = true;
    let _ = a.run_hunk(&file, 20_000_000).unwrap();
    let unimpl = a.refused.iter().filter(|r| r.ends_with(amiga68k_engine::libtable::UNIMPLEMENTED)).cloned().collect();
    Some((a.refused.clone(), unimpl))
}

fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path()); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

#[test]
fn viewfont_and_directory_opus_run_until_the_next_function_nobody_has_written() {
    // The first unimplemented function each real binary reaches (None = it runs on without one: Directory Opus reaches its idle
    // loop, which tests/dopus_idle.rs checks in detail).
    for (prog, progdir, next) in [("C/viewfont", "DOpus5:C", Some("intuition.library/QueryOverscan")), ("DirectoryOpus", "DOpus5:", None)] {
        let Some((_, unimpl)) = census(prog, progdir) else { return };
        let want = next.map(|n| format!("{n}: unimplemented"));
        assert_eq!(unimpl.last(), want.as_ref(), "{prog}: all of {unimpl:?}");
        assert!(unimpl.iter().all(|u| !u.starts_with("icon.library") && !u.starts_with("lowlevel.library")), "{prog}: {unimpl:?}");
    }
}
