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
use ps7_common::export;

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

/// Every log line since the last drain, appended to `PS7_LOG_FILE` when set (polling noise left out); else the newest 4000 kept.
fn drain(m: &mut MacPpc<RecordingGuestUi>) {
    let mut lines = core::mem::take(&mut m.mac.log);
    let Ok(path) = std::env::var("PS7_LOG_FILE") else {
        lines.drain(..lines.len().saturating_sub(4000));
        m.mac.log = lines;
        return;
    };
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    for l in lines {
        if ["PtInRect", "BlockMove", "IsDialogEvent", "null event", "GetMouse", "Button ->", "FrontWindow", "would idle"].iter().any(|n| l.contains(n)) { continue }
        writeln!(f, "{l}").unwrap();
    }
}

/// Run `slices` host slices of 5M instructions / 20 ms. False when the program stopped.
fn run(m: &mut MacPpc<RecordingGuestUi>, t: &mut u64, slices: u32) -> bool {
    for i in 0..slices {
        drain(m);
        let (st, _) = GuestMachine::step(m, 5_000_000);
        if matches!(st, Step::Fault(_) | Step::Exited) {
            drain(m);
            eprintln!("slice {i}: {st:?} {:?}", m.fault());
            if let Some(c) = m.cpu() {
                let pcs: Vec<u32> = c.recent_pcs().collect();
                eprintln!("PCS {pcs:x?}\nLR {:#x} CTR {:#x} GPR {:x?}", c.lr, c.ctr, c.gpr);
                // a cross-TOC glue (lwz r12,d(r2) ... bctr) in the trace: the import its TOC slot holds
                let toc = m.mac.mem.read_u32(c.gpr[1] + 20).unwrap_or(0);
                for &pc in &pcs {
                    let w = m.mac.mem.read_u32(pc).unwrap_or(0);
                    if w >> 16 == 0x8182 { let slot = toc.wrapping_add(w as u16 as i16 as i32 as u32); eprintln!("GLUE at {pc:#x}: TOC {toc:#x} slot {slot:#x} -> {:?} value {:#x}", m.import_slots.get(&slot), m.mac.mem.read_u32(slot).unwrap_or(0)); }
                }
            }
            return false;
        }
        *t += 20_000;
        GuestMachine::advance_clock_to(m, *t).unwrap();
    }
    true
}

/// The newest window the program opened (its native ref).
fn front(m: &MacPpc<RecordingGuestUi>) -> heirloom_ui_abi::UiObjectRef {
    *m.mac.windows.values().filter_map(|w| w.native.as_ref()).max_by_key(|r| r.id).expect("a window")
}

/// The window's picture as the host shows it now: every pixel frame of every plane of the newest window, oldest first (PPM).
fn shot(m: &MacPpc<RecordingGuestUi>, path: &str) {
    let ui = GuestMachine::ui(m);
    let Some((&(win, _), _)) = ui.plane_frames.iter().last() else { eprintln!("shot: no plane"); return };
    let (mut w, mut h) = (1i32, 1i32);
    for ((wid, _), frames) in &ui.plane_frames { if *wid == win { for f in frames { w = w.max(f.rect.left + f.rect.width); h = h.max(f.rect.top + f.rect.height); } } }
    let mut img = vec![0x00C0_C0C0u32; (w * h) as usize];
    for ((wid, _), frames) in &ui.plane_frames {
        if *wid != win { continue }
        for f in frames { for op in &f.ops { if let heirloom_ui_abi::DrawOp::Pixels { rect, rgb } = op {
            for y in 0..rect.height { for x in 0..rect.width {
                let (gx, gy) = (rect.left + x + f.rect.left, rect.top + y + f.rect.top);
                if gx < w && gy < h { img[(gy * w + gx) as usize] = rgb[(y * rect.width + x) as usize]; }
            } }
        } } }
    }
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in img { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]); }
    std::fs::write(path, out).unwrap();
    eprintln!("shot: window {win} {w}x{h} -> {path}");
}

/// Drive the running installer with `;`-separated actions (the Warcraft III harness's verbs). False when the program stopped.
fn act(m: &mut MacPpc<RecordingGuestUi>, t: &mut u64, actions: &str) -> bool {
    use heirloom_ui_abi::{GuestUiEvent, PointerPhase};
    for a in actions.split(';').filter(|a| !a.is_empty()) {
        let (verb, arg) = a.split_once(':').unwrap_or((a, ""));
        match verb {
            "run" => if !run(m, t, arg.parse().unwrap()) { return false },
            "shot" => shot(m, arg),
            "click" => {
                let (x, y) = arg.split_once(',').unwrap();
                let (x, y): (i32, i32) = (x.parse().unwrap(), y.parse().unwrap());
                let win = front(m);
                GuestMachine::deliver(m, GuestUiEvent::primary(win, x, y, PointerPhase::Down)).unwrap();
                if !run(m, t, 5) { return false }
                GuestMachine::deliver(m, GuestUiEvent::primary(win, x, y, PointerPhase::Up)).unwrap();
            }
            "button" => {
                let found = GuestMachine::ui(m).find_control(|c| c.label == arg && matches!(c.kind, heirloom_ui_abi::ControlKind::Button));
                let (w, c, _) = found.unwrap_or_else(|| panic!("no live button {arg:?}"));
                GuestMachine::ui_mut(m).push_intent(w, c, heirloom_ui_abi::Intent::Activate).unwrap();
            }
            "field" => {
                let (n, text) = arg.split_once('=').unwrap();
                let n: usize = n.parse().unwrap();
                let ui = GuestMachine::ui(m);
                let (id, (w, _)) = ui.controls.iter().filter(|(_, (_, c))| matches!(c.kind, heirloom_ui_abi::ControlKind::Field { .. })).nth(n).unwrap_or_else(|| panic!("no field {n}"));
                let (w, c) = (*w, heirloom_ui_abi::UiObjectRef { id: *id, generation: 1 });
                GuestMachine::ui_mut(m).push_intent(w, c, heirloom_ui_abi::Intent::CommitText(text.into())).unwrap();
            }
            // `answer:N`: the oldest open request (a native alert the program asked) is answered with button N
            "answer" => {
                let ui = GuestMachine::ui_mut(m);
                let Some(req) = ui.open_requests().into_iter().next() else { panic!("no open request to answer") };
                ui.answer_request(req, arg.parse().unwrap()).unwrap();
            }
            "controls" => for (id, (_, c)) in &GuestMachine::ui(m).controls { eprintln!("CONTROL {id} {:?} {:?} {:?}", c.kind, c.label, c.rect); },
            other => panic!("action verb {other}"),
        }
    }
    true
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
