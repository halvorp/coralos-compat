//! Tiny host runner: `amiga68k-run --fixture | <hunk-file>`. Runs the program against a
//! RecordingGuestUi and prints the bridged-call log and the recorded UI calls.
use amiga68k_engine::{fixture, Amiga};
use guestui_abi::RecordingGuestUi;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| { eprintln!("usage: amiga68k-run --fixture | <hunk-file>"); std::process::exit(2) });
    let file = if arg == "--fixture" { fixture::build_hunk(None) } else {
        std::fs::read(&arg).unwrap_or_else(|e| { eprintln!("{arg}: {e}"); std::process::exit(1) })
    };
    let mut a = Amiga::new(RecordingGuestUi::new()).expect("machine");
    match a.run_hunk(&file, 1_000_000) {
        Ok(r) => println!("ran {} instructions, halted={}, bus_faults={}", r.stats.insns, r.stats.halted, r.stats.bus_faults),
        Err(e) => { eprintln!("load failed: {e}"); std::process::exit(1) }
    }
    for l in &a.log { println!("{l}"); }
    for c in &a.ui.calls { println!("ui: {c:?}"); }
    if !a.refused.is_empty() { std::process::exit(3) }
}
