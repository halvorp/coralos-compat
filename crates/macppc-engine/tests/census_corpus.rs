//! The corpus lane's PowerPC census on REAL Retro68-built applications (not committed: /data/build/corpus-bin/mac-retro68/ppc, built
//! by /data/build/retro68; docs/CORPUS.md). A missing binary SKIPS loudly. Each test pins how far the app gets TODAY.
use guestui_abi::RecordingGuestUi;
use macppc_engine::census::Census;
use macppc_engine::source::fragment_bytes;
use macppc_engine::MacPpc;
use retro_pef::Pef;

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
    Some((c, m.mac.log.clone(), m.mac.refused.clone()))
}

#[test]
fn retro68_ppc_samples_bind_their_interfacelib_imports_and_stop_at_the_first_routine_without_a_handler() {
    // (sample, an import that IS bound, an import that is NEXT-NEEDED, the routine the run stops at)
    for (app, bound, needed, stop) in [("Dialog", "InitDialogs", "ModalDialog", "GetNewDialog"), ("Raytracer", "InitWindows", "Button", "Button"), ("HelloWorld", "InitWindows", "GetFPos", "NGetTrapAddress"), ("WDEFShell", "InitWindows", "OpenDeskAcc", "NewRoutineDescriptor")] {
        let Some((c, log, refused)) = run(app) else { return };
        assert!(!c.next_needed.iter().any(|n| n.ends_with(&format!("/{bound}"))), "{app}: {bound} is bound: {:?}", c.next_needed);
        assert!(c.next_needed.iter().any(|n| n.ends_with(&format!("/{needed}"))), "{app}: {needed} is unbound (named, not silent): {:?}", c.next_needed);
        assert!(refused.iter().any(|r| r.contains(stop)) || log.iter().any(|l| l.contains(stop)), "{app} stops at {stop}: {refused:?}\n{log:#?}");
    }
    // NEGATIVE CONTROL: a stop name an app never reaches is not claimed
    let Some((_, log, refused)) = run("Dialog") else { return };
    assert!(!refused.iter().any(|r| r.contains("NGetTrapAddress")) && !log.iter().any(|l| l.contains("NewRoutineDescriptor")));
}
