//! Adobe Photoshop 7.0 Tryout started from its INSTALLED tree (the host Installer run's export, tests/ps7_installer.rs PS7_EXPORT_DIR; corpus
//! test input, never committed: /data/build/corpus-bin/mac/photoshop7-tryout/installed) the way Heirloom runs an application: the export is the
//! boot volume "Macintosh HD" (data forks as streams read on demand, resource forks from the `<name>.rsrc` sidecars, Finder info from
//! manifest.tsv), and the program is Contents:MacOSClassic:Adobe Photoshop 7.0 launched from its own folder (the CFM finds its libraries there).
//! A missing corpus SKIPS loudly. Diagnostic, not a gate:
//! `PS7_INSTALLED=... PS7_SLICES=500 cargo test --release --features fixtures --test ps7_launch -- --ignored --nocapture`.
use heirloom_common::{GuestMachine, Step};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

mod ps7_common;
mod wdef_bindings;

fn installed() -> String { std::env::var("PS7_INSTALLED").unwrap_or_else(|_| String::from("/data/build/corpus-bin/mac/photoshop7-tryout/installed")) }
/// The application inside the package, relative to the export (`PS7_APP` runs another one of the tree: VSetupT, the tryout set-up tool the
/// installer asks LaunchApplication for).
const PHOTOSHOP: &str = "Applications (Mac OS 9)/Adobe Photoshop 7/Adobe Photoshop 7.0/Contents/MacOSClassic/Adobe Photoshop 7.0";
fn app_path() -> String { std::env::var("PS7_APP").unwrap_or_else(|_| String::from(PHOTOSHOP)) }

/// A plain host file per stream (the export's data forks): the locator is its path.
struct Files;
impl heirloom_volume::StreamSource for Files {
    fn read_at(&self, locator: &str, offset: u64, dst: &mut [u8]) -> Option<usize> {
        let f = std::fs::File::open(locator).ok()?;
        std::os::unix::fs::FileExt::read_at(&f, dst, offset).ok()
    }
}

fn fourcc(s: &str) -> [u8; 4] { let mut b = [b' '; 4]; for (i, c) in s.bytes().take(4).enumerate() { b[i] = c; } b }

fn started() -> Option<MacPpc<RecordingGuestUi>> {
    let root = std::path::PathBuf::from(installed());
    let manifest = std::fs::read_to_string(root.join("manifest.tsv")).ok().or_else(|| { eprintln!("SKIPPED: no installed tree at {}", root.display()); None })?;
    let rel = app_path();
    let (data, rsrc) = (std::fs::read(root.join(&rel)).ok()?, std::fs::read(root.join(format!("{rel}.rsrc"))).ok()?);
    let leaf = rel.rsplit('/').next().unwrap().to_string();
    let row = manifest.lines().find(|l| l.split('\t').next() == Some(rel.as_str())).map(|l| l.split('\t').map(String::from).collect::<Vec<_>>()).unwrap_or_default();
    let creator = row.get(2).map_or(*b"????", |c| fourcc(c));
    let app = heirloom_fmt_resfork::writer::macbinary(&leaf, b"APPL", &creator, &data, &rsrc);
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), &app).unwrap();
    // the compat repo's window-definition bindings (DATA, bindings/macppc/*.sexp), as the host hands them to the engine
    for b in wdef_bindings::load("macppc") { eprintln!("BINDING {}", b.id); m.add_window_def_binding(b); }
    let vol = GuestMachine::volume(&mut m);
    vol.enable_streams();
    vol.set_stream_source(Box::new(Files));
    let hd = vol.volume("Macintosh HD").expect("the boot volume");
    vol.set_capacity(hd, 8 << 30).unwrap();
    let mut n = 0;
    for line in manifest.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 6 || f[0] == rel { continue }
        let (path, ty, cr) = (f[0], f[1], f[2]);
        let flags = u16::from_str_radix(f[3].trim_start_matches("0x"), 16).unwrap_or(0);
        let len: u64 = f[4].parse().unwrap_or(0);
        let parts: Vec<&str> = path.split('/').collect();
        let (dirs, leaf) = parts.split_at(parts.len() - 1);
        // a host ':' in a component is the Mac name's '/'
        let dirs: Vec<String> = dirs.iter().map(|d| d.replace(':', "/")).collect();
        let leaf = leaf[0].replace(':', "/");
        let dir = vol.mkdirs(hd, dirs.iter().map(String::as_str)).unwrap();
        let host = root.join(path);
        let Ok(id) = vol.put_stream(dir, &leaf, len, host.to_str().unwrap()) else { eprintln!("NOT STAGED (exists): {path}"); continue };
        let node = vol.get_mut(id);
        node.finfo.file_type = fourcc(ty);
        node.finfo.creator = fourcc(cr);
        node.finfo.flags = flags;
        if f[5] != "0" { node.rsrc = std::fs::read(format!("{}.rsrc", host.display())).unwrap_or_default(); }
        n += 1;
    }
    eprintln!("staged {n} files of the installed tree on Macintosh HD");
    let home: Vec<String> = std::iter::once(String::from("Macintosh HD")).chain(rel.split('/').map(|c| c.replace(':', "/"))).collect();
    let home: Vec<&str> = home[..home.len() - 1].iter().map(String::as_str).collect();
    // the program's own folder exists even when the program is its only file (start_file installs the program there)
    let vol = GuestMachine::volume(&mut m);
    let hd = vol.volume("Macintosh HD").unwrap();
    vol.mkdirs(hd, home[1..].iter().copied()).unwrap();
    GuestMachine::set_home(&mut m, &home).unwrap();
    GuestMachine::ui_mut(&mut m).auto_cancel_requests = false;
    GuestMachine::start_file(&mut m, &app, &leaf).unwrap();
    if let Some(n) = std::env::var("PS7_TRACE").ok().and_then(|v| v.parse().ok()) { m.trace_pcs(n); }
    Some(m)
}

fn drain(m: &mut MacPpc<RecordingGuestUi>) {
    let lines = core::mem::take(&mut m.mac.log);
    if let Ok(path) = std::env::var("PS7_LOG_FILE") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        for l in &lines { if !["PtInRect", "BlockMove", "null event", "GetMouse", "would idle"].iter().any(|n| l.contains(n)) { writeln!(f, "{l}").unwrap(); } }
    }
    m.mac.log = lines.into_iter().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
}

#[test]
#[ignore]
fn ps7_launch_host_run() {
    let Some(mut m) = started() else { return };
    // PS7_BINDCHECK=<file of names>: which CarbonLib imports this engine does not serve (a fragment's gap census), then stop
    if let Ok(f) = std::env::var("PS7_BINDCHECK") {
        for n in std::fs::read_to_string(f).unwrap().lines() {
            if let heirloom_macppc_engine::symbols::Binding::Unbound(why) = m.bind_from(&std::env::var("PS7_BINDLIB").unwrap_or_else(|_| String::from("CarbonLib")), n) { eprintln!("UNBOUND {n}: {}", why.chars().take(80).collect::<String>()); }
        }
        return;
    }
    // PS7_ACTIONS: drive it first with the shared verbs (ps7_common::act), then PS7_SLICES more slices
    let mut t = 0u64;
    if let Ok(actions) = std::env::var("PS7_ACTIONS") { let ok = ps7_common::act(&mut m, &mut t, &actions); eprintln!("ACTIONS DONE: {ok}"); }
    let slices: u32 = std::env::var("PS7_SLICES").ok().and_then(|s| s.parse().ok()).unwrap_or(500);
    let mut stop = None;
    // PS7_BUDGET: instructions per slice (5M); PS7_STOP_REFUSED: stop at the first slice that adds a refusal (with a small budget and
    // PS7_TRACE, the trace ends right at the refused call)
    let budget: u64 = std::env::var("PS7_BUDGET").ok().and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let stop_refused = std::env::var("PS7_STOP_REFUSED").is_ok();
    for i in 0..slices {
        drain(&mut m);
        let (st, _) = GuestMachine::step(&mut m, budget);
        if matches!(st, Step::Fault(_) | Step::Exited) { stop = Some((i, st)); break }
        if stop_refused && !m.mac.refused.is_empty() { stop = Some((i, st)); break }
        t += 20_000;
        GuestMachine::advance_clock_to(&mut m, t).unwrap();
    }
    drain(&mut m);
    eprintln!("STOPPED {stop:?} {:?}", m.fault());
    if let Some(c) = m.cpu() {
        eprintln!("PCS {:x?}", c.recent_pcs().collect::<Vec<u32>>());
        let (mut sp, mut chain) = (c.gpr[1], vec![c.pc, c.lr]);
        for _ in 0..24 {
            let Ok(next) = m.mac.mem.read_u32(sp) else { break };
            if next == 0 || next <= sp { break }
            chain.push(m.mac.mem.read_u32(next + 8).unwrap_or(0));
            sp = next;
        }
        eprintln!("BACKCHAIN {chain:x?}");
        if let Ok(tv) = std::env::var("PS7_TOC_OF") {
            // the TOC of a TVector, and which import a TOC offset holds: PS7_TOC_OF=<tvector hex>,<d>
            let mut it = tv.split(',');
            let tvec = u32::from_str_radix(it.next().unwrap().trim_start_matches("0x"), 16).unwrap();
            let d: i32 = it.next().unwrap().parse().unwrap();
            let toc = m.mac.mem.read_u32(tvec + 4).unwrap_or(0);
            let slot = toc.wrapping_add(d as u32);
            eprintln!("TOC {toc:#x} slot {slot:#x} -> {:?}", m.import_slots.get(&slot));
        }
        if let Ok(syms) = std::env::var("PS7_SLOTS") {
            for (slot, (lib, sym, weak)) in &m.import_slots { if syms.split(',').any(|w| w == sym) { eprintln!("SLOT {slot:#x} {lib}/{sym} weak={weak} value={:#x}", m.mac.mem.read_u32(*slot).unwrap_or(0)); } }
            let mut seen = std::collections::BTreeSet::new();
            for pc in c.recent_pcs() {
                let w = m.mac.mem.read_u32(pc).unwrap_or(0);
                if w >> 16 == 0x8182 && seen.insert(pc) { eprintln!("GLUESTUB {pc:#x} d={}", w as u16 as i16); }
            }
        }
        // PS7_DUMP=<addr hex>,<len>: guest bytes to run/dump.bin (disassemble with llvm-objdump --triple=powerpc, base = addr)
        if let Ok(d) = std::env::var("PS7_DUMP") {
            let (a, n) = d.split_once(',').unwrap();
            let (a, n) = (u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap(), n.parse::<u32>().unwrap());
            let b: Vec<u8> = (0..n).map(|i| m.mac.mem.read_u8(a + i).unwrap_or(0)).collect();
            std::fs::write("/build/patches/ps7/run/dump.bin", b).unwrap();
            eprintln!("DUMPED {a:#x} {n}; GPR {:x?} CTR {:#x} LR {:#x}", c.gpr, c.ctr, c.lr);
        }
        // a call through a glue whose TVector is empty: the TOC slot the caller's glue named (caller TOC at 20(r1), d from the glue at LR-4's target)
        if let Some(call) = m.mac.mem.read_u32(c.lr.wrapping_sub(4)).ok().filter(|w| w >> 26 == 18) {
            let li = ((call & 0x03FF_FFFC) as i32) << 6 >> 6;
            let glue = c.lr.wrapping_sub(4).wrapping_add(li as u32);
            let first = m.mac.mem.read_u32(glue).unwrap_or(0);
            if first >> 16 == 0x8182 {
                let toc = m.mac.mem.read_u32(c.gpr[1] + 20).unwrap_or(0);
                let slot = toc.wrapping_add(first as u16 as i16 as i32 as u32);
                let tv = m.mac.mem.read_u32(slot).unwrap_or(0);
                eprintln!("GLUESLOT glue {glue:#x} toc {toc:#x} slot {slot:#x} -> {:?} tvector {tv:#x} = [{:#x}, {:#x}] in {:?}", m.import_slots.get(&slot), m.mac.mem.read_u32(tv).unwrap_or(0), m.mac.mem.read_u32(tv + 4).unwrap_or(0), m.code_map.locate(tv));
                for (a, v) in m.import_slots.range(slot.saturating_sub(0x40)..slot + 0x40) { eprintln!("  NEAR {a:#x} {v:?}"); }
                // Photoshop's DRM-decrypted TOC copies its import words from the CFM-filled table (measured: TOC word 0x1000700 = the
                // import word at 0xf47c6a, the run's placement): the import this TOC word stands for
                let mirrored = slot.wrapping_add(0xf47c6a).wrapping_sub(0x0100_0700);
                eprintln!("  MIRROR {mirrored:#x} {:?}", m.import_slots.get(&mirrored));
                // the delta measured NOW: TOC words near the slot whose nonzero value equals exactly one import slot's value
                let mut deltas: std::collections::BTreeMap<u32, u32> = Default::default();
                for k in 0..64u32 {
                    let w = slot.wrapping_sub(128).wrapping_add(4 * k);
                    let v = m.mac.mem.read_u32(w).unwrap_or(0);
                    if v == 0 { continue }
                    let hits: Vec<u32> = m.import_slots.keys().copied().filter(|&a| m.mac.mem.read_u32(a).ok() == Some(v)).collect();
                    if hits.len() == 1 { *deltas.entry(w.wrapping_sub(hits[0])).or_default() += 1; }
                }
                if let Some((&d, &n)) = deltas.iter().max_by_key(|(_, &n)| n) {
                    let imp = slot.wrapping_sub(d);
                    eprintln!("  MIRROR-NOW delta {d:#x} ({n} matches) -> import {imp:#x} {:?}", m.import_slots.get(&imp));
                }
                let near: Vec<_> = m.import_slots.range(toc - 0x8000..toc + 0x8000).take(3).collect();
                let n = m.import_slots.range(toc - 0x8000..toc + 0x8000).count();
                eprintln!("  {n} import slots within the TOC's 64K, first {near:?}; slot in {:?}, toc in {:?}", m.code_map.locate_any(slot), m.code_map.locate_any(toc));
            }
        }
        // PS7_SLOTAT=<hex>,..: the import relocated into this guest word
        if let Ok(v) = std::env::var("PS7_SLOTAT") { for x in v.split(',') { let x = u32::from_str_radix(x, 16).unwrap(); eprintln!("SLOTAT {x:#x} {:?} value {:#x}", m.import_slots.get(&x), m.mac.mem.read_u32(x).unwrap_or(0)); } }
        // PS7_WHERE=<hex>,..: which fragment section / heap block holds an address
        if let Ok(v) = std::env::var("PS7_WHERE") { for x in v.split(',') { let x = u32::from_str_radix(x, 16).unwrap(); let h = m.mac.mm.recover_handle(x & !3); eprintln!("WHERE {x:#x} section {:?} handle-of-ptr {h:?} resinfo {:?}", m.code_map.locate_any(x), h.and_then(|h| m.mac.res_info(h))); } }
        // PS7_FIND=<hex>,<hex>..: the import slots that hold these values (which import a copied TOC word is)
        if let Ok(v) = std::env::var("PS7_FIND") {
            for x in v.split(',') {
                let x = u32::from_str_radix(x, 16).unwrap();
                for (a, s) in &m.import_slots { if m.mac.mem.read_u32(*a).unwrap_or(1) == x { eprintln!("FIND {x:#x} at {a:#x} {s:?} in {:?}", m.code_map.locate_any(*a)); } }
            }
        }
        eprintln!("CODEMAP {}", chain.iter().map(|&a| format!("{a:#x}={:?}", m.code_map.locate(a))).collect::<Vec<_>>().join(" "));
    }
    for l in &m.mac.log[m.mac.log.len().saturating_sub(40)..] { eprintln!("LOG {l}"); }
    let mut refused = m.mac.refused.clone();
    refused.sort();
    refused.dedup();
    eprintln!("REFUSED {refused:?}");
    for c in GuestMachine::ui(&m).calls.iter().rev().take(10) { eprintln!("UI {}", format!("{c:?}").chars().take(300).collect::<String>()); }
    for r in GuestMachine::ui(&m).requests.values() { eprintln!("REQUEST {}", format!("{r:?}").chars().take(400).collect::<String>()); }
    // the volume as the program left it (VSetupT's set-up output, for the next program's run)
    if let Ok(dir) = std::env::var("PS7_EXPORT_DIR") {
        let vol = GuestMachine::volume(&mut m);
        let hd = vol.volume("Macintosh HD").unwrap();
        ps7_common::export(vol, hd, std::path::Path::new(&dir));
    }
}
