//! What the Photoshop 7 host harnesses (ps7_installer.rs, ps7_launch.rs) share: writing a volume's tree out in the installed-tree form, and
//! driving the running program (`PS7_ACTIONS`: run:N, click:X,Y, button:LABEL, field:N=TEXT, answer:N, controls:, shot:F.ppm).

/// The tree under `top`, written for staging (corpus lane, never committed): each file's data fork at its Mac path (`:` -> `/`), its
/// resource fork beside it as `<name>.rsrc`, and `manifest.tsv` (path, type, creator, finder flags, data bytes, resource bytes).
pub fn export(vol: &heirloom_volume::Tree, top: heirloom_volume::NodeId, dir: &std::path::Path) {
    let mut manifest = String::from("# path\ttype\tcreator\tflags\tdata\trsrc\n");
    let mut stack = vec![(top, dir.to_path_buf(), String::new())];
    while let Some((d, host, rel)) = stack.pop() {
        std::fs::create_dir_all(&host).unwrap();
        for &c in vol.children(d) {
            let name = vol.name(c).replace('/', ":");
            let (h, r) = (host.join(&name), if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") });
            if vol.is_dir(c) { stack.push((c, h, r)); continue }
            let n = vol.get(c);
            // an in-memory file's bytes, or a stream's read through its source (a staged tree the program did not rewrite)
            let streamed;
            let data = match vol.file_data(c) {
                Some(d) => d,
                None => {
                    let mut v = vec![0u8; vol.file_len(c).unwrap_or(0) as usize];
                    let got = vol.read_at(c, 0, &mut v).unwrap_or(0);
                    assert_eq!(got, v.len(), "the stream {name} read back whole");
                    streamed = v;
                    &streamed[..]
                }
            };
            std::fs::write(&h, data).unwrap();
            if !n.rsrc.is_empty() { std::fs::write(host.join(format!("{name}.rsrc")), &n.rsrc).unwrap(); }
            let fourcc = |b: [u8; 4]| b.iter().map(|&x| if x.is_ascii_graphic() || x == b' ' { char::from(x) } else { '?' }).collect::<String>();
            manifest += &format!("{r}\t{}\t{}\t{:#06x}\t{}\t{}\n", fourcc(n.finfo.file_type), fourcc(n.finfo.creator), n.finfo.flags, data.len(), n.rsrc.len());
        }
    }
    std::fs::write(dir.join("manifest.tsv"), manifest).unwrap();
    eprintln!("EXPORTED -> {}", dir.display());
}


use heirloom_common::{GuestMachine, Step};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

/// Every log line since the last drain, appended to `PS7_LOG_FILE` when set (polling noise left out); else the newest 4000 kept.
pub fn drain(m: &mut MacPpc<RecordingGuestUi>) {
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
pub fn run(m: &mut MacPpc<RecordingGuestUi>, t: &mut u64, slices: u32) -> bool {
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
pub fn front(m: &MacPpc<RecordingGuestUi>) -> heirloom_ui_abi::UiObjectRef {
    *m.mac.windows.values().filter_map(|w| w.native.as_ref()).max_by_key(|r| r.id).expect("a window")
}

/// The window's picture as the host shows it now: every pixel frame of every plane of the newest window, oldest first (PPM).
pub fn shot(m: &MacPpc<RecordingGuestUi>, path: &str) {
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

/// Drive the running program with `;`-separated actions (the Warcraft III harness's verbs). False when the program stopped.
pub fn act(m: &mut MacPpc<RecordingGuestUi>, t: &mut u64, actions: &str) -> bool {
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
            // `plane:` the text and fills the newest presented plane holds (where a program-drawn button is)
            "plane" => if let Some(c) = GuestMachine::ui(m).calls.iter().rev().find(|c| matches!(c, heirloom_ui_abi::UiCall::PresentPlane(..))) {
                if let heirloom_ui_abi::UiCall::PresentPlane(_, p) = c { for op in &p.ops { if !matches!(op, heirloom_ui_abi::DrawOp::Pixels { .. }) { eprintln!("OP {op:?}"); } } }
            },
            other => panic!("action verb {other}"),
        }
    }
    true
}

