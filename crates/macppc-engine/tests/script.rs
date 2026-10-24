//! The Script Manager by C name over the 68k engine's Roman-only answers: one table for both CPUs, refusals by name for what has no honest answer.
use guestui_abi::RecordingGuestUi;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
fn run(m: &mut M, name: &str, args: &[u32]) -> GlueOutcome {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let o = m.glue_call(name, &mut regs);
    if o == GlueOutcome::Return { assert!(regs[3] <= u32::MAX); LAST.with(|l| l.set(regs[3])); }
    o
}
thread_local! { static LAST: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
fn call(m: &mut M, name: &str, args: &[u32]) -> u32 { assert_eq!(run(m, name, args), GlueOutcome::Return, "{name}: {:?}", m.mac.refused); LAST.with(|l| l.get()) }

#[test]
fn roman_only_answers_are_the_68k_ones_and_the_rest_is_refused_by_name() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "GetScriptManagerVariable", &[4]), 1, "smEnabled: one script system");
    assert_eq!(call(&mut m, "GetScriptManagerVariable", &[0]), 7, "smVersion");
    assert_eq!(call(&mut m, "GetScriptManagerVariable", &[6]), 0, "smBidirect");
    assert_eq!((call(&mut m, "FontScript", &[]), call(&mut m, "IntlScript", &[]), call(&mut m, "FontToScript", &[3])), (0, 0, 0));
    assert_eq!(call(&mut m, "KeyScript", &[0xFFFF]), 0);
    // setting a variable to the value it has is fine; to another value it is refused (one script system)
    assert_eq!(call(&mut m, "SetScriptManagerVariable", &[4, 1]), 0);
    // NEGATIVE CONTROLS: an unknown selector is not given a made-up number; changing the script system is refused; GetScriptVariable has no verified table
    let GlueOutcome::Refused(w) = run(&mut m, "GetScriptManagerVariable", &[77]) else { panic!() };
    assert!(w.contains("GetScriptManagerVariable(77)"), "{w}");
    let GlueOutcome::Refused(w) = run(&mut m, "SetScriptManagerVariable", &[4, 0]) else { panic!() };
    assert!(w.contains("one script system"), "{w}");
    let GlueOutcome::Refused(w) = run(&mut m, "GetScriptVariable", &[0, 2]) else { panic!() };
    assert!(w.contains("GetScriptVariable"), "{w}");
}
