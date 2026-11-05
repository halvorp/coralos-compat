//! The Adobe Photoshop 7.0 Tryout installer (a MindVision Installer VISE 7.4 application, type APPL creator VIS3; corpus test input,
//! never committed: /data/build/corpus-bin/mac/photoshop7-tryout, docs/CORPUS.md) run on the host the way Heirloom runs it: the
//! installer sits on a read-only volume "MacFormat 123" in the folder "Photoshop 7" as it does on the disc, and "Macintosh HD" (the
//! boot volume) is the writable disk the install goes to. A missing corpus SKIPS loudly.
//!
//! `cargo test --release --features fixtures --test ps7_installer ps7_installer_host_run -- --ignored --nocapture` with `PS7_ACTIONS`
//! (the same verbs as the Warcraft III harness: `run:N`, `click:X,Y`, `button:LABEL`, `field:N=TEXT`, `controls:`, `shot:FILE.ppm`)
//! and `PS7_EXPORT_DIR` to write the installed tree out (data forks, `<name>.rsrc` sidecars, manifest.tsv).
use heirloom_common::{GuestMachine, Step};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

mod ps7_common;
use ps7_common::{act, export};

/// The installer as one MacBinary file: `HEIRLOOM_PS7_INSTALLER`, else the corpus lane's place.
fn installer() -> String {
    std::env::var("HEIRLOOM_PS7_INSTALLER").unwrap_or_else(|_| String::from("/data/build/corpus-bin/mac/photoshop7-tryout/Install Adobe Photoshop.bin"))
}

/// A machine with the disc folder ":Photoshop 7:" on a locked volume, the installer launched from it.
fn started() -> Option<MacPpc<RecordingGuestUi>> {
    let app = std::fs::read(installer()).ok().or_else(|| { eprintln!("SKIPPED: the Photoshop 7 Tryout installer corpus ({}) is absent", installer()); None })?;
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), &app).unwrap();
    let vol = GuestMachine::volume(&mut m);
    let cd = vol.add_volume("MacFormat 123", false); // a CD: locked
    vol.mkdirs(cd, ["Photoshop 7"].iter().copied()).unwrap();
    let hd = vol.volume("Macintosh HD").expect("the boot volume");
    vol.set_capacity(hd, 4 << 30).unwrap(); // a 4 GB disk: room for the ~250 MB install
    GuestMachine::set_home(&mut m, &["MacFormat 123", "Photoshop 7"]).unwrap();
    GuestMachine::ui_mut(&mut m).auto_cancel_requests = false;
    GuestMachine::start_file(&mut m, &app, "Install Adobe Photoshop").unwrap();
    if let Some(n) = std::env::var("PS7_TRACE").ok().and_then(|v| v.parse().ok()) { m.trace_pcs(n); }
    Some(m)
}

/// Diagnostic, not a gate: how far the installer gets, its last log lines, refusals, UI calls and what it wrote on Macintosh HD.
#[test]
#[ignore]
fn ps7_installer_host_run() {
    let Some(mut m) = started() else { return };
    let mut t = 0u64;
    let actions = std::env::var("PS7_ACTIONS").unwrap_or_else(|_| "run:200".into());
    let ok = act(&mut m, &mut t, &actions);
    eprintln!("ACTIONS DONE: {ok} {:?}", m.fault());
    // the guest's call chain where it stopped (PowerPC frames: [r1] = the caller's r1, its saved LR at 8(caller's r1))
    if let Some(c) = m.cpu() {
        let (mut sp, mut chain) = (c.gpr[1], vec![c.pc, c.lr]);
        for _ in 0..24 {
            let Ok(next) = m.mac.mem.read_u32(sp) else { break };
            if next == 0 || next <= sp { break }
            chain.push(m.mac.mem.read_u32(next + 8).unwrap_or(0));
            sp = next;
        }
        eprintln!("BACKCHAIN {chain:x?}");
        if std::env::var("PS7_DUMP_R31").is_ok() {
            let r31 = c.gpr[31];
            let b: Vec<u8> = (0..180).map(|i| m.mac.mem.read_u8(r31 + i).unwrap_or(0)).collect();
            eprintln!("R31 {r31:#x} {}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
            let mut sp = c.gpr[1];
            for k in 0..4 { let Ok(n) = m.mac.mem.read_u32(sp) else { break }; let regs: Vec<u32> = (1..=4).map(|j| m.mac.mem.read_u32(n - 4 * j).unwrap_or(0)).collect(); eprintln!("FRAME {k} {n:#x} saved r31..r28 {regs:x?}"); sp = n; }
        }
    }
    let keep: usize = std::env::var("PS7_LOG_LINES").ok().and_then(|s| s.parse().ok()).unwrap_or(80);
    let log = &m.mac.log;
    for l in log.iter().skip(log.len().saturating_sub(keep)) { eprintln!("LOG {l}"); }
    let mut refused = m.mac.refused.clone();
    refused.sort();
    refused.dedup();
    eprintln!("REFUSED {refused:?}");
    for c in GuestMachine::ui(&m).calls.iter().rev().take(15) { eprintln!("UI {}", format!("{c:?}").chars().take(300).collect::<String>()); }
    for r in GuestMachine::ui(&m).requests.values() { eprintln!("REQUEST {}", format!("{r:?}").chars().take(400).collect::<String>()); }
    let vol = GuestMachine::volume(&mut m);
    let hd = vol.volume("Macintosh HD").unwrap();
    let mut stack = vec![(hd, String::from("Macintosh HD"))];
    while let Some((d, path)) = stack.pop() {
        for &c in vol.children(d) {
            let p = format!("{path}:{}", vol.name(c));
            eprintln!("INSTALLED {p} {}", vol.file_len(c).unwrap_or(0));
            if vol.is_dir(c) { stack.push((c, p)); }
        }
    }
    if let Ok(dir) = std::env::var("PS7_EXPORT_DIR") { export(vol, hd, std::path::Path::new(&dir)); }
}
