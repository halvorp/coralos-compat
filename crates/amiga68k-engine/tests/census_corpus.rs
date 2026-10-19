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

// ---- the 2026-10-18 corpus lane: more real binaries (docs/CORPUS.md). Absent binaries SKIP loudly; nothing is committed. ----

const ROOT: &str = "/data/build/corpus-bin";

/// What a census run of one real binary left behind.
struct Run { log: Vec<String>, refused: Vec<String>, ui: Vec<String>, insns: u64, halted: bool }

/// Run `rel` (under /data/build/corpus-bin) with `mounts` = (device, dir under the root) and SYS: loaded from `sys` if given.
fn run_app(rel: &str, sys: Option<&str>, mounts: &[(&str, &str)], progdir: Option<&str>, max: u64) -> Option<Run> {
    let file = std::fs::read(format!("{ROOT}/{rel}")).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {ROOT}/{rel} not present (see docs/CORPUS.md)"); None })?;
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    if let Some(s) = sys { load(&mut a, "SYS:", std::path::Path::new(&format!("{ROOT}/{s}"))); a.fs.vol.assign("LIBS", "SYS:Libs"); }
    for (dev, dir) in mounts {
        a.fs.vol.add_device(dev, false);
        load(&mut a, &format!("{dev}:"), std::path::Path::new(&format!("{ROOT}/{dir}")));
        if *dev == "MCC" { a.fs.vol.assign_add("LIBS", "MCC:Libs"); } // the MUI custom classes as LIBS:mui/*.mcc
    }
    if let Some(p) = progdir { a.fs.vol.assign("PROGDIR", p); }
    a.stop_on_unimplemented = true;
    let r = a.run_hunk(&file, max).unwrap();
    Some(Run { log: a.log.clone(), refused: a.refused.clone(), ui: a.ui.calls.iter().map(|c| format!("{c:?}")).collect(), insns: r.stats.insns, halted: r.stats.halted })
}

fn logged(r: &Run, needle: &str) -> bool { r.log.iter().any(|l| l.contains(needle)) }

#[test]
fn an_aros_built_binary_gets_through_task_resource_and_stdc_library_and_loads_its_mui_class_from_disk() {
    // aros-m68k "mini" = the nightly's Libs/Classes/Devs trimmed to what the engine does not bridge itself (CORPUS.md). Before
    // task.resource existed all 60 AROS binaries stopped inside stdc.library's init ("its init routine returned NULL").
    let Some(r) = run_app("aros-m68k/sys/Utilities/Clock", Some("aros-m68k/mini"), &[], None, 20_000_000) else { return };
    assert!(logged(&r, "OpenResource \"task.resource\" ->") && !logged(&r, "OpenResource \"task.resource\" -> NULL"), "{:#?}", r.log);
    assert!(logged(&r, "AllocTaskStorageSlot") && logged(&r, "SetTaskStorageSlot"), "stdc.library's init took a storage slot: {:#?}", r.log);
    // stdc.library's init needs timer.device/GetSysTime (implemented in this lane); Clock then builds its MUI application, loads its custom
    // class Clock.mcc from disk (docs/MUI-NATIVE.md: MCC_Query in the guest) and opens its window (drawn as a plane); utility.library's date
    // functions are written, and the next missing function is graphics.library/InitArea, named.
    assert!(logged(&r, "disk class \"Clock.mcc\" ="), "{:#?}", r.log);
    assert!(r.ui.iter().any(|c| c.starts_with("OpenWindow") && c.contains("title: \"Clock\"")), "{:?}", r.ui);
    assert!(r.refused.iter().all(|x| x == "graphics.library/InitArea: unimplemented"), "{:?}", r.refused);
    assert!(!r.refused.iter().any(|x| x.contains("GetSysTime")), "{:?}", r.refused);
    assert!(!r.refused.iter().any(|x| x.contains("init routine returned NULL")), "stdc.library initialised: {:?}", r.refused);
    // NEGATIVE CONTROL: without the AROS Libs the same binary cannot open stdc.library at all and says so.
    let Some(bare) = run_app("aros-m68k/sys/Utilities/Clock", None, &[], None, 20_000_000) else { return };
    assert!(!logged(&bare, "AllocTaskStorageSlot"), "{:#?}", bare.log);
}

#[test]
fn amimon_and_amimon_mui_open_their_windows_from_the_real_release_binaries() {
    let Some(gt) = run_app("amimcp/amimon.020", None, &[], None, 5_000_000) else { return };
    assert!(gt.ui.iter().any(|c| c.starts_with("OpenWindow")) && logged(&gt, "CreateGadgetA kind=13"), "a GadTools TEXT_KIND window: {:#?}", gt.log);
    let Some(mui) = run_app("amimcp/amimon-mui.020", None, &[], None, 5_000_000) else { return };
    assert!(mui.ui.iter().any(|c| c.starts_with("OpenWindow")) && logged(&mui, "MUI_NewObjectA \"Application.mui\""), "{:#?}", mui.log);
    assert!(gt.refused.is_empty() && mui.refused.is_empty(), "{:?} {:?}", gt.refused, mui.refused);
}

#[test]
fn yam_runs_its_startup_and_the_igame_mcc_load_reaches_custom_class_creation() {
    let Some(yam) = run_app("yam/ex/YAM 2.9p1/YAM", None, &[("YAM", "yam/ex/YAM 2.9p1")], Some("YAM:"), 20_000_000) else { return };
    assert!(yam.insns > 50_000, "YAM's startup ran {} instructions (halted {})", yam.insns, yam.halted);
    // iGame needs TextEditor.mcc as LIBS:mui/TextEditor.mcc; with it the run reaches MUI custom-class creation, without it
    // iGame prints "Can't open TextEditor.mcc" (the negative control).
    let Some(without) = run_app("igame/iGame/iGame.030", None, &[("PD", "igame/iGame")], Some("PD:"), 20_000_000) else { return };
    assert!(logged(&without, "Can't open TextEditor.mcc"), "{:#?}", without.log);
    let Some(with) = run_app("igame/iGame/iGame.030", None, &[("PD", "igame/iGame"), ("MCC", "mccs")], Some("PD:"), 20_000_000) else { return };
    assert!(!logged(&with, "Can't open TextEditor.mcc"), "{:#?}", with.log);
}

#[test]
fn the_aros_gadtools_demo_creates_its_real_button_gadget_before_the_next_missing_function() {
    // gtdemo is AROS-built (stdc + stdcio + posixc rellibs, task.resource, per-opener bases): it can only get this far because of
    // the engine work recorded in docs/CORPUS.md. The window opens after the gadgets; its debug output (exec RawPutChar, now the
    // engine log) was the first missing function until the amilibs lane, and the next gap is the GadTools PALETTE_KIND.
    let Some(r) = run_app("aros-m68k/sys/Demos/gtdemo", Some("aros-m68k/mini"), &[], None, 20_000_000) else { return };
    assert!(logged(&r, "CreateGadgetA kind=1 id=1") && logged(&r, "label=\"Exit (1)\""), "{:#?}", r.log);
    assert!(r.refused.iter().all(|x| !x.contains("RawPutChar")), "RawPutChar is bridged now: {:?}", r.refused);
    assert!(r.refused.iter().any(|x| x.contains("CreateGadgetA: kind 8 not bridged: PALETTE_KIND")), "{:?}", r.refused);
    // NEGATIVE CONTROL: no gadget is created before the libraries are initialised.
    let Some(bare) = run_app("aros-m68k/sys/Demos/gtdemo", None, &[], None, 20_000_000) else { return };
    assert!(!logged(&bare, "CreateGadgetA"), "{:#?}", bare.log);
}
