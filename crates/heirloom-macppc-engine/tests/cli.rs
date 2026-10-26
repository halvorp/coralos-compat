//! The `macppc-run` binary end to end on files: an AppleDouble header file whose data fork is found as its sibling.
use heirloom_macppc_engine::fixture::{appledouble_app, startup_app};
use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_macppc-run")).args(args).output().unwrap();
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

#[test]
fn census_of_an_appledouble_application_finds_the_sibling_data_fork() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("macppc-cli");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("%Startup"), appledouble_app()).unwrap();
    std::fs::write(dir.join("Startup"), startup_app()).unwrap();
    let (ok, out, err) = run(&["--census", dir.join("%Startup").to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("21 imports: 21 bound, 0 NEXT-NEEDED, 0 weak-absent") && out.contains("NEXT-NEEDED none"), "{out}");
    // negative control: without the data fork file the run says so and fails
    std::fs::remove_file(dir.join("Startup")).unwrap();
    let (ok, _, err) = run(&["--census", dir.join("%Startup").to_str().unwrap()]);
    assert!(!ok && err.contains("pass the data fork file"), "{err}");
    // the built-in fixture works with no files, and --strict turns the NEXT-NEEDED imports into a load failure
    let (ok, out, _) = run(&["--census", "--fixture"]);
    assert!(ok && out.contains("21 bound"), "{out}");
    // --strict passes the fixture (every import is bound) and fails a fragment importing a call nothing serves
    let (ok, _, err) = run(&["--strict", "--fixture"]);
    assert!(ok, "{err}");
    let bogus = heirloom_macppc_engine::fixture::app(&[("InterfaceLib", false, &[("PlayNoteForever", heirloom_fmt_pef::loader::SymbolClass::TVector, false)])]);
    std::fs::write(dir.join("bogus.pef"), bogus).unwrap();
    let (ok, out, err) = run(&["--census", dir.join("bogus.pef").to_str().unwrap()]);
    assert!(ok && out.contains("NEXT-NEEDED InterfaceLib/PlayNoteForever"), "{out}{err}");
    let (ok, _, err) = run(&["--strict", dir.join("bogus.pef").to_str().unwrap()]);
    assert!(!ok && err.contains("unresolved strong import InterfaceLib/PlayNoteForever"), "{err}");
}
