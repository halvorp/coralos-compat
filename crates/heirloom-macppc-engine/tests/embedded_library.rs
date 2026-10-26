//! An application's own cfrg library members are provided before it links: Warcraft III carries StormCarbon in its data
//! fork, and through the GuestMachine path (Heirloom.app) it stopped at "library StormCarbon is not provided by the engine"
//! (Z83, 2026-10-26) while macppc-run, which provided them, linked. `HEIRLOOM_WC3_APP` names the owner's MacBinary of the
//! application (never part of this repository); unset = skipped, said so.
use heirloom_common::GuestMachine;
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

#[test]
fn an_applications_embedded_library_is_provided_before_it_links() {
    let Ok(path) = std::env::var("HEIRLOOM_WC3_APP") else {
        eprintln!("SKIP: HEIRLOOM_WC3_APP is not set; no real application with an embedded library read");
        return;
    };
    let file = std::fs::read(&path).unwrap();
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), &file).unwrap();
    if let Err(why) = m.start_file(&file, "Warcraft III") {
        assert!(!why.0.contains("StormCarbon"), "the embedded library was not provided: {}", why.0);
        return;
    }
    // An unbound import refuses when it is CALLED, so run the guest: StormCarbon's own init is the first thing that needs it.
    let (step, _) = m.step(5_000_000);
    let seen = format!("{step:?}");
    assert!(!seen.contains("StormCarbon is not provided"), "the embedded library was not provided: {seen}");
}
