//! Host runner: `macppc-run [--census] [--run] [--insns N] [--strict] [--rsrc <resource fork file>] [--data <data fork file>] <file>`.
//! `<file>` is `--fixture` (a built-in MacBinary application importing the classic startup calls), a bare PEF, or a classic Mac application (MacBinary, AppleSingle, or an AppleDouble header file whose data
//! fork is the sibling file with the `%` / `._` prefix removed, or `--data`). It instantiates the fragment (sections placed,
//! imports bound by symbol name onto the mac68k Toolbox handlers, relocations applied) and prints what it did. `--run`
//! then EXECUTES it on the PowerPC interpreter against a recording UI (up to `--insns`, default 50 million) and prints the
//! Toolbox calls it made and why it stopped; `--census` does the same and ends with the first NEXT-NEEDED (the static
//! import census, or the call the run stopped at) and exits 0; `--strict` fails the instantiation on an unresolved strong
//! import (exit 3).
use guestui_abi::RecordingGuestUi;
use macppc_engine::census::Census;
use macppc_engine::source::fragment_bytes;
use macppc_engine::{MacPpc, Step};
use retro_pef::Pef;
use std::path::{Path, PathBuf};

fn sibling_data_fork(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_str()?;
    let stripped = name.strip_prefix('%').or_else(|| name.strip_prefix("._"))?;
    let q = p.with_file_name(stripped);
    q.is_file().then_some(q)
}

fn usage() -> ! { eprintln!("usage: macppc-run [--census] [--run] [--insns N] [--strict] [--data <data fork file>] (--fixture | <file>)"); std::process::exit(2) }

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut census, mut strict, mut data, mut run, mut insns, mut rsrc) = (false, false, None, false, 50_000_000u64, None);
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--census" => { census = true; args.remove(0); }
            "--strict" => { strict = true; args.remove(0); }
            "--rsrc" => { args.remove(0); if args.is_empty() { usage() } rsrc = Some(PathBuf::from(args.remove(0))); }
            "--run" => { run = true; args.remove(0); }
            "--insns" => { args.remove(0); if args.is_empty() { usage() } insns = args.remove(0).parse().unwrap_or_else(|_| usage()); }
            "--data" => { args.remove(0); if args.is_empty() { usage() } data = Some(PathBuf::from(args.remove(0))); }
            _ => break,
        }
    }
    let path = PathBuf::from(args.first().cloned().unwrap_or_else(|| usage()));
    let file = if path.as_os_str() == "--fixture" { macppc_engine::fixture::macbinary_app(&macppc_engine::fixture::startup_app()) } else {
        std::fs::read(&path).unwrap_or_else(|e| { eprintln!("{}: {e}", path.display()); std::process::exit(1) })
    };
    let data_bytes = data.or_else(|| sibling_data_fork(&path)).map(|p| std::fs::read(&p).unwrap_or_else(|e| { eprintln!("{}: {e}", p.display()); std::process::exit(1) }));
    let (name, bytes) = fragment_bytes(&file, data_bytes.as_deref()).unwrap_or_else(|e| { eprintln!("{}: {e}", path.display()); std::process::exit(1) });
    let pef = Pef::parse(&bytes).unwrap_or_else(|e| { eprintln!("{}: {e}", path.display()); std::process::exit(1) });
    let mut m = MacPpc::new(RecordingGuestUi::new()).expect("machine");
    // the application's resources: a separate raw fork (--rsrc), else the resource fork inside the application file
    let fork_file = rsrc.map(|p| std::fs::read(&p).unwrap_or_else(|e| { eprintln!("{}: {e}", p.display()); std::process::exit(1) }));
    match m.attach_resources(fork_file.as_deref().unwrap_or(&file)) {
        Ok(n) => println!("resources: {n} in the application's resource fork"),
        Err(e) => println!("resources: none attached ({e})"),
    }
    m.strict_imports = strict;
    let frag = match m.instantiate(&name, &pef) {
        Ok(f) => f,
        Err(e) => { eprintln!("instantiate failed: {e}"); std::process::exit(3) }
    };
    println!("fragment {:?}: {} sections, {} libraries, {} imports, {} exports", frag.name, pef.sections.len(), pef.loader().map(|l| l.libraries.len()).unwrap_or(0), frag.imports.len(), frag.exports.len());
    for (i, s) in pef.sections.iter().enumerate() {
        let nm = pef.section_name(i).ok().flatten().unwrap_or_default();
        match frag.section_bases[i] {
            Some(b) => println!("  section {i} {:?} {nm:?}: {} bytes at {b:#x}", s.kind, s.total_size),
            None => println!("  section {i} {:?} {nm:?}: stays in the container", s.kind),
        }
    }
    for (n, a) in &frag.exports { println!("  export {n} = {a:#x}"); }
    for (label, tv) in [("main", frag.main), ("init", frag.init), ("term", frag.term)] {
        if let Some(t) = tv { println!("  {label}: TVector {:#x} code {:#x} toc {:#x}", t.address, t.code, t.toc); }
    }
    let started = match m.start(&frag) {
        Ok(()) => { let c = m.cpu().expect("started"); println!("  start: pc {:#x} r1 {:#x} r2 {:#x} r12 {:#x} lr {:#x} (the ExitToShell glue)", c.pc, c.gpr[1], c.gpr[2], c.gpr[12], c.lr); true }
        Err(e) => { println!("  no start state: {e}"); false }
    };
    let c = Census::of(&frag);
    for l in &c.lines { println!("{l}"); }
    println!("{}", c.summary());
    let mut stopped_at = None;
    if (run || census) && started {
        let (step, n) = m.run_loop(insns);
        for l in &m.mac.log { println!("  | {l}"); }
        for l in &m.mac.refused { println!("  | REFUSED {l}"); }
        let why = match &step {
            Step::Exited => "the program exited (ExitToShell or returned from main)".to_string(),
            Step::Blocked => "the program is waiting for an event (it reached its event loop)".to_string(),
            Step::BudgetSpent => format!("the {insns}-instruction budget ran out"),
            Step::Fault(f) => { if let Some(n) = f.strip_prefix("NEXT-NEEDED ") { stopped_at = Some(n.to_string()); } f.clone() }
        };
        println!("RAN {n} PowerPC instructions: {why}");
    }
    if census {
        match (stopped_at, c.next_needed.first()) {
            (Some(n), _) => println!("NEXT-NEEDED {n}"),
            (None, Some(n)) => println!("NEXT-NEEDED {n}"),
            (None, None) => println!("NEXT-NEEDED none (every import is bound or weak-absent)"),
        }
    }
}
