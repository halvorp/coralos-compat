use std::collections::BTreeSet;
use heirloom_ekaarm_engine::{layout, loader, Outcome};
use heirloom_fmt_e32::{E32Image, OrdinalTable};
use heirloom_ui_abi::RecordingGuestUi;

use heirloom_ekaarm_engine::hostdir;

const ORD_TSV: &str = include_str!("../../../heirloom-fmt-e32/data/eka-ordinals.tsv");

fn usage() -> ! {
    eprintln!("usage: ekaarm-run [--census] [--imports] [--max-insns N] [--rom DIR] [--rom-manifest FILE] [--key SCAN]... [--trace] [--dump HEXADDR WORDS] [--dump-code FILE] [--drive DIR] [--cdl-data FILE] <e32 file>   (--census: unimplemented calls return 0 and are listed in order)");
    std::process::exit(2)
}

fn main() {
    let mut max_insns = 50_000_000u64;
    let mut census = false;
    let mut list_imports = false;
    let mut trace = false;
    let mut res_ids: Vec<u32> = Vec::new();
    let mut vtables: Vec<(u32, u32)> = Vec::new();
    let mut rom_dir: Option<String> = None;
    let mut rom_manifest: Option<String> = None;
    let mut keys: Vec<u32> = Vec::new();
    let mut rom_log = None;
    let mut dumps: Vec<(u32, u32)> = Vec::new();
    let mut dump_code: Option<String> = None;
    let mut file = None;
    let mut drive_dir: Option<String> = None;
    let mut cdl_data: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--census" => census = true,
            "--imports" => list_imports = true,
            "--trace" => trace = true,
            "--res" => res_ids.push(u32::from_str_radix(args.next().unwrap_or_else(|| usage()).trim_start_matches("0x"), 16).unwrap_or_else(|_| usage())),
            "--vtable" => {
                let a = args.next().unwrap_or_else(|| usage());
                let n = args.next().unwrap_or_else(|| usage());
                vtables.push((u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap_or_else(|_| usage()), n.parse().unwrap_or_else(|_| usage())));
            }
            // `--key SCAN`: once the app waits (Blocked), press and release the key with TStdScanCode SCAN (hex), run on, repeat per key.
            "--key" => keys.push(u32::from_str_radix(args.next().unwrap_or_else(|| usage()).trim_start_matches("0x"), 16).unwrap_or_else(|_| usage())),
            "--rom-manifest" => rom_manifest = Some(args.next().unwrap_or_else(|| usage())),
            "--rom" => rom_dir = Some(args.next().unwrap_or_else(|| usage())),
            "--dump" => {
                let a = args.next().unwrap_or_else(|| usage());
                let n = args.next().unwrap_or_else(|| usage());
                dumps.push((u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap_or_else(|_| usage()), n.parse().unwrap_or_else(|_| usage())));
            }
            "--dump-code" => dump_code = Some(args.next().unwrap_or_else(|| usage())),
            "--drive" => drive_dir = Some(args.next().unwrap_or_else(|| usage())),
            "--cdl-data" => cdl_data = Some(args.next().unwrap_or_else(|| usage())),
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
    if let Some(path) = &dump_code { std::fs::write(path, &image.code).expect("write code dump"); }
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
    if let Some(dir) = &rom_dir {
        match heirloom_ekaarm_engine::hostdir::DirRom::new(std::path::Path::new(dir)) {
            Ok(rom) => match proc.host.drive.as_mut() { Some(d) => { rom_log = Some(rom.access_log()); d.set_rom(Box::new(rom)) } None => { println!("OUTCOME RomRefused --rom needs --drive"); std::process::exit(1) } },
            Err(e) => { println!("OUTCOME RomRefused {dir}: {e}"); std::process::exit(1) }
        }
    }
    if let Some(path) = &cdl_data {
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| proc.set_cdl_data(&b)) {
            Ok(pack) => println!("cdl data: {path} (pack {pack})"),
            Err(e) => { println!("OUTCOME CdlDataRefused {path}: {e}"); std::process::exit(1) }
        }
    }
    if census { proc.census = Some(Vec::new()); }
    if trace { proc.trace = Some(Vec::new()); }
    let mut outcome = proc.run(max_insns);
    for scan in &keys {
        if !matches!(outcome, Outcome::Blocked) { break; }
        for down in [true, false] {
            if let Err(m) = proc.deliver_key(*scan, "", heirloom_ui_abi::Modifiers::default(), down) { println!("OUTCOME KeyRefused {scan:#x}: {m}"); std::process::exit(1) }
            outcome = proc.run(proc.insns.saturating_add(max_insns));
            if !matches!(outcome, Outcome::Blocked) { break; }
        }
    }
    if let Some(seen) = &proc.census {
        let mut first = std::collections::BTreeMap::new();
        for (i, m) in seen.iter().enumerate() { first.entry(m.as_str()).or_insert(i); }
        println!("census: {} unimplemented calls, {} distinct (in first-call order):", seen.len(), first.len());
        let mut order: Vec<_> = first.into_iter().collect();
        order.sort_by_key(|&(_, i)| i);
        for (m, _) in order { println!("  {m}"); }
    }
    for line in &proc.log { println!("{line}"); }
    if let Some(t) = &proc.trace { for line in t { println!("trace {line}"); } }
    if let Some(ui) = proc.ui_as::<RecordingGuestUi>() {
        println!("ui: {} recorded call(s)", ui.calls.len());
        for c in &ui.calls { println!("  {c:?}"); }
    }
    for id in &res_ids {
        match proc.resource(*id) {
            Ok(b) => println!("res {id:#x} ({} bytes): {}", b.len(), b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")),
            Err(e) => println!("res {id:#x}: {e}"),
        }
    }
    for (a, n) in &vtables { println!("{}", proc.describe_vtable(*a, *n)); }
    for (a, n) in &dumps {
        for i in 0..*n { let at = a + i * 4; match heirloom_cpu_arm::Memory::read_u32(&mut proc.mem, at) { Ok(v) => println!("mem {at:#010x}: {v:#010x}"), Err(_) => println!("mem {at:#010x}: <unreadable>") } }
    }
    println!("cpu: pc={:#x} lr={:#x} sp={:#x} r0-r3={:#x} {:#x} {:#x} {:#x} (code base {:#x}: caller = lr - base is the offset into --dump-code)", proc.cpu.pc(), proc.cpu.regs[14], proc.cpu.regs[13], proc.cpu.regs[0], proc.cpu.regs[1], proc.cpu.regs[2], proc.cpu.regs[3], layout::CODE_BASE);
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
    if let Some(path) = &rom_manifest {
        let Some(log) = &rom_log else { println!("OUTCOME RomManifestRefused --rom-manifest needs --rom"); std::process::exit(1) };
        let mut text = log.paths().join("\n");
        text.push('\n');
        if let Err(e) = std::fs::write(path, text) { println!("OUTCOME RomManifestRefused {path}: {e}"); std::process::exit(1) }
    }
    std::process::exit(0)
}
