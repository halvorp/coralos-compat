//! Live host harness & replay test for Photoshop 7.0:
//! 1. `File > New...` dialog controls & plane ops (Name, Preset, Width, Height, Resolution, Mode, Contents, OK/Cancel)
//! 2. Options bar edit fields (`Feather`, `Width`, `Height`)
//! 3. End-to-end `File > New -> OK -> draw stroke -> File > Save` round-trip.
use heirloom_common::{GuestMachine, Step};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::{DrawOp, GuestUiEvent, Modifiers, PointerPhase, RecordingGuestUi};

mod ps7_common;
mod wdef_bindings;

struct Files;
impl heirloom_volume::StreamSource for Files {
    fn read_at(&self, locator: &str, offset: u64, dst: &mut [u8]) -> Option<usize> {
        let f = std::fs::File::open(locator).ok()?;
        std::os::unix::fs::FileExt::read_at(&f, dst, offset).ok()
    }
}

fn fourcc(s: &str) -> [u8; 4] {
    let mut b = [b' '; 4];
    for (i, c) in s.bytes().take(4).enumerate() { b[i] = c; }
    b
}

fn boot_ps7() -> Option<(MacPpc<RecordingGuestUi>, u64)> {
    let root = std::path::PathBuf::from(
        std::env::var("PS7_INSTALLED").unwrap_or_else(|_| String::from("/data/build/corpus-bin/mac/photoshop7-tryout/installed")),
    );
    let manifest = std::fs::read_to_string(root.join("manifest.tsv")).ok()?;
    let rel = "Applications (Mac OS 9)/Adobe Photoshop 7/Adobe Photoshop 7.0/Contents/MacOSClassic/Adobe Photoshop 7.0";
    let (data, rsrc) = (std::fs::read(root.join(rel)).ok()?, std::fs::read(root.join(format!("{rel}.rsrc"))).ok()?);
    let leaf = "Adobe Photoshop 7.0";
    let app = heirloom_fmt_resfork::writer::macbinary(leaf, b"APPL", b"8BIM", &data, &rsrc);
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), &app).unwrap();
    m.mac.set_display_size(1280, 800).unwrap();
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        m.exec().set_code_mem(Box::new(heirloom_cpu_ppc::HostCodeMem::new()));
        m.exec().tier = heirloom_cpu_ppc::Tier::Jit;
    }
    for b in wdef_bindings::load("macppc") {
        m.add_window_def_binding(b);
    }
    for pc in wdef_bindings::load_plane_chrome("macppc") {
        m.mac.add_plane_chrome_binding(pc);
    }
    let vol = GuestMachine::volume(&mut m);
    vol.enable_streams();
    vol.set_stream_source(Box::new(Files));
    let hd = vol.volume("Macintosh HD").unwrap();
    vol.set_capacity(hd, 8 << 30).unwrap();
    for line in manifest.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 6 || f[0] == rel { continue; }
        let (path, ty, cr) = (f[0], f[1], f[2]);
        let flags = u16::from_str_radix(f[3].trim_start_matches("0x"), 16).unwrap_or(0);
        let len: u64 = f[4].parse().unwrap_or(0);
        let parts: Vec<&str> = path.split('/').collect();
        let (dirs, leaf) = parts.split_at(parts.len() - 1);
        let dirs: Vec<String> = dirs.iter().map(|d| d.replace(':', "/")).collect();
        let leaf = leaf[0].replace(':', "/");
        let dir = vol.mkdirs(hd, dirs.iter().map(String::as_str)).unwrap();
        let host = root.join(path);
        let Ok(id) = vol.put_stream(dir, &leaf, len, host.to_str().unwrap()) else { continue };
        let node = vol.get_mut(id);
        node.finfo.file_type = fourcc(ty);
        node.finfo.creator = fourcc(cr);
        node.finfo.flags = flags;
        if f[5] != "0" { node.rsrc = std::fs::read(format!("{}.rsrc", host.display())).unwrap_or_default(); }
    }
    let home: Vec<String> = std::iter::once(String::from("Macintosh HD")).chain(rel.split('/').map(|c| c.replace(':', "/"))).collect();
    let home: Vec<&str> = home[..home.len() - 1].iter().map(String::as_str).collect();
    let vol = GuestMachine::volume(&mut m);
    let hd = vol.volume("Macintosh HD").unwrap();
    vol.mkdirs(hd, home[1..].iter().copied()).unwrap();
    GuestMachine::set_home(&mut m, &home).unwrap();
    GuestMachine::ui_mut(&mut m).auto_cancel_requests = false;
    GuestMachine::start_file(&mut m, &app, leaf).unwrap();

    let mut t = 0u64;
    assert!(ps7_common::act(&mut m, &mut t, "run:200;click:565,240"), "PS7 tryout click failed");
    for _ in 0..3500 {
        let (st, _) = GuestMachine::step(&mut m, 5_000_000);
        if matches!(st, Step::Fault(_) | Step::Exited) { break; }
        let open = GuestMachine::ui(&m).open_requests();
        for req in open {
            GuestMachine::ui_mut(&mut m).answer_request(req, 0).unwrap();
        }
        t += 20_000;
        GuestMachine::advance_clock_to(&mut m, t).unwrap();
    }
    Some((m, t))
}

#[test]
#[ignore]
fn probe_ps7_file_new_and_save() {
    let Some((mut m, mut t)) = boot_ps7() else { return };
    // Clear startup logs so we see ONLY File > New... activity!
    m.mac.log.clear();
    m.mac.refused.clear();
    let strip = m.mac.menu_strip().expect("menu strip");
    let mut new_token = None;
    let mut save_token = None;
    for mm in &strip.menus {
        if mm.title == "File" {
            for it in &mm.items {
                eprintln!("FILE ITEM: {:?} enabled={} token={:?}", it.label, it.enabled, it.token);
                if it.label.starts_with("New") { new_token = Some(it.token); }
                if it.label == "Save" || it.label == "Save As..." {
                    if save_token.is_none() { save_token = Some(it.token); }
                }
            }
        }
    }
    let new_tok = new_token.expect("File > New... menu item");
    let front = ps7_common::front(&m);
    eprintln!("=== Triggering File > New... ({new_tok:?}) on {front:?} ===");
    GuestMachine::deliver(&mut m, GuestUiEvent::MenuCommand { win: front, token: new_tok }).unwrap();
    for _ in 0..200 {
        let (st, _) = GuestMachine::step(&mut m, 5_000_000);
        if matches!(st, Step::Fault(_) | Step::Exited) {
            eprintln!("STOPPED: {st:?} {:?}", m.fault());
            if let Some(c) = m.cpu() {
                let pcs: Vec<u32> = c.recent_pcs().collect();
                eprintln!("PCS {:x?} LR={:#x} CTR={:#x} GPR={:x?}", pcs, c.lr, c.ctr, c.gpr);
                if let Some(call) = m.mac.mem.read_u32(c.lr.wrapping_sub(4)).ok().filter(|w| w >> 26 == 18) {
                    let li = ((call & 0x03FF_FFFC) as i32) << 6 >> 6;
                    let glue = c.lr.wrapping_sub(4).wrapping_add(li as u32);
                    let first = m.mac.mem.read_u32(glue).unwrap_or(0);
                    eprintln!("CALLER-CALL glue={glue:#x} first={first:#x}");
                    if first >> 16 == 0x8182 {
                        let toc = m.mac.mem.read_u32(c.gpr[1] + 20).unwrap_or(0);
                        let slot = toc.wrapping_add(first as u16 as i16 as i32 as u32);
                        let tv = m.mac.mem.read_u32(slot).unwrap_or(0);
                        eprintln!("GLUESLOT toc={toc:#x} slot={slot:#x} -> {:?} tv={tv:#x}", m.import_slots.get(&slot));
                        let mut deltas: std::collections::BTreeMap<u32, u32> = Default::default();
                        for k in 0..64u32 {
                            let w = slot.wrapping_sub(128).wrapping_add(4 * k);
                            let v = m.mac.mem.read_u32(w).unwrap_or(0);
                            if v == 0 { continue; }
                            let hits: Vec<u32> = m.import_slots.keys().copied().filter(|&a| m.mac.mem.read_u32(a).ok() == Some(v)).collect();
                            if hits.len() == 1 { *deltas.entry(w.wrapping_sub(hits[0])).or_default() += 1; }
                        }
                        if let Some((&d, &n)) = deltas.iter().max_by_key(|(_, &n)| n) {
                            let imp = slot.wrapping_sub(d);
                            eprintln!("MIRROR-NOW delta={d:#x} ({n} matches) -> import {imp:#x} {:?}", m.import_slots.get(&imp));
                        }
                    }
                }
                // Also dump 8 instructions around LR-16..LR+16
                for off in (-16i32..=16).step_by(4) {
                    let a = c.lr.wrapping_add(off as u32);
                    eprintln!("  @{a:#x}: {:#010x}", m.mac.mem.read_u32(a).unwrap_or(0));
                }
            }
            break;
        }
        t += 20_000;
        GuestMachine::advance_clock_to(&mut m, t).unwrap();
    }

    eprintln!("=== OPEN WINDOWS AFTER File > New... ===");
    for (&addr, w) in &m.mac.windows {
        eprintln!("  WIN addr={addr:#x} native={:?} title={:?} vis={} rect={:?} proc={:#x}", w.native, w.title, w.visible, w.rect, w.proc_id);
    }
    eprintln!("=== ALL GUEST CONTROLS IN Mac::tr.procs ({}) ===", m.mac.tr.procs.len());
    for (&ctl, &proc_id) in &m.mac.tr.procs {
        if let Ok(f) = m.mac.control_facts(ctl) {
            eprintln!("  GUEST-CTL {ctl:#x} owner={:#x} proc={proc_id} vis={} hilite={} val={} min={} max={} rect={:?} title={:?} items={:?}",
                f.owner, f.vis, f.hilite, f.value, f.min, f.max, f.rect, f.title, f.items);
        }
    }
    let ui = GuestMachine::ui(&m);
    let new_win = ps7_common::front(&m);
    let mut new_fields = 0usize;
    let mut new_popups = 0usize;
    let mut new_radios = Vec::new();
    let mut new_buttons = Vec::new();
    let mut opts_edit_fields = 0usize;
    for (_id, (w, c)) in &ui.controls {
        if *w == new_win {
            match &c.kind {
                heirloom_ui_abi::ControlKind::Field { editable: true, .. } => new_fields += 1,
                heirloom_ui_abi::ControlKind::Choice { style: heirloom_ui_abi::ChoiceStyle::Popup, .. } => new_popups += 1,
                heirloom_ui_abi::ControlKind::Choice { style: heirloom_ui_abi::ChoiceStyle::Radio, labels, .. } => {
                    if let Some(lbl) = labels.first() { new_radios.push(lbl.clone()); }
                }
                heirloom_ui_abi::ControlKind::Button => new_buttons.push(c.label.clone()),
                _ => {}
            }
        } else if matches!(c.kind, heirloom_ui_abi::ControlKind::Field { editable: true, .. }) {
            opts_edit_fields += 1;
        }
    }
    assert_eq!(new_fields, 4, "File > New must project 4 native editable fields (Name, Width, Height, Resolution)");
    assert_eq!(new_popups, 5, "File > New must project 5 native popups (Preset Sizes, Width/Height/Res units, Mode)");
    assert_eq!(new_radios.len(), 3, "File > New must project 3 Contents radio buttons, got {new_radios:?}");
    assert!(new_radios.iter().any(|r| r == "White") && new_radios.iter().any(|r| r == "Background Color") && new_radios.iter().any(|r| r == "Transparent"),
        "expected White/Background Color/Transparent radios, got {new_radios:?}");
    assert!(new_buttons.iter().any(|b| b == "OK") && new_buttons.iter().any(|b| b == "Cancel"),
        "expected OK and Cancel buttons, got {new_buttons:?}");
    assert_eq!(opts_edit_fields, 3, "Options bar must project 3 bound editable fields (Feather, Width, Height)");
    let new_ctrl_rects: Vec<_> = ui.controls.values().filter(|(w, _)| *w == new_win).map(|(_, c)| c.rect).collect();
    let covered = |r: heirloom_ui_abi::GuestRect| {
        new_ctrl_rects.iter().any(|c| {
            r.left >= c.left && r.top >= c.top && r.left + r.width <= c.left + c.width && r.top + r.height <= c.top + c.height
        })
    };
    let unmasked_pixels_on_new: usize = ui.plane_frames.get(&(new_win.id, 0)).and_then(|fs| fs.last()).map_or(0, |f| {
        f.ops.iter().filter(|op| match op {
            DrawOp::Pixels { rect, .. } => !covered(*rect),
            _ => false,
        }).count()
    });
    assert_eq!(unmasked_pixels_on_new, 0, "File > New latest frame must have zero unmasked guest DrawOp::Pixels blits");
    assert!(m.mac.refused.is_empty(), "File > New had refusals: {:?}", m.mac.refused);

    // Try confirming the New dialog with Return
    m.mac.log.clear();
    m.mac.refused.clear();
    eprintln!("=== Pressing Return on New dialog {new_win:?} ===");
    GuestMachine::deliver(&mut m, GuestUiEvent::Key {
        win: new_win,
        code: 13,
        text: String::from("\r"),
        modifiers: Modifiers(0),
        down: true,
    }).unwrap();
    for _ in 0..200 {
        let (st, _) = GuestMachine::step(&mut m, 5_000_000);
        if matches!(st, Step::Fault(_) | Step::Exited) {
            eprintln!("STOPPED: {st:?} {:?}", m.fault());
            if let Some(c) = m.cpu() {
                let pcs: Vec<u32> = c.recent_pcs().collect();
                eprintln!("PCS {:x?} LR={:#x} CTR={:#x} GPR={:x?}", pcs, c.lr, c.ctr, c.gpr);
                if let Some(call) = m.mac.mem.read_u32(c.lr.wrapping_sub(4)).ok().filter(|w| w >> 26 == 18) {
                    let li = ((call & 0x03FF_FFFC) as i32) << 6 >> 6;
                    let glue = c.lr.wrapping_sub(4).wrapping_add(li as u32);
                    let first = m.mac.mem.read_u32(glue).unwrap_or(0);
                    eprintln!("CALLER-CALL glue={glue:#x} first={first:#x}");
                    if first >> 16 == 0x8182 {
                        let toc = m.mac.mem.read_u32(c.gpr[1] + 20).unwrap_or(0);
                        let slot = toc.wrapping_add(first as u16 as i16 as i32 as u32);
                        let tv = m.mac.mem.read_u32(slot).unwrap_or(0);
                        eprintln!("GLUESLOT toc={toc:#x} slot={slot:#x} -> {:?} tv={tv:#x}", m.import_slots.get(&slot));
                        let mut deltas: std::collections::BTreeMap<u32, u32> = Default::default();
                        for k in 0..64u32 {
                            let w = slot.wrapping_sub(128).wrapping_add(4 * k);
                            let v = m.mac.mem.read_u32(w).unwrap_or(0);
                            if v == 0 { continue; }
                            let hits: Vec<u32> = m.import_slots.keys().copied().filter(|&a| m.mac.mem.read_u32(a).ok() == Some(v)).collect();
                            if hits.len() == 1 { *deltas.entry(w.wrapping_sub(hits[0])).or_default() += 1; }
                        }
                        if let Some((&d, &n)) = deltas.iter().max_by_key(|(_, &n)| n) {
                            let imp = slot.wrapping_sub(d);
                            eprintln!("MIRROR-NOW delta={d:#x} ({n} matches) -> import {imp:#x} {:?}", m.import_slots.get(&imp));
                        }
                    }
                }
                // Also dump 8 instructions around LR-16..LR+16
                for off in (-16i32..=16).step_by(4) {
                    let a = c.lr.wrapping_add(off as u32);
                    eprintln!("  @{a:#x}: {:#010x}", m.mac.mem.read_u32(a).unwrap_or(0));
                }
            }
            break;
        }
        t += 20_000;
        GuestMachine::advance_clock_to(&mut m, t).unwrap();
    }
    eprintln!("=== OPEN WINDOWS AFTER Return ===");
    for (&addr, w) in &m.mac.windows {
        eprintln!("  WIN addr={addr:#x} native={:?} title={:?} vis={} rect={:?} proc={:#x}", w.native, w.title, w.visible, w.rect, w.proc_id);
    }
    for r in &m.mac.refused { eprintln!("  POST-RET REFUSED: {r}"); }
    let doc_win = ps7_common::front(&m);
    if doc_win != new_win {
        eprintln!("=== DOCUMENT WINDOW OPENED: {doc_win:?}! Testing click-drag + Save ===");
        // Count pixels differing from the initial blank canvas background (sampled at (40, 40)).
        let doc_addr = m.mac.windows.iter().find(|(_, w)| w.native == Some(doc_win)).map(|(&a, _)| a).unwrap();
        let bg_pixel = m.mac.qd.canvases.get(&doc_addr).and_then(|bm| bm.get(40, 40)).unwrap_or(0);
        let count_drawn = |m: &MacPpc<RecordingGuestUi>| -> usize {
            m.mac.qd.canvases.get(&doc_addr).map_or(0, |bm| {
                let mut n = 0usize;
                for y in 40..300.min(bm.height()) {
                    for x in 40..400.min(bm.width()) {
                        if bm.get(x, y).unwrap_or(bg_pixel) != bg_pixel { n += 1; }
                    }
                }
                n
            })
        };
        let drawn_before = count_drawn(&m);
        // Select Brush tool by clicking Tools palette at Paintbrush (col 2, row 5 -> x=40, y=102) AND pressing 'b'
        let tools_win = m.mac.windows.values().find(|w| w.visible && w.rect.right - w.rect.left < 80).and_then(|w| w.native);
        if let Some(tw) = tools_win {
            GuestMachine::deliver(&mut m, GuestUiEvent::primary(tw, 40, 102, PointerPhase::Down)).unwrap();
            let _ = ps7_common::run(&mut m, &mut t, 5);
            GuestMachine::deliver(&mut m, GuestUiEvent::primary(tw, 40, 102, PointerPhase::Up)).unwrap();
            let _ = ps7_common::run(&mut m, &mut t, 20);
        }
        // Activate doc window and drag a diagonal brush stroke across the canvas
        GuestMachine::deliver(&mut m, GuestUiEvent::Key {
            win: doc_win,
            code: 0x0B, // Mac virtual keycode for 'b'
            text: String::from("b"),
            modifiers: Modifiers(0),
            down: true,
        }).unwrap();
        let _ = ps7_common::run(&mut m, &mut t, 15);
        GuestMachine::deliver(&mut m, GuestUiEvent::primary(doc_win, 100, 100, PointerPhase::Down)).unwrap();
        let _ = ps7_common::run(&mut m, &mut t, 5);
        for step in 1..=6 {
            GuestMachine::deliver(&mut m, GuestUiEvent::primary(doc_win, 100 + step * 25, 100 + step * 18, PointerPhase::Move)).unwrap();
            let _ = ps7_common::run(&mut m, &mut t, 3);
        }
        GuestMachine::deliver(&mut m, GuestUiEvent::primary(doc_win, 250, 208, PointerPhase::Up)).unwrap();
        let _ = ps7_common::run(&mut m, &mut t, 40);
        let drawn_after = count_drawn(&m);
        eprintln!("  CANVAS DRAWN PIXELS: bg={bg_pixel:#x} before={drawn_before} after={drawn_after}");
        assert!(drawn_after > drawn_before, "brush stroke must paint visible pixels onto Untitled-1 canvas (before={drawn_before}, after={drawn_after})");

        let mut save_tok_now = None;
        let mut save_as_token = None;
        let strip2 = m.mac.menu_strip().expect("menu strip 2");
        for mm in &strip2.menus {
            if mm.title == "File" {
                for it in &mm.items {
                    if it.label.starts_with("Save") {
                        eprintln!("  SAVE ITEM: {:?} enabled={} token={:?}", it.label, it.enabled, it.token);
                        if it.label == "Save" && it.enabled { save_tok_now = Some(it.token); }
                        if it.label == "Save As..." && it.enabled { save_as_token = Some(it.token); }
                    }
                }
            }
        }
        if let Some(st_tok) = save_tok_now.or(save_as_token) {
            eprintln!("=== Triggering File > Save/SaveAs ({st_tok:?}) ===");
            m.mac.log.clear();
            m.mac.refused.clear();
            GuestMachine::deliver(&mut m, GuestUiEvent::MenuCommand { win: doc_win, token: st_tok }).unwrap();
            for _ in 0..200 {
                let (st, _) = GuestMachine::step(&mut m, 5_000_000);
                if matches!(st, Step::Fault(_) | Step::Exited) {
                    eprintln!("SAVE STOPPED: {st:?} {:?}", m.fault());
                    if let Some(c) = m.cpu() {
                        if let Some(call) = m.mac.mem.read_u32(c.lr.wrapping_sub(4)).ok().filter(|w| w >> 26 == 18) {
                            let li = ((call & 0x03FF_FFFC) as i32) << 6 >> 6;
                            let glue = c.lr.wrapping_sub(4).wrapping_add(li as u32);
                            let first = m.mac.mem.read_u32(glue).unwrap_or(0);
                            eprintln!("SAVE CALLER-CALL glue={glue:#x} first={first:#x}");
                            if first >> 16 == 0x8182 {
                                let toc = m.mac.mem.read_u32(c.gpr[1] + 20).unwrap_or(0);
                                let slot = toc.wrapping_add(first as u16 as i16 as i32 as u32);
                                let tv = m.mac.mem.read_u32(slot).unwrap_or(0);
                                eprintln!("SAVE GLUESLOT toc={toc:#x} slot={slot:#x} -> {:?} tv={tv:#x}", m.import_slots.get(&slot));
                                let mut deltas: std::collections::BTreeMap<u32, u32> = Default::default();
                                for k in 0..64u32 {
                                    let w = slot.wrapping_sub(128).wrapping_add(4 * k);
                                    let v = m.mac.mem.read_u32(w).unwrap_or(0);
                                    if v == 0 { continue; }
                                    let hits: Vec<u32> = m.import_slots.keys().copied().filter(|&a| m.mac.mem.read_u32(a).ok() == Some(v)).collect();
                                    if hits.len() == 1 { *deltas.entry(w.wrapping_sub(hits[0])).or_default() += 1; }
                                }
                                if let Some((&d, &n)) = deltas.iter().max_by_key(|(_, &n)| n) {
                                    let imp = slot.wrapping_sub(d);
                                    eprintln!("SAVE MIRROR-NOW delta={d:#x} ({n} matches) -> import {imp:#x} {:?}", m.import_slots.get(&imp));
                                }
                            }
                        }
                    }
                    break;
                }
                t += 20_000;
                GuestMachine::advance_clock_to(&mut m, t).unwrap();
            }
            assert!(m.mac.refused.is_empty(), "File > Save As had refusals: {:?}", m.mac.refused);
            assert!(GuestMachine::ui(&m).open_requests().is_empty(), "File > Save As must not raise an error alert");
            let vol = GuestMachine::volume(&mut m);
            let hd = vol.volume("Macintosh HD").unwrap();
            let children: Vec<_> = vol.children(hd).to_vec();
            let mut saved_psd = false;
            for id in children {
                let n = vol.get(id);
                let flen = vol.file_len(id).unwrap_or(0);
                if n.name.contains("Untitled") || n.name.ends_with(".psd") {
                    let magic = vol.file_data(id).map(|d| &d[..d.len().min(8)]);
                    eprintln!("  SAVED FILE ON HD: {:?} len={flen} type={:?} creator={:?} magic={magic:?}", n.name, core::str::from_utf8(&n.finfo.file_type), core::str::from_utf8(&n.finfo.creator));
                    assert_eq!(&n.finfo.file_type, b"8BPS", "saved file type must be 8BPS");
                    assert_eq!(&n.finfo.creator, b"8BIM", "saved file creator must be 8BIM");
                    assert_eq!(magic, Some(&b"8BPS\x00\x01\x00\x00"[..]), "saved PSD must begin with 8BPS v1 header");
                    assert!(flen > 10_000, "saved PSD must contain real image data (got {flen} bytes)");
                    saved_psd = true;
                }
            }
            assert!(saved_psd, "File > Save As must write a valid Untitled-1 8BPS file to Macintosh HD");
        }
    } else {
        panic!("File > New Return did not open Untitled-1 document window");
    }
}
