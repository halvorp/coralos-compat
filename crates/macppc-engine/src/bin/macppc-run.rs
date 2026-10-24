//! Host runner: `macppc-run [--census] [--run] [--insns N] [--pcs N] [--libdir DIR] [--idle TICKS] [--tier T] [--prof] [--strict] [--dump ADDR:WORDS]... [--bind-list <exports file> [LIB]] [--rsrc <resource fork file>] [--data <data fork file>] <file>`.
//! `<file>` is `--fixture` (a built-in MacBinary application importing the classic startup calls), a bare PEF, or a classic Mac application (MacBinary, AppleSingle, or an AppleDouble header file whose data
//! fork is the sibling file with the `%` / `._` prefix removed, or `--data`). It instantiates the fragment (sections placed,
//! imports bound by symbol name onto the mac68k Toolbox handlers, relocations applied) and prints what it did. `--run`
//! then EXECUTES it on the PowerPC interpreter against a recording UI (up to `--insns`, default 50 million) and prints the
//! Toolbox calls it made and why it stopped; `--census` does the same and ends with the first NEXT-NEEDED (the static
//! import census, or the call the run stopped at) and exits 0; `--strict` fails the instantiation on an unresolved strong
//! import (exit 3). `--tier T` picks the PowerPC execution tier (retro_ppc::Tier::parse); `--prof` prints a
//! `[PPC-PROF insns= native_calls= elapsed_us= mips= tier=]` line for the run (elapsed = the run loop alone).
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

fn usage() -> ! { eprintln!("usage: macppc-run [--census] [--run] [--insns N] [--pcs N] [--libdir DIR] [--idle TICKS] [--tier T] [--prof] [--strict] [--data <data fork file>] (--fixture | <file>)"); std::process::exit(2) }

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dump: Vec<(u32, u32)> = Vec::new();
    let (mut census, mut strict, mut data, mut run, mut insns, mut rsrc, mut pcs, mut libdir, mut idle) = (false, false, None, false, 50_000_000u64, None, 0usize, None::<PathBuf>, 0u32);
    let (mut tier, mut prof) = (None::<retro_ppc::Tier>, false);
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--tier" => { args.remove(0); if args.is_empty() { usage() } tier = Some(retro_ppc::Tier::parse(&args.remove(0)).unwrap_or_else(|| usage())); }
            "--prof" => { prof = true; args.remove(0); }
            "--census" => { census = true; args.remove(0); }
            "--strict" => { strict = true; args.remove(0); }
            "--rsrc" => { args.remove(0); if args.is_empty() { usage() } rsrc = Some(PathBuf::from(args.remove(0))); }
            "--run" => { run = true; args.remove(0); }
            "--insns" => { args.remove(0); if args.is_empty() { usage() } insns = args.remove(0).parse().unwrap_or_else(|_| usage()); }
            "--pcs" => { args.remove(0); if args.is_empty() { usage() } pcs = args.remove(0).parse().unwrap_or_else(|_| usage()); }
            "--libdir" => { args.remove(0); if args.is_empty() { usage() } libdir = Some(PathBuf::from(args.remove(0))); }
            "--idle" => { args.remove(0); if args.is_empty() { usage() } idle = args.remove(0).parse().unwrap_or_else(|_| usage()); }
            "--bind-list" => {
                // `--bind-list <exports file> [LIBRARY]`: for every `class<TAB>name` line (the 9.2.1 oracle's export lists) print BOUND or UNBOUND as this engine would bind it for an import from LIBRARY (default CarbonLib)
                args.remove(0);
                if args.is_empty() { usage() }
                let list = std::fs::read_to_string(args.remove(0)).unwrap_or_else(|e| { eprintln!("--bind-list: {e}"); std::process::exit(1) });
                let lib = if args.first().is_some_and(|a| !a.starts_with("--")) { args.remove(0) } else { "CarbonLib".to_string() };
                let mut m = MacPpc::new(RecordingGuestUi::new()).expect("machine");
                for l in list.lines().filter(|l| !l.starts_with('#')) {
                    let Some((class, name)) = l.split_once('\t') else { continue };
                    let b = m.bind_from(&lib, name);
                    println!("{}\t{class}\t{name}", if matches!(b, macppc_engine::symbols::Binding::Unbound(_)) { "UNBOUND" } else { "BOUND" });
                }
                return;
            }
            "--dump" => {
                // `--dump ADDR:WORDS`: print that many big-endian words of guest memory after the run stops (hex ADDR), for reading a fault's neighbourhood
                args.remove(0);
                if args.is_empty() { usage() }
                let a = args.remove(0);
                let (at, n) = a.split_once(':').unwrap_or((&a, "16"));
                dump.push((u32::from_str_radix(at.trim_start_matches("0x"), 16).unwrap_or_else(|_| usage()), n.parse().unwrap_or_else(|_| usage())));
            }
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
    // a sibling `<file>:rsrc` (how 7z unpacks an HFS+ resource fork) is the resource fork unless --rsrc says otherwise
    let rsrc = rsrc.or_else(|| { let mut n = path.clone().into_os_string(); n.push(":rsrc"); let p = PathBuf::from(n); p.is_file().then_some(p) });
    let fork_file = rsrc.map(|p| std::fs::read(&p).unwrap_or_else(|e| { eprintln!("{}: {e}", p.display()); std::process::exit(1) }));
    match m.attach_resources(fork_file.as_deref().unwrap_or(&file)) {
        Ok(n) => println!("resources: {n} in the application's resource fork"),
        Err(e) => println!("resources: none attached ({e})"),
    }
    // private libraries the application carries in its own data fork (its cfrg import-library members), then every PEF file in --libdir (named by file name)
    for (lname, bytes) in macppc_engine::source::library_fragments(&file, data_bytes.as_deref()) { println!("library {lname:?}: {} bytes inside the application file", bytes.len()); m.provide_library(&lname, &bytes); }
    if let Some(dir) = &libdir {
        let mut names: Vec<_> = std::fs::read_dir(dir).unwrap_or_else(|e| { eprintln!("{}: {e}", dir.display()); std::process::exit(1) }).filter_map(|e| e.ok()).collect();
        names.sort_by_key(|e| e.file_name());
        for e in names {
            let Ok(bytes) = std::fs::read(e.path()) else { continue };
            if !Pef::has_magic(&bytes) || e.file_name().to_string_lossy().ends_with(":rsrc") { continue; }
            let lname = e.file_name().to_string_lossy().into_owned();
            println!("library {lname:?}: {} bytes from {}", bytes.len(), dir.display());
            m.provide_library(&lname, &bytes);
        }
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
    for l in c.library_table() { println!("{l}"); }
    let mut stopped_at = None;
    if (run || census) && started {
        m.trace_pcs(pcs);
        if let Some(t) = tier { m.exec().tier = t; }
        let t0 = std::time::Instant::now();
        let (mut step, mut n) = m.run_loop(insns);
        // --idle T: a program waiting for an event (or a timeout) is given T more ticks of virtual time and resumed, up to the instruction budget in total
        let mut rounds = 0u32;
        while idle > 0 && step == Step::Blocked && n < insns && rounds < 2_000_000 {
            m.mac.advance_ticks(idle);
            rounds += 1;
            let (st, k) = m.run_loop(insns - n);
            step = st; n += k;
        }
        let elapsed_us = t0.elapsed().as_micros() as u64;
        for l in &m.mac.log { println!("  | {l}"); }
        for l in &m.mac.refused { println!("  | REFUSED {l}"); }
        let why = match &step {
            Step::Exited => "the program exited (ExitToShell or returned from main)".to_string(),
            Step::Blocked => "the program is waiting for an event (it reached its event loop)".to_string(),
            Step::BudgetSpent => format!("the {insns}-instruction budget ran out"),
            Step::Fault(f) => { if let Some(n) = f.strip_prefix("NEXT-NEEDED ") { stopped_at = Some(n.to_string()); } f.clone() }
        };
        println!("RAN {n} PowerPC instructions: {why}");
        for (at, n) in &dump {
            print!("dump {at:#x}:");
            for i in 0..*n { print!(" {:08x}", m.mac.mem.read_u32(at + 4 * i).unwrap_or(0xDEAD_DEAD)); }
            println!();
        }
        if prof { let t = m.exec().tier; println!("{}", m.exec().prof.line(elapsed_us, t)); }
        if let Some(d) = m.exec().divergence() { println!("LOCKSTEP DIVERGENCE: tier 1 and tier 0 differ in the block at {:#x}: {}", d.block, d.what.join(", ")); }
        else if m.exec().tier == retro_ppc::Tier::Lockstep { println!("lockstep: no divergence ({:?})", m.exec().lockstep_stats()); }
        if pcs > 0 {
            if let Some(c) = m.cpu() {
                println!("recent pcs (oldest first): {}", c.recent_pcs().map(|p| format!("{p:#x}")).collect::<Vec<_>>().join(" "));
                println!("registers: pc {:#x} lr {:#x} ctr {:#x} cr {:#x} xer {:#x}", c.pc, c.lr, c.ctr, c.cr, c.xer);
                for r in 0..32 { print!("r{r}={:#x}{}", c.gpr[r], if r % 8 == 7 { "\n" } else { " " }); }
            }
        }
    }
    if census {
        match (stopped_at, c.next_needed.first()) {
            (Some(n), _) => println!("NEXT-NEEDED {n}"),
            (None, Some(n)) => println!("NEXT-NEEDED {n}"),
            (None, None) => println!("NEXT-NEEDED none (every import is bound or weak-absent)"),
        }
    }
}
