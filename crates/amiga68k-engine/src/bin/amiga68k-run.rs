//! Tiny host runner: `amiga68k-run [--census] --fixture | <hunk-file>`. Runs the program against a
//! RecordingGuestUi and prints the bridged-call log and the recorded UI calls. `--census` stops the guest at the first
//! call to an unimplemented library function and prints it as `NEXT-NEEDED library/function`: run a real binary, bridge
//! what it names, run it again.
//!
//! Volume options (repeatable, before the program): `--sys DIR` loads DIR into SYS:, `--mount NAME=DIR` makes a device
//! NAME: from DIR, `--assign NAME=PATH` / `--assign-add NAME=PATH` define or extend an assign (`LIBS=DOpus5:Libs`),
//! `--progdir PATH` points PROGDIR: at a guest path. `--stub lib/Func` (census only) makes an unimplemented function return a
//! zeroed block so the run continues past another lane's missing library (census.rs).
use amiga68k_engine::{fixture, Amiga};
#[path = "../volume_host.rs"]
mod volume_host;
use guestui_abi::RecordingGuestUi;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let census = args.first().map(|a| a == "--census").unwrap_or(false);
    if census { args.remove(0); }
    let mut a = Amiga::new(RecordingGuestUi::new()).expect("machine");
    let die = |m: String| -> ! { eprintln!("{m}"); std::process::exit(2) };
    let mut max_insns: u64 = 1_000_000;
    while args.first().map_or(false, |x| x.starts_with("--") && x != "--fixture") {
        let opt = args.remove(0);
        let val = if args.is_empty() { die(format!("{opt} needs a value")) } else { args.remove(0) };
        let pair = |v: &str| -> (String, String) { v.split_once('=').map(|(k, p)| (k.to_string(), p.to_string())).unwrap_or_else(|| die(format!("{opt} wants NAME=VALUE, got {v}"))) };
        match opt.as_str() {
            "--sys" => report(&val, volume_host::load_dir(&mut a.fs.vol, "SYS:", std::path::Path::new(&val))),
            "--mount" => { let (n, d) = pair(&val); a.fs.vol.add_device(&n, false); report(&d, volume_host::load_dir(&mut a.fs.vol, &format!("{n}:"), std::path::Path::new(&d))); }
            "--assign" => { let (n, p) = pair(&val); a.fs.vol.assign(&n, &p); }
            "--assign-add" => { let (n, p) = pair(&val); a.fs.vol.assign_add(&n, &p); }
            "--max-insns" => max_insns = val.replace('_', "").parse().unwrap_or_else(|_| die(format!("--max-insns wants a number, got {val}"))),
            "--progdir" => a.fs.vol.assign("PROGDIR", &val),
            "--stub" => { let (l, f) = val.split_once('/').unwrap_or_else(|| die(format!("--stub wants library/Function, got {val}"))); a.census_stubs.push((l.to_string(), f.to_string())); }
            _ => die(format!("unknown option {opt}")),
        }
    }
    let arg = args.first().cloned().unwrap_or_else(|| { eprintln!("usage: amiga68k-run [--census] --fixture | <hunk-file>"); std::process::exit(2) });
    let file = if arg == "--fixture" { fixture::build_hunk(None) } else {
        std::fs::read(&arg).unwrap_or_else(|e| { eprintln!("{arg}: {e}"); std::process::exit(1) })
    };
    a.stop_on_unimplemented = census;
    match a.run_hunk(&file, max_insns) {
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

fn report(what: &str, r: std::io::Result<volume_host::Loaded>) {
    match r {
        Ok(l) => { eprintln!("volume: {what}: {} files, {} bytes", l.files, l.bytes); for s in &l.skipped { eprintln!("volume: skipped {s}"); } }
        Err(e) => { eprintln!("volume: {what}: {e}"); std::process::exit(2) }
    }
}
