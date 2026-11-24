//! Witness and real-app replay tests for Photoshop 7.0 declarative compact-palette bindings (`bindings/macppc/photoshop-7.0-palettes.sexp`):
//! 1. Always-on unit test verifies all 5 shipped palette bindings parse cleanly from DATA and wire-roundtrip byte-for-byte
//!    through `heirloom_bind::wire::{encode_plane_chrome, decode_plane_chrome}`.
//! 2. Corpus replay test (`--ignored`, requires `/data/build/corpus-bin/mac/photoshop7-tryout/installed`) boots real Photoshop 7.0
//!    at `1280x800` on Tier 2 JIT and proves all 5 floating ADM palettes (Tools, Navigator/Info, Color/Swatches/Styles,
//!    History/Actions/Presets, Layers/Channels/Paths) translate into native `DrawOp::Palette` ops when bound, while staying raw
//!    when unbound (negative control).
use heirloom_common::{GuestMachine, Step};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::{DrawOp, PalettePart, RecordingGuestUi, UiCall};

mod ps7_common;
mod wdef_bindings;

#[test]
fn photoshop_7_palette_bindings_parse_and_wire_roundtrip() {
    let bindings = wdef_bindings::load_plane_chrome("macppc");
    assert_eq!(bindings.len(), 5, "expected 5 Photoshop 7 palette bindings, got {}", bindings.len());
    let ids: Vec<&str> = bindings.iter().map(|b| b.id.as_str()).collect();
    for want in [
        "photoshop-7.0-palette-tools",
        "photoshop-7.0-palette-navigator",
        "photoshop-7.0-palette-color",
        "photoshop-7.0-palette-history",
        "photoshop-7.0-palette-layers",
    ] {
        assert!(ids.contains(&want), "missing binding {want:?} in {ids:?}");
    }
    for b in &bindings {
        assert!(!b.pixel_guards.is_empty(), "{} must carry a pixel-mask guard", b.id);
        assert!(!b.palette_rules.is_empty(), "{} must carry palette rules", b.id);
        let wire = heirloom_bind::wire::encode_plane_chrome(b).expect("encode_plane_chrome");
        let decoded = heirloom_bind::wire::decode_plane_chrome(&wire).expect("decode_plane_chrome");
        assert_eq!(&decoded, b, "wire roundtrip mismatch for {}", b.id);
    }
}

struct Files;
impl heirloom_volume::StreamSource for Files {
    fn read_at(&self, locator: &str, offset: u64, dst: &mut [u8]) -> Option<usize> {
        let f = std::fs::File::open(locator).ok()?;
        std::os::unix::fs::FileExt::read_at(&f, dst, offset).ok()
    }
}

fn fourcc(s: &str) -> [u8; 4] { let mut b = [b' '; 4]; for (i, c) in s.bytes().take(4).enumerate() { b[i] = c; } b }

fn boot_ps7(with_palette_bindings: bool) -> Option<MacPpc<RecordingGuestUi>> {
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
    if with_palette_bindings {
        for pc in wdef_bindings::load_plane_chrome("macppc") {
            m.mac.add_plane_chrome_binding(pc);
        }
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
    Some(m)
}

#[test]
#[ignore]
fn photoshop_7_palettes_translate_natively_when_bound_and_stay_raw_when_unbound() {
    // 1. Negative control: unbound -> zero DrawOp::Palette ops.
    let Some(unbound) = boot_ps7(false) else { return };
    let unbound_palettes = GuestMachine::ui(&unbound).calls.iter().filter(|c| match c {
        UiCall::PresentPlane(_, p) => p.ops.iter().any(|o| matches!(o, DrawOp::Palette { .. })),
        _ => false,
    }).count();
    assert_eq!(unbound_palettes, 0, "unbound PS7 must emit 0 DrawOp::Palette ops");

    // 2. Bound -> all 5 floating ADM palettes emit native DrawOp::Palette ops!
    let bound = boot_ps7(true).expect("PS7 installed tree");
    let mut latest_planes = std::collections::BTreeMap::new();
    for c in &GuestMachine::ui(&bound).calls {
        if let UiCall::PresentPlane(w, p) = c {
            latest_planes.insert(w.id, p.ops.clone());
        }
    }
    let mut saw_tool_grid = false;
    let mut saw_color_well = 0usize;
    let mut saw_tabs = std::collections::BTreeSet::new();
    let mut saw_grad_sliders = std::collections::BTreeSet::new();
    let mut saw_footers = 0usize;
    let mut saw_mini_toggles = 0usize;
    for (w, ops) in &latest_planes {
        let pal_ops: Vec<_> = ops.iter().filter_map(|o| match o { DrawOp::Palette { part, .. } => Some(part), _ => None }).collect();
        if !pal_ops.is_empty() {
            eprintln!("WIN {w}: {} total ops, {} palette parts", ops.len(), pal_ops.len());
        }
        for part in pal_ops {
            match part {
                PalettePart::ToolGrid { cols, tools, active } => {
                    assert_eq!(*cols, 2);
                    assert_eq!(tools.len(), 22);
                    assert_eq!(*active, 0, "default active tool on launch must be BoxSelect (index 0)");
                    eprintln!("  ToolGrid: cols={cols} tools={} active={active}", tools.len());
                    saw_tool_grid = true;
                }
                PalettePart::ColorWell { fore, back, .. } => {
                    eprintln!("  ColorWell: fore={fore:#08x} back={back:#08x}");
                    assert_eq!(*fore, 0x0000_0000);
                    assert_eq!(*back, 0x00FF_FFFF);
                    saw_color_well += 1;
                }
                PalettePart::Tabs { labels, active, flyout } => {
                    eprintln!("  Tabs: {labels:?} active={active} flyout={flyout}");
                    assert_eq!(*active, 0);
                    assert!(*flyout);
                    saw_tabs.insert(labels[0].clone());
                }
                PalettePart::GradientSlider { label, permille, .. } => {
                    eprintln!("  GradientSlider: {label} permille={permille}");
                    saw_grad_sliders.insert(label.clone());
                }
                PalettePart::Footer { keys } => {
                    eprintln!("  Footer: {} keys", keys.len());
                    saw_footers += 1;
                }
                PalettePart::MiniToggles { keys, latched } => {
                    eprintln!("  MiniToggles: {} keys latched={latched:#b}", keys.len());
                    saw_mini_toggles += 1;
                }
            }
        }
    }
    assert!(saw_tool_grid, "expected Tools palette ToolGrid");
    assert!(saw_color_well >= 2, "expected Tools + Color palette ColorWell, got {saw_color_well}");
    for tab in ["Navigator", "Color", "History", "Layers"] {
        assert!(saw_tabs.contains(tab), "missing native Tabs for {tab:?}; got {saw_tabs:?}");
    }
    for ch in ["R", "G", "B"] {
        assert!(saw_grad_sliders.contains(ch), "missing GradientSlider {ch:?}; got {saw_grad_sliders:?}");
    }
    assert!(saw_footers >= 3, "expected Navigator + History + Layers Footer parts, got {saw_footers}");
    assert!(saw_mini_toggles >= 3, "expected Tools + Layers MiniToggles parts, got {saw_mini_toggles}");
}
