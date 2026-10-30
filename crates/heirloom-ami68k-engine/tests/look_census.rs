//! What still reaches the host as raw guest pixels: a census of the plane route on real programs (skipped when the corpus is absent).
//! Per window it counts the named ops by kind and the holes (`DrawOp::Pixels`) by count and area against the window's area, from the
//! LAST plane frame of each id (what the host shows at the idle point). `LOOK_CENSUS=1` prints the table and the biggest holes.
use heirloom_ami68k_engine::Amiga;
use heirloom_ui_abi::{DrawOp, IconRole, RecordingGuestUi, UiCall};
use std::collections::BTreeMap;

const ROOT: &str = "/data/build/corpus-bin";

fn load(a: &mut Amiga<RecordingGuestUi>, prefix: &str, dir: &std::path::Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = format!("{prefix}{}", e.file_name().to_string_lossy());
        if e.file_type().unwrap().is_dir() { load(a, &format!("{path}/"), &e.path()); }
        else if let Ok(d) = std::fs::read(e.path()) { a.fs.vol.put_file(&path, d).unwrap(); }
    }
}

/// Run a program on the plane route, as shipped (the Gute Form screen face), stepping the virtual clock while every task sleeps.
fn run(rel: &str, dev: Option<(&str, &str)>, seconds: u64, prep: impl FnOnce(&mut RecordingGuestUi)) -> Option<Amiga<RecordingGuestUi>> {
    let file = std::fs::read(format!("{ROOT}/{rel}")).ok().or_else(|| {
        // A gate that must prove the census sets HEIRLOOM_REQUIRE_CORPUS: an absent corpus is then a failure, not a silent pass.
        assert!(std::env::var_os("HEIRLOOM_REQUIRE_CORPUS").is_none(), "HEIRLOOM_REQUIRE_CORPUS is set but {ROOT}/{rel} is absent");
        eprintln!("skipped: corpus not present"); None
    })?;
    let mut ui = RecordingGuestUi::new();
    prep(&mut ui);
    let mut a = Amiga::new(ui).unwrap();
    a.gfx.present.set_plane_route(true);
    if let Some((dev, dir)) = dev {
        a.fs.vol.add_device(dev, false);
        load(&mut a, &format!("{dev}:"), std::path::Path::new(&format!("{ROOT}/{dir}")));
        a.fs.vol.assign_add("LIBS", &format!("{dev}:Libs"));
        a.fs.vol.assign("PROGDIR", &format!("{dev}:"));
    }
    a.start_hunk(&file).unwrap();
    let mut insns = 0u64;
    while a.clock_us < seconds * 1_000_000 && insns < 200_000_000 {
        let (step, st) = a.step(2_000_000);
        insns += st.insns;
        match step {
            heirloom_ami68k_engine::Step::Blocked => match a.next_timer_deadline() { Some(t) => a.advance_clock_to(t).unwrap(), None => break },
            heirloom_ami68k_engine::Step::BudgetSpent => {}
            other => panic!("{rel} stopped: {other:?}\nrefused: {:?}", a.refused),
        }
    }
    Some(a)
}

/// One window's census: named ops by kind, holes (count, pixel area), the window area, and the holes' rects.
#[derive(Default, Debug)]
struct Win { planes: String, area: i64, kinds: BTreeMap<&'static str, u32>, holes: u32, hole_px: i64, rects: Vec<(i32, i32, i32, i32)>, controls: u32 }

fn kind(op: &DrawOp) -> &'static str {
    match op { DrawOp::Fill { .. } => "fill", DrawOp::Text { .. } => "text", DrawOp::Line { .. } => "line", DrawOp::Bevel { .. } => "bevel", DrawOp::Icon { .. } => "icon", DrawOp::Pixels { .. } => "PIXELS", DrawOp::Swatch { .. } => "swatch" }
}

/// The census of every window, from the recorder's merged plane per (window, plane id): what the host shows at the idle point.
fn census(a: &Amiga<RecordingGuestUi>) -> BTreeMap<u32, Win> {
    let mut wins: BTreeMap<u32, Win> = BTreeMap::new();
    for c in &a.ui.calls {
        if let UiCall::AddControl(w, _) = c { wins.entry(w.id).or_default().controls += 1; }
    }
    for ((win, _), p) in &a.ui.planes {
        let w = wins.entry(*win).or_default();
        w.area += i64::from(p.rect.width) * i64::from(p.rect.height);
        w.planes.push_str(&format!("{}x{} ", p.rect.width, p.rect.height));
        for op in &p.ops {
            *w.kinds.entry(kind(op)).or_default() += 1;
            if let DrawOp::Pixels { rect, .. } = op {
                w.holes += 1;
                w.hole_px += i64::from(rect.width) * i64::from(rect.height);
                w.rects.push((rect.left, rect.top, rect.width, rect.height));
            }
        }
    }
    wins
}

fn report(name: &str, a: &Amiga<RecordingGuestUi>, wins: &BTreeMap<u32, Win>) {
    if std::env::var_os("LOOK_CENSUS").is_none() { return; }
    let mut why: Vec<_> = a.gfx.present.hole_why.iter().collect();
    why.sort_by_key(|(_, (_, px))| -*px);
    for (w, (n, px)) in why { eprintln!("[LOOK-CENSUS] {name} hole-why n={n} px={px}: {w}"); }
    for r in &a.refused { eprintln!("[LOOK-CENSUS] {name} refused: {r}"); }
    if let Ok(w) = std::env::var("LOOK_DUMP") { for ((win, _), p) in &a.ui.planes { if w.parse() == Ok(*win) { for o in p.ops.iter().take(60) { let s = format!("{o:?}"); eprintln!("[LOOK-CENSUS] {name} op: {}", &s[..s.len().min(160)]); } } } }
    if let Ok(g) = std::env::var("LOOK_GREP") { for l in a.log.iter().filter(|l| g.split('|').any(|x| l.contains(x))).take(40) { eprintln!("[LOOK-CENSUS] {name} log: {}", &l[..l.len().min(220)]); } }
    for (id, w) in wins {
        let pct = if w.area > 0 { w.hole_px * 100 / w.area } else { 0 };
        eprintln!("[LOOK-CENSUS] {name} win={id} planes={} area={} controls={} holes={} hole_px={} ({pct}%) ops={:?}", w.planes, w.area, w.controls, w.holes, w.hole_px, w.kinds);
        let mut r = w.rects.clone();
        r.sort_by_key(|(_, _, ww, hh)| -(i64::from(*ww) * i64::from(*hh)));
        for x in r.iter().take(12) { eprintln!("[LOOK-CENSUS]   hole at {},{} {}x{}", x.0, x.1, x.2, x.3); }
    }
}

#[test]
fn directory_opus_census() {
    // The host has icon artwork, as Heirloom.app does (its staged Adwair icons).
    let art: Vec<u32> = (0..32 * 32u32).map(|i| if i % 7 == 0 { 0xFF00_0000 } else { 0xFF44_88DD }).collect();
    let roles = [IconRole::Disk, IconRole::Drawer, IconRole::Device, IconRole::Tool, IconRole::Project, IconRole::Trash, IconRole::Kick];
    let Some(a) = run("dopus5/DOpus5/DirectoryOpus", Some(("DOpus5", "dopus5/DOpus5")), 5, |ui| for r in roles { ui.icon_art.insert(r, (32, 32, art.clone())); }) else { return };
    let wins = census(&a);
    report("dopus5", &a, &wins);
    // The lister is the biggest window: under its clip region its file area is named fills and text, so only its toolbar
    // images stay raw (3% on 2026-10-30; 94% while a fill under a clip region was a hole).
    let lister = wins.values().max_by_key(|w| w.area).expect("DOpus opened windows");
    assert!(lister.hole_px * 100 <= lister.area * 5, "the lister is mostly raw pixels again: {lister:?}");
    assert!(!a.gfx.present.hole_why.keys().any(|k| k.contains("EraseRect")), "{:?}", a.gfx.present.hole_why);
    // The drive strip draws its volumes' default icons itself (a BitMap over the icon's ImageData, blitted): each is the Disk icon,
    // named, and the strip has no raw pixels left.
    let disks = a.ui.planes.values().flat_map(|p| p.ops.iter()).filter(|o| matches!(o, DrawOp::Icon { role: IconRole::Disk, .. })).count();
    assert!(disks >= 3, "the three volume icons are Disk icons: {disks}");
    let strip = wins.values().find(|w| w.kinds.contains_key("icon")).expect("the window with the volume icons");
    assert_eq!(strip.holes, 0, "{strip:?}");
}

#[test]
fn snoopdos_census() {
    let Some(a) = run("aminet-snoopdos/ex/snoopdos", None, 5, |_| {}) else { return };
    let wins = census(&a);
    report("snoopdos", &a, &wins);
    assert!(!wins.is_empty(), "SnoopDos opened a window");
    assert!(wins.values().all(|w| w.holes == 0), "SnoopDos's window is native controls and named ops only: {wins:?}");
}
