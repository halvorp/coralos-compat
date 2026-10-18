//! Host runner: `mac68k-run [--census] [--event EV]... (--fixture | <application file>)`.
//! Launches a classic Mac application (MacBinary, AppleDouble header file or raw resource fork) against a
//! RecordingGuestUi, runs it until it exits, blocks in an event call or spends its instruction budget, then prints the
//! bridged-call log and the recorded UI calls. `--census` stops at the first unimplemented trap and prints it as
//! `NEXT-NEEDED _TrapName ($Axxx)`: run a real application, bridge what it names, run it again.
//! `--event close` / `--event menu:ID:ITEM` queue a native close request / menu command (window 1) before running.
use guestui_abi::{CallbackToken, GuestUiEvent, RecordingGuestUi, UiObjectRef};
use mac68k_engine::fixture::test_app_macbinary;
use mac68k_engine::machine::UNIMPLEMENTED;
use mac68k_engine::Mac;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut census, mut events) = (false, Vec::new());
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--census" => { census = true; args.remove(0); }
            "--event" => { args.remove(0); if args.is_empty() { usage() } events.push(args.remove(0)); }
            _ => break,
        }
    }
    let arg = args.first().cloned().unwrap_or_else(|| usage());
    let file = if arg == "--fixture" { test_app_macbinary() } else {
        std::fs::read(&arg).unwrap_or_else(|e| { eprintln!("{arg}: {e}"); std::process::exit(1) })
    };
    let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
    m.stop_on_unimplemented = census;
    if let Err(e) = m.launch(&file) { eprintln!("launch failed: {e}"); std::process::exit(1) }
    let win = UiObjectRef { id: 1, generation: 1 };
    for e in &events {
        let parts: Vec<&str> = e.split(':').collect();
        let ev = match parts.as_slice() {
            ["close"] => GuestUiEvent::CloseRequested { win, token: CallbackToken(0) },
            ["menu", id, item] => GuestUiEvent::MenuCommand { win, token: CallbackToken(id.parse::<u64>().unwrap() << 16 | item.parse::<u64>().unwrap()) },
            _ => { eprintln!("unknown event {e}"); std::process::exit(2) }
        };
        m.ui.events.push_back(ev);
    }
    match m.run_loop(5_000_000) {
        Ok(r) => println!("ran {} instructions, halted={}, blocked={}, bus_faults={}", r.stats.insns, r.stats.halted, r.stats.blocked, r.stats.bus_faults),
        Err(e) => { eprintln!("run failed: {e}"); std::process::exit(1) }
    }
    for l in &m.log { println!("{l}"); }
    for c in &m.ui.calls { println!("ui: {c:?}"); }
    if census {
        match m.refused.iter().rev().find(|r| r.starts_with(UNIMPLEMENTED)) {
            Some(r) => println!("NEXT-NEEDED {}{}", r.trim_start_matches(UNIMPLEMENTED).trim(),
                m.unimplemented_at.last().map(|(_, pc)| format!(" at pc={pc:#x}")).unwrap_or_default()),
            None => println!("NEXT-NEEDED none (no unimplemented trap reached)"),
        }
    }
    for r in &m.refused { println!("refused: {r}"); }
    if !m.refused.is_empty() { std::process::exit(3) }
}

fn usage() -> ! { eprintln!("usage: mac68k-run [--census] [--event close|menu:ID:ITEM]... --fixture | <application file>"); std::process::exit(2) }
