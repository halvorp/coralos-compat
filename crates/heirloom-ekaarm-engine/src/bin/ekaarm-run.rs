use std::collections::BTreeSet;
use heirloom_ekaarm_engine::{layout, loader, Outcome};
use heirloom_fmt_e32::{E32Image, OrdinalTable};
use heirloom_ui_abi::RecordingGuestUi;

use heirloom_ekaarm_engine::hostdir;

const ORD_TSV: &str = include_str!("../../../heirloom-fmt-e32/data/eka-ordinals.tsv");

fn usage() -> ! {
    eprintln!("usage: ekaarm-run [--census] [--imports] [--max-insns N] [--drive DIR] <e32 file>   (--census: unimplemented calls return 0 and are listed in order)");
    std::process::exit(2)
}

fn main() {
    let mut max_insns = 50_000_000u64;
    let mut census = false;
    let mut list_imports = false;
    let mut file = None;
    let mut drive_dir: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--census" => census = true,
            "--imports" => list_imports = true,
            "--drive" => drive_dir = Some(args.next().unwrap_or_else(|| usage())),
            "--max-insns" => {
                let value = args.next().unwrap_or_else(|| usage());
                max_insns = value.parse().unwrap_or_else(|_| usage());
            }
            _ if arg.starts_with("--") => usage(),
            _ => {
                if file.replace(arg).is_some() {
                    usage();
                }
            }
        }
    }
    let file = file.unwrap_or_else(|| usage());
    let bytes = std::fs::read(&file).expect("read file");
    let table = OrdinalTable::parse(ORD_TSV).expect("parse ordinals");
    let image = E32Image::parse(&bytes).expect("parse layout");
    let code_len = image.code.len() as u32;
    let data_len = image.data.len() as u32;
    let bss_len = image.header.bss_size;
    let stack_sz = image.header.stack_size;
    let heap_max = image.header.heap_max;

    let mut proc = match loader::load(&bytes, &table) {
        Ok(p) => p,
        Err(e) => { println!("OUTCOME LoadRefused {e:?}"); std::process::exit(1) }
    };
    let stub_size = proc.stubs.len() as u32 * 8;
    println!("code:  base=0x{:08X} size=0x{:08X} prot=RX", layout::CODE_BASE, layout::pru(code_len));
    println!("data:  base=0x{:08X} size=0x{:08X} prot=RW", layout::DATA_BASE, layout::pru(data_len + bss_len));
    println!("heap:  base=0x{:08X} size=0x{:08X} prot=RW", layout::HEAP_BASE, layout::pru(heap_max));
    println!("stack: base=0x{:08X} size=0x{:08X} prot=RW", layout::STACK_TOP - layout::pru(stack_sz), layout::pru(stack_sz));
    println!("stubs: base=0x{:08X} size=0x{:08X} prot=RX", layout::STUB_BASE, layout::pru(stub_size));
    println!("exit:  base=0x{:08X} size=0x{:08X} prot=RX", layout::EXIT_STUB, layout::PAGE);

    let dlls = proc.stubs.iter().map(|s| s.dll_base.as_str()).collect::<BTreeSet<_>>().len();
    let unnamed = proc.stubs.iter().filter(|s| s.name.is_none()).count();
    println!("imports: {} dlls, {} stubs ({} unnamed)", dlls, proc.stubs.len(), unnamed);

    if list_imports {
        for s in &proc.stubs { println!("import {}/{} {}", s.dll_base, s.ordinal, s.name.as_deref().unwrap_or("?")); }
    }
    proc.set_ui(RecordingGuestUi::new());
    if let Some(dir) = &drive_dir {
        match hostdir::load_dir(std::path::Path::new(dir)) {
            Ok((drive, files)) => { println!("drive: {dir} ({files} files)"); proc.set_drive(drive); }
            Err(e) => { println!("OUTCOME DriveRefused {dir}: {e}"); std::process::exit(1) }
        }
    }
    if census { proc.census = Some(Vec::new()); }
    let outcome = proc.run(max_insns);
    if let Some(seen) = &proc.census {
        let mut first = std::collections::BTreeMap::new();
        for (i, m) in seen.iter().enumerate() { first.entry(m.as_str()).or_insert(i); }
        println!("census: {} unimplemented calls, {} distinct (in first-call order):", seen.len(), first.len());
        let mut order: Vec<_> = first.into_iter().collect();
        order.sort_by_key(|&(_, i)| i);
        for (m, _) in order { println!("  {m}"); }
    }
    for line in &proc.log { println!("{line}"); }
    if let Some(ui) = proc.ui_as::<RecordingGuestUi>() {
        println!("ui: {} recorded call(s)", ui.calls.len());
        for c in &ui.calls { println!("  {c:?}"); }
    }
    match outcome {
        Outcome::Exited(code) => println!("OUTCOME Exited({}) after {} instructions", code, proc.insns),
        Outcome::Refused(msg) => {
            println!("OUTCOME Refused after {} instructions", proc.insns);
            println!("{}", msg);
        }
        Outcome::Fault(msg) => {
            println!("OUTCOME Fault after {} instructions", proc.insns);
            println!("{}", msg);
        }
        Outcome::BudgetSpent => println!("OUTCOME BudgetSpent after {} instructions", proc.insns),
        Outcome::Blocked => println!("OUTCOME Blocked after {} instructions", proc.insns),
    }
    std::process::exit(0)
}
