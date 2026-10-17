//! Tiny host runner: `amiga68k-run [--census] --fixture | <hunk-file>`. Runs the program against a
//! RecordingGuestUi and prints the bridged-call log and the recorded UI calls. `--census` stops the guest at the first
//! call to an unimplemented library function and prints it as `NEXT-NEEDED library/function`: run a real binary, bridge
//! what it names, run it again.
use amiga68k_engine::{fixture, Amiga};
use guestui_abi::RecordingGuestUi;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let census = args.first().map(|a| a == "--census").unwrap_or(false);
    if census { args.remove(0); }
    let arg = args.first().cloned().unwrap_or_else(|| { eprintln!("usage: amiga68k-run [--census] --fixture | <hunk-file>"); std::process::exit(2) });
    let file = if arg == "--fixture" { fixture::build_hunk(None) } else {
        std::fs::read(&arg).unwrap_or_else(|e| { eprintln!("{arg}: {e}"); std::process::exit(1) })
    };
    let mut a = Amiga::new(RecordingGuestUi::new()).expect("machine");
    a.stop_on_unimplemented = census;
    match a.run_hunk(&file, 1_000_000) {
        Ok(r) => println!("ran {} instructions, halted={}, bus_faults={}", r.stats.insns, r.stats.halted, r.stats.bus_faults),
        Err(e) => { eprintln!("load failed: {e}"); std::process::exit(1) }
    }
    for l in &a.log { println!("{l}"); }
    for c in &a.ui.calls { println!("ui: {c:?}"); }
    if census {
        match a.refused.iter().rev().find(|r| r.ends_with(amiga68k_engine::libtable::UNIMPLEMENTED)) {
            Some(r) => println!("NEXT-NEEDED {}", r.trim_end_matches(&format!(": {}", amiga68k_engine::libtable::UNIMPLEMENTED))),
            None => println!("NEXT-NEEDED none (no unimplemented call reached)"),
        }
    }
    if !a.refused.is_empty() { std::process::exit(3) }
}
