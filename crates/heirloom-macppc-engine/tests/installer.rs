//! The Warcraft III Installer (the owner's disc, corpus test input, never committed: /data/build/corpus-bin/mac/warcraft3/roc-fs,
//! docs/CORPUS.md) run on the host the way Heirloom runs it: the disc is a read-only volume "Warcraft III" whose files' data forks
//! are STREAMS read on demand from the MacBinary files, and "Macintosh HD" is a scratch, writable volume the install goes to.
//! A missing corpus SKIPS loudly. The CD key is typed at run time by the owner; a host test types a format-valid DUMMY that
//! exists only here (never shipped, never written anywhere else).
use heirloom_common::{GuestMachine, Step};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

/// The owner's disc as MacBinary files: `HEIRLOOM_WC3_DISC`, else the corpus lane's place (docs/CORPUS.md).
fn disc() -> String { std::env::var("HEIRLOOM_WC3_DISC").unwrap_or_else(|_| String::from("/data/build/corpus-bin/mac/warcraft3/roc-fs")) }

/// The disc's data forks, read from the MacBinary files by locator (the host path): a data fork starts after the 128-byte header.
struct DiscStreams;
impl heirloom_volume::StreamSource for DiscStreams {
    fn read_at(&self, locator: &str, offset: u64, dst: &mut [u8]) -> Option<usize> {
        let f = std::fs::File::open(locator).ok()?;
        std::os::unix::fs::FileExt::read_at(&f, dst, 128 + offset).ok()
    }
}

/// One MacBinary file's header facts: (name, type, creator, finder flags, data length, resource fork bytes).
fn macbinary(path: &std::path::Path) -> Option<(String, [u8; 4], [u8; 4], u16, u64, Vec<u8>)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; 128];
    f.read_exact(&mut h).ok()?;
    let n = h[1] as usize;
    if n == 0 || n > 63 { return None; }
    let name: String = h[2..2 + n].iter().map(|&c| char::from(c)).collect();
    let dlen = u32::from_be_bytes(h[83..87].try_into().unwrap()) as u64;
    let rlen = u32::from_be_bytes(h[87..91].try_into().unwrap()) as usize;
    let mut rsrc = vec![0u8; rlen];
    if rlen > 0 {
        f.seek(SeekFrom::Start(128 + dlen.div_ceil(128) * 128)).ok()?;
        f.read_exact(&mut rsrc).ok()?;
    }
    let flags = u16::from_be_bytes([h[73], h[101]]);
    Some((name, h[65..69].try_into().unwrap(), h[69..73].try_into().unwrap(), flags, dlen, rsrc))
}

/// A machine with the disc mounted (every file under its folder, `_` in the corpus name = a folder step) and a blank scratch disk.
fn staged() -> Option<(MacPpc<RecordingGuestUi>, Vec<u8>)> {
    let app = std::fs::read(format!("{}/Warcraft III Installer.bin", disc())).ok().or_else(|| { eprintln!("SKIPPED: the Warcraft III disc corpus ({}) is absent", disc()); None })?;
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), &app).unwrap();
    let vol = GuestMachine::volume(&mut m);
    vol.enable_streams();
    vol.set_stream_source(Box::new(DiscStreams));
    let cd = vol.add_volume("Warcraft III", false); // a CD: locked
    let mut files: Vec<_> = std::fs::read_dir(disc()).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for p in files {
        let corpus = p.file_name().unwrap().to_string_lossy().into_owned();
        let Some(stem) = corpus.strip_suffix(".bin") else { continue };
        let Some((name, ty, cr, flags, dlen, rsrc)) = macbinary(&p) else { continue };
        // the folders are the corpus name BEFORE the file's own (header) name, `_` between them: a name may hold `_` itself
        let Some(parts) = stem.strip_suffix(name.as_str()).map(|d| d.strip_suffix('_').unwrap_or(d)).map(|d| d.split('_').filter(|c| !c.is_empty()).collect::<Vec<&str>>()) else {
            eprintln!("SKIPPED {corpus}: its header name {name:?} is not the end of its corpus name");
            continue;
        };
        // Finder housekeeping (desktop databases, icons, .DS_Store) and the installer itself (start_file puts it there) are not staged
        if name.starts_with('.') || name.starts_with("Icon") || name.starts_with("Desktop D") || parts.first() == Some(&"TheVolumeSettingsFolder") || stem == "Warcraft III Installer" { continue; }
        let dir = vol.mkdirs(cd, parts.iter().copied()).unwrap();
        let id = vol.put_stream(dir, &name, dlen, p.to_str().unwrap()).unwrap();
        let n = vol.get_mut(id);
        n.finfo.file_type = ty;
        n.finfo.creator = cr;
        n.finfo.flags = flags;
        n.rsrc = rsrc;
    }
    let hd = vol.add_volume("Macintosh HD", true);
    vol.set_capacity(hd, 4 << 30).unwrap(); // a 4 GB scratch disk: room for the 450 MB install
    GuestMachine::set_home(&mut m, &["Warcraft III"]).unwrap();
    Some((m, app))
}

/// Every disc file the copy phase opens is staged at its real place: names hold `_` too ("c_left.jpg"), so a corpus file's folders are
/// never guessed from `_` inside its own name. The 2026-11-01 owner-key run stopped copying at ":Support:ReadMe:Images:c_left.jpg"
/// (fnfErr: it had been staged as Support:ReadMe:Images:c:c_left.jpg), leaving War3.mpq, the maps and every later file pre-sized
/// and all zero. Corpus-skipping, not ignored.
#[test]
fn a_disc_file_whose_name_holds_an_underscore_is_staged_at_its_real_place() {
    let Some((mut m, _)) = staged() else { return };
    let vol = GuestMachine::volume(&mut m);
    let cd = vol.volume("Warcraft III").unwrap();
    let mut at = cd;
    for c in ["Support", "ReadMe", "Images", "c_left.jpg"] { at = vol.find_child(at, c).unwrap_or_else(|| panic!("no {c} on the disc")); }
    assert!(vol.is_file(at));
    assert!(vol.find_child(vol.find_child(vol.find_child(vol.find_child(cd, "Support").unwrap(), "ReadMe").unwrap(), "Images").unwrap(), "c").is_none(),
        "no folder made out of a file name's underscore");
}

/// The window's picture as the host shows it now: every pixel frame of every plane, oldest first, composed (PPM).
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

/// The newest log lines the harness keeps in the machine between drains (what the diagnostics print at the end).
const LOG_TAIL: usize = 2000;

/// Every log line since the last drain, appended to `INST_LOG_FILE` when set (the file then holds the log; the machine keeps none), with the
/// polling noise left out; without a file the machine keeps only the newest [`LOG_TAIL`] lines. The machine's log is NOT a ring (a host drains it every slice, as Heirloom.app does): the Installer writes ~70 000
/// lines per 5M-instruction slice in its polling loops, so a harness that put the lines back held 39 million of them (3.5 GiB) after 558
/// slices - the 2026-10-30 owner-key run's OOM at 8 G and 20 G (measured 2026-11-01 with [`probe`]).
fn drain(m: &mut MacPpc<RecordingGuestUi>) {
    let mut lines = core::mem::take(&mut m.mac.log);
    let Ok(path) = std::env::var("INST_LOG_FILE") else {
        lines.drain(..lines.len().saturating_sub(LOG_TAIL));
        m.mac.log = lines;
        return;
    };
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    // the owner's key never reaches a file: each of its five parts is blanked wherever it appears
    let key: String = std::env::var("INST_CDKEY").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    let parts: Vec<&str> = if key.len() == 26 { vec![&key[0..6], &key[6..10], &key[10..16], &key[16..20], &key[20..26]] } else { vec![] };
    for mut l in lines {
        for p in &parts { l = l.replace(p, "<key>"); }
        if ["PtInRect", "BlockMove", "IsDialogEvent", "null event", "GetMHandle 130", "DialogSelect -> FALSE", "GetMouse", "Button ->", "FrontWindow", "would idle"].iter().any(|n| l.contains(n)) { continue }
        writeln!(f, "{l}").unwrap();
    }
}

/// This process's resident set, MiB (Linux /proc/self/statm: pages).
fn rss_mib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 4096 >> 20
}

/// What the host harness holds, every `INST_PROBE_EVERY` slices: `[WC3-PROBE]` with RSS and each retained store (UI call history and
/// the pixels in it, retained plane frames, queued audio, the log). Over `INST_RSS_MAX_MB` the run stops BY NAME (`[WC3-PROBE
/// RSS-CEILING]`) instead of being OOM-killed with no output.
fn probe(m: &MacPpc<RecordingGuestUi>, slice: u64) -> bool {
    let every: u64 = std::env::var("INST_PROBE_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ceiling: u64 = std::env::var("INST_RSS_MAX_MB").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let rss = rss_mib();
    let over = ceiling > 0 && rss > ceiling;
    if (every > 0 && slice % every == 0) || over {
        let ui = GuestMachine::ui(m);
        let px = |p: &heirloom_ui_abi::Plane| p.ops.iter().map(|o| if let heirloom_ui_abi::DrawOp::Pixels { rgb, .. } = o { rgb.len() } else { 0 }).sum::<usize>();
        let call_px: usize = ui.calls.iter().map(|c| if let heirloom_ui_abi::UiCall::PresentPlane(_, p) = c { px(p) } else { 0 }).sum();
        let frames: usize = ui.plane_frames.values().map(Vec::len).sum();
        let frame_px: usize = ui.plane_frames.values().flatten().map(px).sum();
        let audio: usize = ui.audio.values().map(|a| a.queued.len()).sum();
        eprintln!("[WC3-PROBE] slice={slice} rss_mib={rss} calls={} call_px={call_px} plane_frames={frames} frame_px={frame_px} audio_bytes={audio} log={}",
            ui.calls.len(), m.mac.log.len());
    }
    if over { eprintln!("[WC3-PROBE RSS-CEILING] rss_mib={rss} > {ceiling} at slice {slice}"); }
    !over
}

/// Run `slices` host slices of 5M instructions / 20 ms.
fn run(m: &mut MacPpc<RecordingGuestUi>, t: &mut u64, slices: u32) -> bool {
    for i in 0..slices {
        if !probe(m, *t / 20_000) { return false }
        drain(m);
        let (st, _) = GuestMachine::step(m, 5_000_000);
        if matches!(st, Step::Fault(_) | Step::Exited) {
            eprintln!("slice {i}: {st:?} {:?}", m.fault());
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

/// Drive the running Installer with `;`-separated actions: `run:N` (slices), `click:X,Y` (window coordinates), `button:LABEL` (a native
/// button's activation), `field:N=TEXT` (the Nth text field committed with TEXT), `controls:` (print them), `shot:FILE.ppm`. False when the
/// program stopped (fault or exit).
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
            // a native button by its label: the host's control gets the user's click
            "button" => {
                let found = GuestMachine::ui(m).find_control(|c| c.label == arg && matches!(c.kind, heirloom_ui_abi::ControlKind::Button));
                let (w, c, _) = found.unwrap_or_else(|| panic!("no live button {arg:?}"));
                GuestMachine::ui_mut(m).push_intent(w, c, heirloom_ui_abi::Intent::Activate).unwrap();
            }
            // `field:N=TEXT`: the Nth text field (creation order) is committed with TEXT, as a user typing into it and leaving it would
            "field" => {
                let (n, text) = arg.split_once('=').unwrap();
                let n: usize = n.parse().unwrap();
                let ui = GuestMachine::ui(m);
                let (id, (w, _)) = ui.controls.iter().filter(|(_, (_, c))| matches!(c.kind, heirloom_ui_abi::ControlKind::Field { .. })).nth(n).unwrap_or_else(|| panic!("no field {n}"));
                // a field made once has generation 1 (its token's first incarnation); push_intent refuses a stale one by name
                let (w, c) = (*w, heirloom_ui_abi::UiObjectRef { id: *id, generation: 1 });
                GuestMachine::ui_mut(m).push_intent(w, c, heirloom_ui_abi::Intent::CommitText(text.into())).unwrap();
            }
            "controls" => for (id, (_, c)) in &GuestMachine::ui(m).controls { eprintln!("CONTROL {id} {:?} {:?} {:?}", c.kind, c.label, c.rect); },
            other => panic!("action verb {other}"),
        }
    }
    true
}

/// The clicks from the Installer's start to its CD-key dialog: "Install Warcraft III" on the main menu, Install (to Macintosh HD), Agree.
const TO_THE_KEY: &str = "run:150;click:150,222;run:100;button:Install;run:150;button:Agree;run:150";

fn started() -> Option<(MacPpc<RecordingGuestUi>, u64)> {
    let (mut m, app) = staged()?;
    GuestMachine::ui_mut(&mut m).auto_cancel_requests = false;
    GuestMachine::start_file(&mut m, &app, "Warcraft III Installer").unwrap();
    Some((m, 0))
}

fn label_shown(m: &MacPpc<RecordingGuestUi>, needle: &str) -> bool {
    GuestMachine::ui(m).controls.values().any(|(_, c)| matches!(&c.kind, heirloom_ui_abi::ControlKind::Label { text, .. } if text.contains(needle)))
}

/// The REAL Installer runs on the host to its CD-key dialog (Name + the five fields of the 26-character key), and a FORMAT-valid dummy key
/// (right length and alphabet, typed into the real fields, never a real key) is refused by the Installer's own check with its "Invalid
/// CD-Key" alert. A key that passes that check is the owner's to type at run time (`installer_installs_with_the_owners_key`).
#[test]
fn the_installer_reaches_its_cd_key_dialog_and_refuses_a_dummy_key() {
    let Some((mut m, mut t)) = started() else { return };
    assert!(act(&mut m, &mut t, TO_THE_KEY), "{:?}", m.fault());
    assert!(label_shown(&m, "26 character CD-key"), "the CD-key dialog is up");
    let fields = GuestMachine::ui(&m).controls.values().filter(|(_, c)| matches!(c.kind, heirloom_ui_abi::ControlKind::Field { .. })).count();
    assert_eq!(fields, 6, "Name and the five key fields");
    assert!(act(&mut m, &mut t, "field:0=Heirloom Test;run:5;field:1=BCDEFG;run:5;field:2=HJKM;run:5;field:3=NPRTVW;run:5;field:4=XYZ2;run:5;field:5=46789B;run:20;button:OK;run:100"), "{:?}", m.fault());
    let asked: Vec<String> = GuestMachine::ui(&m).requests.values().map(|r| format!("{r:?}")).collect();
    assert!(asked.iter().any(|r| r.contains("Invalid CD-Key")), "the Installer's own check refused the dummy: {asked:?}");
    // the harness holds a bounded log however long the Installer runs (the owner-key run's OOM: millions of lines put back)
    assert!(m.mac.log.len() <= LOG_TAIL, "the harness holds {} log lines", m.mac.log.len());
}

/// The whole install with the OWNER's CD key, typed at run time: `INST_CDKEY=<26 characters> INST_NAME=<name> cargo test --release --features
/// fixtures --test installer owners_key -- --ignored --nocapture`. The key is never stored, logged (drain() redacts it) or committed. Proves the
/// install writes War3.mpq on the scratch disk with font\font.gid and font\font.clh in it (what Warcraft III checks).
#[test]
#[ignore]
fn installer_installs_with_the_owners_key() {
    let Ok(key) = std::env::var("INST_CDKEY") else { eprintln!("SKIPPED: INST_CDKEY (the owner's key, typed at run time) is not set"); return };
    let key: String = key.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    assert_eq!(key.len(), 26, "a Warcraft III key has 26 characters");
    let name = std::env::var("INST_NAME").unwrap_or_else(|_| "Owner".into());
    let Some((mut m, mut t)) = started() else { return };
    assert!(act(&mut m, &mut t, TO_THE_KEY), "{:?}", m.fault());
    let parts = [&key[0..6], &key[6..10], &key[10..16], &key[16..20], &key[20..26]];
    let typed = format!("field:0={name};run:5;field:1={};run:5;field:2={};run:5;field:3={};run:5;field:4={};run:5;field:5={};run:20;button:OK", parts[0], parts[1], parts[2], parts[3], parts[4]);
    let ok = act(&mut m, &mut t, &typed);
    let slices: u32 = std::env::var("INST_SLICES").ok().and_then(|s| s.parse().ok()).unwrap_or(3000);
    let ok = ok && act(&mut m, &mut t, &format!("run:{slices}"));
    drain(&mut m);
    eprintln!("stopped: {ok} {:?}", m.fault());
    let mut refused = m.mac.refused.clone();
    refused.sort();
    refused.dedup();
    eprintln!("REFUSED {refused:?}");
    let vol = GuestMachine::volume(&mut m);
    let hd = vol.volume("Macintosh HD").unwrap();
    let mut stack = vec![(hd, String::from("Macintosh HD"))];
    let mut mpq = None;
    while let Some((d, path)) = stack.pop() {
        for &c in vol.children(d) {
            let p = format!("{path}:{}", vol.name(c));
            eprintln!("INSTALLED {p} {}", vol.file_len(c).unwrap_or(0));
            if vol.is_dir(c) { stack.push((c, p)); } else if vol.name(c).eq_ignore_ascii_case("War3.mpq") { mpq = vol.file_data(c).map(<[u8]>::to_vec); }
        }
    }
    if let Ok(dir) = std::env::var("INST_EXPORT_DIR") { export(vol, hd, std::path::Path::new(&dir)); }
    let mpq = mpq.expect("the install wrote War3.mpq on Macintosh HD");
    for f in ["font\\font.gid", "font\\font.clh"] { assert!(mpq_has(&mpq, f), "{f} is in the installed War3.mpq"); }
}

/// The installed tree, written out for staging (owner data: the corpus lane, never committed): each file's data fork at its Mac path
/// (`:` -> `/`), its resource fork beside it as `<name>.rsrc` when it has one, and `manifest.tsv` (path, type, creator, finder flags,
/// data bytes, resource bytes) for the Finder info a plain file cannot carry.
fn export(vol: &heirloom_volume::Tree, top: heirloom_volume::NodeId, dir: &std::path::Path) {
    let mut manifest = String::from("# path\ttype\tcreator\tflags\tdata\trsrc\n");
    let mut stack = vec![(top, dir.to_path_buf(), String::new())];
    while let Some((d, host, rel)) = stack.pop() {
        std::fs::create_dir_all(&host).unwrap();
        for &c in vol.children(d) {
            let name = vol.name(c).replace('/', ":");
            let (h, r) = (host.join(&name), if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") });
            if vol.is_dir(c) { stack.push((c, h, r)); continue }
            let n = vol.get(c);
            let data = vol.file_data(c).unwrap_or(&[]);
            std::fs::write(&h, data).unwrap();
            if !n.rsrc.is_empty() { std::fs::write(host.join(format!("{name}.rsrc")), &n.rsrc).unwrap(); }
            let fourcc = |b: [u8; 4]| b.iter().map(|&x| if x.is_ascii_graphic() || x == b' ' { char::from(x) } else { '?' }).collect::<String>();
            manifest += &format!("{r}\t{}\t{}\t{:#06x}\t{}\t{}\n", fourcc(n.finfo.file_type), fourcc(n.finfo.creator), n.finfo.flags, data.len(), n.rsrc.len());
        }
    }
    std::fs::write(dir.join("manifest.tsv"), manifest).unwrap();
    eprintln!("EXPORTED the installed tree -> {}", dir.display());
}

/// Storm's hash of `name` for `kind` (0 index, 1 name A, 2 name B, 3 key) over its crypt table: the MPQ format's public lookup.
fn mpq_hash(name: &str, kind: u32, table: &[u32]) -> u32 {
    let (mut s1, mut s2) = (0x7FED_7FEDu32, 0xEEEE_EEEEu32);
    for ch in name.to_ascii_uppercase().bytes() {
        s1 = table[(kind * 0x100 + u32::from(ch)) as usize] ^ s1.wrapping_add(s2);
        s2 = u32::from(ch).wrapping_add(s1).wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
    }
    s1
}
fn crypt_table() -> Vec<u32> {
    let mut t = vec![0u32; 0x500];
    let mut seed = 0x0010_0001u32;
    for i in 0..0x100 {
        let mut idx = i;
        for _ in 0..5 {
            seed = (seed * 125 + 3) % 0x2A_AAAB;
            let a = (seed & 0xFFFF) << 16;
            seed = (seed * 125 + 3) % 0x2A_AAAB;
            t[idx] = a | (seed & 0xFFFF);
            idx += 0x100;
        }
    }
    t
}
/// Is `name` in the MPQ archive `a` (header at 0, hash table decrypted with its fixed key)?
fn mpq_has(a: &[u8], name: &str) -> bool {
    let t = crypt_table();
    let u = |o: usize| u32::from_le_bytes(a[o..o + 4].try_into().unwrap());
    assert_eq!(&a[..4], b"MPQ\x1a");
    let (ht, hc) = (u(16) as usize, u(24) as usize);
    let mut key = mpq_hash("(hash table)", 3, &t);
    let mut s2 = 0xEEEE_EEEEu32;
    let mut words = Vec::with_capacity(hc * 4);
    for i in 0..hc * 4 {
        s2 = s2.wrapping_add(t[(0x400 + (key & 0xFF)) as usize]);
        let v = u(ht + 4 * i) ^ key.wrapping_add(s2);
        key = ((!key << 0x15).wrapping_add(0x1111_1111)) | (key >> 0x0B);
        s2 = v.wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
        words.push(v);
    }
    let (a1, b1) = (mpq_hash(name, 1, &t), mpq_hash(name, 2, &t));
    let mut i = mpq_hash(name, 0, &t) as usize % hc;
    for _ in 0..hc {
        let (ha, hb, blk) = (words[4 * i], words[4 * i + 1], words[4 * i + 3]);
        if ha == a1 && hb == b1 && blk < 0xFFFF_FFFE { return true; }
        if blk == 0xFFFF_FFFF { return false; }
        i = (i + 1) % hc;
    }
    false
}

/// Diagnostic, not a gate: how far the Installer gets today, its last log lines and UI calls. `INST_ACTIONS` drives it ([`act`]).
/// `cargo test --features fixtures --test installer host_run -- --ignored --nocapture`.
#[test]
#[ignore]
fn installer_host_run() {
    let Some((mut m, mut t)) = started() else { return };
    let actions = std::env::var("INST_ACTIONS").unwrap_or_else(|_| "run:200".into());
    act(&mut m, &mut t, &actions);
    let log = &m.mac.log;
    let keep: usize = std::env::var("INST_LOG_LINES").ok().and_then(|s| s.parse().ok()).unwrap_or(80);
    for l in log.iter().skip(log.len().saturating_sub(keep)) { eprintln!("LOG {l}"); }
    let mut refused = m.mac.refused.clone();
    refused.dedup();
    eprintln!("REFUSED {refused:?}");
    for c in GuestMachine::ui(&m).calls.iter().rev().take(15) { eprintln!("UI {}", format!("{c:?}").chars().take(300).collect::<String>()); }
}

/// The MPQ lookup the owner-key test relies on, checked on the disc's War3.mpq: a file the game reads from it is found, the two the Installer
/// adds are not (the reason the game needs a real install). Ignored: it reads the 401 MB archive.
#[test]
#[ignore]
fn mpq_lookup_finds_what_the_discs_war3_mpq_holds_and_not_the_installed_fonts() {
    let Ok(f) = std::fs::read(format!("{}/Support_War3.mpq.bin", disc())) else { eprintln!("SKIPPED: no disc"); return };
    let len = u32::from_be_bytes(f[83..87].try_into().unwrap()) as usize;
    let a = &f[128..128 + len];
    assert!(mpq_has(a, "UI\\MacStrings.txt"));
    assert!(!mpq_has(a, "font\\font.gid") && !mpq_has(a, "font\\font.clh"), "NEGATIVE CONTROL: the disc's archive lacks the installed fonts");
}
