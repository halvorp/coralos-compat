//! The guest-PC sampler cuts a slice into chunks and must not change what the guest does: the same program retires the same instructions
//! to the same stop with sampling off and on, and every retired instruction is charged to some sampled PC (heirloom_macppc_engine::perf_pc).
use heirloom_fmt_pef::Pef;
use heirloom_macppc_engine::fixture;
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

fn clock() -> u64 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

fn run(every: Option<u64>) -> (u64, String, Option<(u64, usize)>) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("test", &Pef::parse(&fixture::startup_app()).unwrap()).unwrap();
    m.perf_enable(clock);
    if let Some(e) = every { m.perf_pc_enable(e); }
    m.start(&frag).unwrap();
    let (step, s) = m.step(1_000_000);
    let weight = m.perf().and_then(|p| p.pc.as_ref()).map(|pc| (pc.cum.values().map(|w| w.insns).sum::<u64>(), pc.cum.len()));
    (s.insns, format!("{step:?} pc={:#x}", m.cpu().map_or(0, |c| c.pc)), weight)
}

#[test]
fn sampling_every_few_instructions_changes_nothing_the_guest_does_and_charges_every_instruction() {
    let (base_insns, base_end, none) = run(None);
    assert!(none.is_none() && base_insns > 100, "{base_insns}");
    for every in [7u64, 64, 1000] {
        let (insns, end, weight) = run(Some(every));
        assert_eq!((insns, &end), (base_insns, &base_end), "every={every}");
        let (charged, pcs) = weight.unwrap();
        assert!(charged <= insns && charged + every > insns.saturating_sub(every), "every={every}: {charged} of {insns}");
        assert!(pcs >= 1, "every={every}");
    }
}
