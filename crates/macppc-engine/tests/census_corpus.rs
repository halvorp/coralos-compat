//! The corpus lane's PowerPC census on REAL Retro68-built applications (not committed: /data/build/corpus-bin/mac-retro68/ppc, built
//! by /data/build/retro68; docs/CORPUS.md). A missing binary SKIPS loudly. Each test pins how far the app gets TODAY.
use guestui_abi::RecordingGuestUi;
use macppc_engine::census::Census;
use macppc_engine::source::fragment_bytes;
use macppc_engine::MacPpc;
use retro_pef::Pef;

fn alloc_fault(f: &str) -> String { format!("FAULT {f}") }

fn run(name: &str) -> Option<(Census, Vec<String>, Vec<String>)> {
    let path = format!("/data/build/corpus-bin/mac-retro68/ppc/{name}.bin");
    let file = std::fs::read(&path).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); None })?;
    let (frag_name, bytes) = fragment_bytes(&file, None).unwrap();
    let pef = Pef::parse(&bytes).unwrap();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let _ = m.attach_resources(&file);
    let frag = m.instantiate(&frag_name, &pef).unwrap();
    m.start(&frag).unwrap();
    let c = Census::of(&frag);
    let _ = m.run_loop(20_000_000);
    // a run that ends in a FAULT (a handler that cannot serve a PowerPC caller) is a stop too: it is counted with the
    // refusals, so "nothing refused" can never hide a halted program
    let mut refused = m.mac.refused.clone();
    if let Some(f) = m.fault() { refused.push(alloc_fault(f)); }
    Some((c, m.mac.log.clone(), refused))
}

#[test]
fn retro68_ppc_samples_bind_their_interfacelib_imports_and_stop_at_the_first_routine_without_a_handler() {
    // 2026-10-20, Mixed Mode slices 1-2: NGetTrapAddress answers from the shared trap registry (tests/trap_address.rs),
    // InvertRect binds to `_InverRect`, and NewRoutineDescriptor builds a real descriptor (mac68k-engine
    // tests/routine_descriptors.rs). The three now run to their event loops (Dialog: slice 3 calls its user item through the UPP).
    for (app, bound, needed) in [("HelloWorld", "NGetTrapAddress", "GrowWindow"), ("Dialog", "NewRoutineDescriptor", "FlushEvents"), ("WDEFShell", "NewRoutineDescriptor", "OpenDeskAcc")] {
        let Some((c, log, refused)) = run(app) else { return };
        assert!(refused.is_empty(), "{app}: {refused:?}");
        assert!(!c.next_needed.iter().any(|n| n.ends_with(&format!("/{bound}"))), "{app}: {bound} is bound: {:?}", c.next_needed);
        assert!(c.next_needed.iter().any(|n| n.ends_with(&format!("/{needed}"))), "{app}: {needed} is unbound (named, not silent): {:?}", c.next_needed);
        let waits = if app == "Dialog" { "ModalDialog 0x245210 blocks" } else { "WaitNextEvent" };
        assert!(log.iter().any(|l| l.contains(waits)), "{app} reached its event loop: {log:#?}");
    }
    let Some((_, log, _)) = run("HelloWorld") else { return };
    assert!(log.iter().any(|l| l.contains("GetTrapAddress Toolbox 0xa860 -> 0x4180 (the trap's stub)")), "{log:#?}");
    // Dialog (slice 3): ModalDialog first calls the user item's procedure through its UPP, as PowerPC code; the procedure
    // draws the default button's outline (PenSize 3x3, FrameRoundRect) and returns; ModalDialog then waits for a hit
    let Some((_, log, _)) = run("Dialog") else { return };
    let at = |needle: &str| log.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle}: {log:#?}"));
    assert!(at("[mixedmode] NewRoutineDescriptor") < at("[mixedmode] user item 2 of 0x245210"));
    assert!(at("[mixedmode] user item 2 of 0x245210") < at("[quickdraw] FrameRoundRect") && at("[quickdraw] FrameRoundRect") < at("ModalDialog 0x245210 blocks"));
    // The Retro68 runtime's File Manager imports (FSRead, FSWrite, FSClose, GetFPos, SetFPos, FlushVol, GetVRefNum ...) are bound since `files.rs`.
    for f in ["FSRead", "FSWrite", "FSClose", "GetFPos", "SetFPos", "FlushVol", "GetVRefNum"] {
        let Some((c, _, _)) = run("HelloWorld") else { return };
        assert!(!c.next_needed.iter().any(|n| n.ends_with(&format!("/{f}"))), "{f} is bound: {:?}", c.next_needed);
    }
    // Raytracer polls Button / GetMouse: both are bound now (mouse.rs) and it no longer stops at an unbound import.
    let Some((c, _, refused)) = run("Raytracer") else { return };
    assert!(!c.next_needed.iter().any(|n| n.ends_with("/Button") || n.ends_with("/GetMouse")), "{:?}", c.next_needed);
    assert!(refused.is_empty(), "{refused:?}");
    // NEGATIVE CONTROL: a stop name an app never reaches is not claimed
    let Some((_, log, refused)) = run("Dialog") else { return };
    assert!(!refused.iter().any(|r| r.contains("NGetTrapAddress")) && !log.iter().any(|l| l.contains("NGetTrapAddress")));
}

#[test]
fn macintosh_garden_glider_pro_fat_app_binds_most_of_interfacelib_and_names_what_is_missing() {
    // Glider PRO 1.1.2 (fat: 68k CODE + a PEF 'cfrg'; Macintosh Garden CD image, owner-authorised local test use; docs/CORPUS.md).
    let path = "/data/build/corpus-bin/mac/glider-pro/fx/Glider PRO\u{2122}.bin";
    let Ok(file) = std::fs::read(path) else { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); return };
    let (frag_name, bytes) = fragment_bytes(&file, None).unwrap();
    let pef = Pef::parse(&bytes).unwrap();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let _ = m.attach_resources(&file);
    let frag = m.instantiate(&frag_name, &pef).unwrap();
    let c = Census::of(&frag);
    // Still unbound and NAMED: Window Manager calls without a handler, and OS traps whose C prototype is not in OS_SIGS yet.
    for needed in ["ZoomWindow", "GetIconSuite", "Gestalt", "SysEnvirons", "FlushEvents", "FSpOpenDF"] {
        assert!(c.next_needed.iter().any(|n| n.ends_with(&format!("/{needed}"))), "{needed} is unbound and named: {:?}", c.next_needed);
    }
    // Bound since the Mac 68k lane added the Scrap Manager, alerts, DragWindow, the resource-file calls and GetCursor to the shared
    // Toolbox core (155 unbound before, 145 now): the PowerPC half gets them with no PowerPC code.
    for bound in ["GetScrap", "PutScrap", "ZeroScrap", "GetCursor", "Alert", "DragWindow", "OpenResFile", "CloseResFile", "SetResLoad", "HiliteWindow"] {
        assert!(!c.next_needed.iter().any(|n| n.ends_with(&format!("/{bound}"))), "{bound} is bound: {:?}", c.next_needed);
    }
    assert!(c.next_needed.len() < 155, "fewer than the 155 unbound before: {}", c.next_needed.len());
    // NEGATIVE CONTROL: InitWindows is bound (it is not NEXT-NEEDED), so the list is not simply "everything".
    assert!(!c.next_needed.iter().any(|n| n.ends_with("/InitWindows")), "{:?}", c.next_needed);
}
