//! Host runner: `mac68k-run [--census] [--32bit] [--machine NAME] [--idle TICKS] [--pcs N] [--insns N] [--dump ADDR N] [--event EV]... (--fixture | <application file>)`.
//! Launches a classic Mac application (MacBinary, AppleDouble header file or raw resource fork) against a
//! RecordingGuestUi, runs it until it exits, blocks in an event call or spends its instruction budget, then prints the
//! bridged-call log and the recorded UI calls. `--census` stops at the first unimplemented trap and prints it as
//! `NEXT-NEEDED _TrapName ($Axxx)`: run a real application, bridge what it names, run it again.
//! `--event close` / `--event menu:ID:ITEM` queue a native close request / menu command (window 1) before running.
use heirloom_ui_abi::{CallbackToken, GuestUiEvent, RecordingGuestUi, UiObjectRef};
use heirloom_mac68k_engine::fixture::test_app_macbinary;
use heirloom_mac68k_engine::machine::UNIMPLEMENTED;
use heirloom_mac68k_engine::Mac;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut machine = None::<String>;
    let mut idle = 0u32;
    let (mut census, mut events, mut bit32, mut pcs, mut insns, mut dump) = (false, Vec::new(), false, 0usize, 5_000_000u64, None::<(u32, u32)>);
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--census" => { census = true; args.remove(0); }
            "--implemented" => { let m = Mac::new(RecordingGuestUi::new()).expect("machine"); for n in m.implemented() { println!("{n}"); } return; } // every registered trap name (corpus-traps-mac.py --rank input)
            "--idle" => { args.remove(0); if args.is_empty() { usage() } idle = args.remove(0).parse().unwrap_or_else(|_| usage()); } // virtual 60 Hz ticks the host lets pass while the program waits for an event (null events)
            "--32bit" => { bit32 = true; args.remove(0); } // 32-bit addressing: the top address byte is significant (default: 24-bit)
            "--insns" => { args.remove(0); if args.is_empty() { usage() } insns = args.remove(0).parse().unwrap_or_else(|_| usage()); } // instruction budget (default 5,000,000)
            "--dump" => { args.remove(0); if args.len() < 2 { usage() } let a = u32::from_str_radix(args.remove(0).trim_start_matches("0x"), 16).unwrap_or_else(|_| usage()); let n = args.remove(0).parse().unwrap_or_else(|_| usage()); dump = Some((a, n)); } // hex-dump N bytes at ADDR after the run
            "--pcs" => { args.remove(0); if args.is_empty() { usage() } pcs = args.remove(0).parse().unwrap_or_else(|_| usage()); } // print the last N instruction addresses
            "--machine" => { args.remove(0); if args.is_empty() { usage() } machine = Some(args.remove(0)); } // profile name: mac-ii (default), mac-plus, mac-se
            "--event" => { args.remove(0); if args.is_empty() { usage() } events.push(args.remove(0)); }
            _ => break,
        }
    }
    let arg = args.first().cloned().unwrap_or_else(|| usage());
    let file = if arg == "--fixture" { test_app_macbinary() } else {
        std::fs::read(&arg).unwrap_or_else(|e| { eprintln!("{arg}: {e}"); std::process::exit(1) })
    };
    let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
    m.ui.drain_audio = true; // nobody is listening: queued audio counts as played, so completion routines are not left waiting
    m.stop_on_unimplemented = census;
    if let Some(name) = &machine { if let Err(e) = m.set_machine(name) { eprintln!("{}", e.0); std::process::exit(2) } }
    if bit32 { m.set_address_mode(heirloom_mac68k_engine::addrmode::AddrMode::Bit32).expect("mode"); }
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
    let ran = (|| {
        let mut r = m.run_loop(insns)?;
        // The host's 60 Hz clock: while the guest waits for an event, one tick passes (and its null event is delivered), up to `idle` times.
        for _ in 0..idle {
            if !r.stats.blocked || r.stats.insns >= insns { break; }
            m.advance_ticks(1);
            let more = m.run_loop(insns - r.stats.insns)?;
            r = heirloom_mac68k_engine::run::RunReport { stats: heirloom_cpu_68k::RunStats { insns: r.stats.insns + more.stats.insns, bus_faults: r.stats.bus_faults + more.stats.bus_faults, ..more.stats } };
        }
        Ok::<_, String>(r)
    })();
    match ran {
        Ok(r) => println!("ran {} instructions, halted={}, blocked={}, bus_faults={}", r.stats.insns, r.stats.halted, r.stats.blocked, r.stats.bus_faults),
        Err(e) => { eprintln!("run failed: {e}"); std::process::exit(1) }
    }
    if pcs > 0 { let r = heirloom_cpu_68k::recent_pcs(); println!("recent pcs: {:x?}", &r[r.len().saturating_sub(pcs)..]); }
    if let Some((a, n)) = dump { if let Ok(sp) = m.mem.span(a, n) { if let Ok(b) = m.mem.bytes(sp) { println!("dump {a:#x}: {}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()); } } }
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

fn usage() -> ! { eprintln!("usage: mac68k-run [--census] [--32bit] [--machine NAME] [--idle TICKS] [--pcs N] [--insns N] [--dump ADDR N] [--event close|menu:ID:ITEM]... --fixture | <application file>"); std::process::exit(2) }
