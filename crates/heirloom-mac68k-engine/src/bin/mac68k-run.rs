//! Host runner: `mac68k-run [--census] [--32bit] [--machine NAME] [--idle TICKS] [--pcs N] [--insns N] [--dump ADDR N] [--event EV]... (--fixture | <application file>)`.
//! Launches a classic Mac application (MacBinary, AppleDouble header file or raw resource fork) against a
//! RecordingGuestUi, runs it until it exits, blocks in an event call or spends its instruction budget, then prints the
//! bridged-call log and the recorded UI calls. `--census` stops at the first unimplemented trap and prints it as
//! `NEXT-NEEDED _TrapName ($Axxx)`: run a real application, bridge what it names, run it again.
//! `--event close` / `--event menu:ID:ITEM` queue a native close request / menu command (window 1) before running.
use heirloom_ui_abi::{CallbackToken, GuestUiEvent, RecordingGuestUi, UiObjectRef};
use heirloom_mac68k_engine::fixture::test_app_macbinary;
use heirloom_mac68k_engine::machine::UNIMPLEMENTED;
use heirloom_mac68k_engine::Mac;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut machine = None::<String>;
    let mut idle = 0u32;
    let mut put_bins: Vec<(String, String)> = Vec::new();
    let mut stage_dirs: Vec<String> = Vec::new();
    let mut save_ppm: Option<String> = None;
    let (mut census, mut events, mut bit32, mut pcs, mut insns, mut dump) = (false, Vec::new(), false, 0usize, 5_000_000u64, None::<(u32, u32)>);
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--census" => { census = true; args.remove(0); }
            "--implemented" => { let m = Mac::new(RecordingGuestUi::new()).expect("machine"); for n in m.implemented() { println!("{n}"); } return; } // every registered trap name (corpus-traps-mac.py --rank input)
            "--idle" => { args.remove(0); if args.is_empty() { usage() } idle = args.remove(0).parse().unwrap_or_else(|_| usage()); } // virtual 60 Hz ticks the host lets pass while the program waits for an event (null events)
            "--32bit" => { bit32 = true; args.remove(0); } // 32-bit addressing: the top address byte is significant (default: 24-bit)
            "--insns" => { args.remove(0); if args.is_empty() { usage() } insns = args.remove(0).parse().unwrap_or_else(|_| usage()); } // instruction budget (default 5,000,000)
            "--dump" => { args.remove(0); if args.len() < 2 { usage() } let a = u32::from_str_radix(args.remove(0).trim_start_matches("0x"), 16).unwrap_or_else(|_| usage()); let n = args.remove(0).parse().unwrap_or_else(|_| usage()); dump = Some((a, n)); } // hex-dump N bytes at ADDR after the run
            "--pcs" => { args.remove(0); if args.is_empty() { usage() } pcs = args.remove(0).parse().unwrap_or_else(|_| usage()); } // print the last N instruction addresses
            "--machine" => { args.remove(0); if args.is_empty() { usage() } machine = Some(args.remove(0)); } // profile name: mac-ii (default), mac-plus, mac-se
            "--put-bin" => { args.remove(0); if args.is_empty() { usage() } let v = args.remove(0); let (dir, path) = v.split_once('=').unwrap_or_else(|| usage()); put_bins.push((dir.to_string(), path.to_string())); }
            "--dir" => { args.remove(0); if args.is_empty() { usage() } stage_dirs.push(args.remove(0)); }
            "--save-ppm" => { args.remove(0); if args.is_empty() { usage() } save_ppm = Some(args.remove(0)); }
            "--event" => { args.remove(0); if args.is_empty() { usage() } events.push(args.remove(0)); }
            _ => break,
        }
    }
    let arg = args.first().cloned().unwrap_or_else(|| usage());
    let file = if arg == "--fixture" { test_app_macbinary() } else {
        std::fs::read(&arg).unwrap_or_else(|e| { eprintln!("{arg}: {e}"); std::process::exit(1) })
    };
    let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
    m.ui.drain_audio = true; // nobody is listening: queued audio counts as played, so completion routines are not left waiting
    m.stop_on_unimplemented = census;
    if let Some(name) = &machine { if let Err(e) = m.set_machine(name) { eprintln!("{}", e.0); std::process::exit(2) } }
    if bit32 { m.set_address_mode(heirloom_mac68k_engine::addrmode::AddrMode::Bit32).expect("mode"); }
    if let Err(e) = m.launch(&file) { eprintln!("launch failed: {e}"); std::process::exit(1) }
    for (subdir, host_path) in &put_bins {
        let raw = std::fs::read(host_path).unwrap_or_else(|e| { eprintln!("read {host_path}: {e}"); std::process::exit(1) });
        let opened = heirloom_fmt_resfork::container::open(&raw).unwrap_or_else(|e| { eprintln!("macbin {host_path}: {e:?}"); std::process::exit(1) });
        let root = m.fm.tree.children(heirloom_volume::ROOT)[0];
        let dir = if subdir.is_empty() || subdir == "." { root } else { m.fm.tree.mkdirs(root, subdir.split('/').filter(|s| !s.is_empty())).expect("mkdirs") };
        let name = opened.name.unwrap_or_else(|| std::path::Path::new(host_path).file_stem().unwrap().to_string_lossy().into_owned());
        let data = opened.data.map(|r| raw[r].to_vec()).unwrap_or_default();
        let id = m.fm.tree.put_file(dir, &name, data).expect("put_file");
        let n = m.fm.tree.get_mut(id);
        n.rsrc = raw[opened.rsrc].to_vec();
        if let Some(ft) = opened.file_type { n.finfo.file_type = ft; }
        if let Some(cr) = opened.creator { n.finfo.creator = cr; }
    }
    for host_dir in &stage_dirs {
        let root = m.fm.tree.children(heirloom_volume::ROOT)[0];
        stage_host_dir(&mut m.fm.tree, root, std::path::Path::new(host_dir));
    }
    let mut staged_events: std::collections::VecDeque<Option<GuestUiEvent>> = std::collections::VecDeque::new();
    let win = UiObjectRef { id: 1, generation: 1 };
    for e in &events {
        let parts: Vec<&str> = e.split(':').collect();
        match parts.as_slice() {
            ["close"] => staged_events.push_back(Some(GuestUiEvent::CloseRequested { win, token: CallbackToken(0) })),
            ["menu", id, item] => staged_events.push_back(Some(GuestUiEvent::MenuCommand { win, token: CallbackToken(id.parse::<u64>().unwrap() << 16 | item.parse::<u64>().unwrap()) })),
            ["key", wid, code, ch] => {
                let w = UiObjectRef { id: wid.parse().unwrap(), generation: 1 };
                let c: u32 = code.parse().unwrap();
                staged_events.push_back(Some(GuestUiEvent::Key { win: w, code: c, text: (*ch).to_string(), modifiers: Default::default(), down: true }));
                staged_events.push_back(Some(GuestUiEvent::Key { win: w, code: c, text: (*ch).to_string(), modifiers: Default::default(), down: false }));
            }
            ["click", wid, x, y] => {
                let w = UiObjectRef { id: wid.parse().unwrap(), generation: 1 };
                let (px, py): (i32, i32) = (x.parse().unwrap(), y.parse().unwrap());
                staged_events.push_back(Some(GuestUiEvent::primary(w, px, py, heirloom_ui_abi::PointerPhase::Down)));
                staged_events.push_back(Some(GuestUiEvent::primary(w, px, py, heirloom_ui_abi::PointerPhase::Up)));
            }
            ["wait", n] => {
                for _ in 0..n.parse::<usize>().unwrap() { staged_events.push_back(None); }
            }
            ["drag", wid, x0, y0, x1, y1] => {
                let w = UiObjectRef { id: wid.parse().unwrap(), generation: 1 };
                let (ax, ay, bx, by): (i32, i32, i32, i32) = (x0.parse().unwrap(), y0.parse().unwrap(), x1.parse().unwrap(), y1.parse().unwrap());
                staged_events.push_back(Some(GuestUiEvent::primary(w, ax, ay, heirloom_ui_abi::PointerPhase::Down)));
                for s in 1..=8 {
                    let mx = ax + (bx - ax) * s / 8;
                    let my = ay + (by - ay) * s / 8;
                    staged_events.push_back(Some(GuestUiEvent::primary(w, mx, my, heirloom_ui_abi::PointerPhase::Move)));
                }
                staged_events.push_back(Some(GuestUiEvent::primary(w, bx, by, heirloom_ui_abi::PointerPhase::Up)));
            }
            _ => { eprintln!("unknown event {e}"); std::process::exit(2) }
        }
    }
    if idle == 0 {
        m.ui.events.extend(staged_events.drain(..).flatten());
    }
    let ran = (|| {
        if idle == 0 {
            return m.run_loop(insns);
        }
        let per_tick = (insns / u64::from(idle).max(1)).clamp(500_000, 5_000_000);
        let mut r = m.run_loop(per_tick.min(insns))?;
        // The host's 60 Hz clock: advance 1 tick per frame so WaitNextEvent and TickCount spin-waits both progress.
        for t in 0..idle {
            if r.stats.halted || r.stats.insns >= insns { break; }
            if t % 8 == 3 {
                if let Some(Some(ev)) = staged_events.pop_front() {
                    m.ui.events.push_back(ev);
                }
            }
            m.advance_ticks(1);
            let step_budget = per_tick.min(insns - r.stats.insns);
            let more = m.run_loop(step_budget)?;
            r = heirloom_mac68k_engine::run::RunReport { stats: heirloom_mac68k_engine::cpu::RunStats { insns: r.stats.insns + more.stats.insns, bus_faults: r.stats.bus_faults + more.stats.bus_faults, ..more.stats } };
        }
        Ok::<_, String>(r)
    })();
    match ran {
        Ok(r) => println!("ran {} instructions, halted={}, blocked={}, bus_faults={}", r.stats.insns, r.stats.halted, r.stats.blocked, r.stats.bus_faults),
        Err(e) => { eprintln!("run failed: {e}"); std::process::exit(1) }
    }
    if pcs > 0 { let r = heirloom_mac68k_engine::cpu::recent_pcs(); println!("recent pcs: {:x?}", &r[r.len().saturating_sub(pcs)..]); }
    if let Some((a, n)) = dump { if let Ok(sp) = m.mem.span(a, n) { if let Ok(b) = m.mem.bytes(sp) { println!("dump {a:#x}: {}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()); } } }
    if let Some(ppm_path) = save_ppm {
        let mut sizes: std::collections::HashMap<u32, (usize, usize)> = Default::default();
        let mut next_id = 0u32;
        let mut target_win = None::<u32>;
        for c in &m.ui.calls {
            match c {
                heirloom_ui_abi::UiCall::OpenWindow(w) => {
                    next_id += 1;
                    sizes.insert(next_id, (w.rect.width.max(1) as usize, w.rect.height.max(1) as usize));
                }
                heirloom_ui_abi::UiCall::AddControl(..) | heirloom_ui_abi::UiCall::OpenAudio(..) => {
                    next_id += 1;
                }
                heirloom_ui_abi::UiCall::PresentPlane(w, p) if p.ops.iter().any(|o| matches!(o, heirloom_ui_abi::DrawOp::Pixels { .. })) => {
                    if let Some(&(ww, hh)) = sizes.get(&w.id) {
                        if ww >= 320 && hh >= 200 { target_win = Some(w.id); }
                    }
                }
                _ => {}
            }
        }
        if let Some(wid) = target_win {
            let (w, h) = sizes[&wid];
            let mut fb = vec![0u32; w * h];
            for c in &m.ui.calls {
                if let heirloom_ui_abi::UiCall::PresentPlane(wr, p) = c {
                    if wr.id != wid { continue; }
                    for op in &p.ops {
                        if let heirloom_ui_abi::DrawOp::Pixels { rect, rgb } = op {
                            let rw = rect.width.max(0) as usize;
                            for dy in 0..rect.height.max(0) as usize {
                                let ty = rect.top + dy as i32;
                                if ty < 0 || ty >= h as i32 { continue; }
                                for dx in 0..rw {
                                    let tx = rect.left + dx as i32;
                                    if tx < 0 || tx >= w as i32 { continue; }
                                    if let Some(&px) = rgb.get(dy * rw + dx) {
                                        fb[ty as usize * w + tx as usize] = px;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for &px in &fb {
                out.push(((px >> 16) & 0xff) as u8);
                out.push(((px >> 8) & 0xff) as u8);
                out.push((px & 0xff) as u8);
            }
            std::fs::write(&ppm_path, out).expect("write ppm");
            println!("saved {w}x{h} composited frame (win {wid}) to {ppm_path}");
        } else if let Some((&wptr, bm)) = m.qd.canvases.iter().max_by_key(|(_, b)| (b.width() as usize) * (b.height() as usize)) {
            let (w, h) = (bm.width() as usize, bm.height() as usize);
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let v = bm.get(x, y).unwrap_or(0);
                    let rgb = if bm.depth() == 1 { if v != 0 { 0x000000 } else { 0xFFFFFF } } else { v };
                    out.push(((rgb >> 16) & 0xff) as u8);
                    out.push(((rgb >> 8) & 0xff) as u8);
                    out.push((rgb & 0xff) as u8);
                }
            }
            std::fs::write(&ppm_path, out).expect("write ppm");
            println!("saved {w}x{h} 1-bit/direct canvas (port {wptr:#x}) to {ppm_path}");
        } else {
            println!("no large pixel frame presented");
        }
    }
    for l in &m.log { println!("{l}"); }
    for c in &m.ui.calls {
        match c {
            heirloom_ui_abi::UiCall::PresentPlane(w, p) => println!("ui: PresentPlane({w:?}, {} ops)", p.ops.len()),
            other => println!("ui: {other:?}"),
        }
    }
    if census {
        match m.refused.iter().rev().find(|r| r.starts_with(UNIMPLEMENTED)) {
            Some(r) => println!("NEXT-NEEDED {}{}", r.trim_start_matches(UNIMPLEMENTED).trim(),
                m.unimplemented_at.last().map(|(_, pc)| format!(" at pc={pc:#x}")).unwrap_or_default()),
            None => println!("NEXT-NEEDED none (no unimplemented trap reached)"),
        }
    }
    for r in &m.refused { println!("refused: {r}"); }
    if !m.refused.is_empty() { std::process::exit(3) }
}

fn usage() -> ! { eprintln!("usage: mac68k-run [--census] [--32bit] [--machine NAME] [--idle TICKS] [--pcs N] [--insns N] [--dir DIR] [--dump ADDR N] [--event close|menu:ID:ITEM]... --fixture | <application file>"); std::process::exit(2) }

fn stage_host_dir(tree: &mut heirloom_volume::Tree, dst_dir: heirloom_volume::NodeId, host_dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(host_dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| (e.file_name().to_string_lossy().ends_with(".rsrc"), e.file_name()));
    for entry in entries {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') { continue; }
        if p.is_dir() {
            let sub = tree.mkdirs(dst_dir, [name.as_str()]).expect("mkdirs");
            stage_host_dir(tree, sub, &p);
        } else if let Some(stem) = name.strip_suffix(".rsrc") {
            let Ok(raw) = std::fs::read(&p) else { continue };
            let Ok(opened) = heirloom_fmt_resfork::container::open(&raw) else { continue };
            let id = match tree.find_child(dst_dir, stem) {
                Some(existing) => existing,
                None => tree.put_file(dst_dir, stem, Vec::new()).expect("put_file"),
            };
            let n = tree.get_mut(id);
            n.rsrc = raw[opened.rsrc].to_vec();
            if let Some(ft) = opened.file_type { n.finfo.file_type = ft; }
            if let Some(cr) = opened.creator { n.finfo.creator = cr; }
        } else if p.is_file() && tree.find_child(dst_dir, &name).is_none() {
            let data = std::fs::read(&p).unwrap_or_default();
            let _ = tree.put_file(dst_dir, &name, data);
        }
    }
}
