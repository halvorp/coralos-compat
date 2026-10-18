//! Host runner: `macppc-run [--census] [--strict] [--data <data fork file>] <file>`.
//! `<file>` is `--fixture` (a built-in MacBinary application importing the classic startup calls), a bare PEF, or a classic Mac application (MacBinary, AppleSingle, or an AppleDouble header file whose data
//! fork is the sibling file with the `%` / `._` prefix removed, or `--data`). The PowerPC CPU is another lane's work, so this
//! instantiates the fragment (sections placed, imports bound by symbol name onto the mac68k Toolbox handlers,
//! relocations applied) and prints what it did. `--census` ends with the first NEXT-NEEDED import and exits 0; `--strict`
//! fails the instantiation on an unresolved strong import (exit 3).
use guestui_abi::RecordingGuestUi;
use macppc_engine::census::Census;
use macppc_engine::source::fragment_bytes;
use macppc_engine::MacPpc;
use retro_pef::Pef;
use std::path::{Path, PathBuf};

fn sibling_data_fork(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_str()?;
    let stripped = name.strip_prefix('%').or_else(|| name.strip_prefix("._"))?;
    let q = p.with_file_name(stripped);
    q.is_file().then_some(q)
}

fn usage() -> ! { eprintln!("usage: macppc-run [--census] [--strict] [--data <data fork file>] (--fixture | <file>)"); std::process::exit(2) }

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut census, mut strict, mut data) = (false, false, None);
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--census" => { census = true; args.remove(0); }
            "--strict" => { strict = true; args.remove(0); }
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
    match m.prepare_main(&frag) { Ok(s) => println!("  start: pc {:#x} r1 {:#x} r2 {:#x} lr {:#x} (the ExitToShell glue)", s.pc, s.r1, s.r2, s.lr), Err(e) => println!("  no start state: {e}") }
    let c = Census::of(&frag);
    for l in &c.lines { println!("{l}"); }
    println!("{}", c.summary());
    if census {
        match c.next_needed.first() { Some(n) => println!("NEXT-NEEDED {n}"), None => println!("NEXT-NEEDED none (every import is bound or weak-absent)") }
    }
}
