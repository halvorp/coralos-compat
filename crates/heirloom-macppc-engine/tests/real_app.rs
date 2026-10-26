//! A REAL PowerPC Mac application runs: the Systemless "Toolbox Showcase" (a Universal Interfaces C program built for
//! PowerPC with MPW/CodeWarrior-style startup, 249 imports from InterfaceLib, StdCLib, MathLib and QuickDraw 3D), taken
//! unmodified out of its StuffIt 5 archive (stored, not compressed: the data fork is the PEF, the resource fork precedes it).
//! `SYSTEMLESS_SHOWCASE` names the archive (default under /data/refs/heirloom); a missing file FAILS unless
//! `HEIRLOOM_ALLOW_MISSING_REFS` is set. The expectations pin how far it gets TODAY; when color QuickDraw lands the stop moves
//! on and this test must be updated deliberately.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::census::Census;
use heirloom_macppc_engine::{MacPpc, Step};
use heirloom_fmt_pef::Pef;

fn be32(d: &[u8], at: usize) -> u32 { u32::from_be_bytes(d[at..at + 4].try_into().unwrap()) }

/// (PEF data fork, raw resource fork) cut out of the stored SIT5 archive.
fn showcase() -> Option<(Vec<u8>, Vec<u8>)> {
    let path = std::env::var("SYSTEMLESS_SHOWCASE").unwrap_or_else(|_| "/data/refs/heirloom/systemless/tests/toolbox-showcase/toolbox-showcase.sit".into());
    let d = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) if std::env::var("HEIRLOOM_ALLOW_MISSING_REFS").is_ok() => { eprintln!("SKIPPED (HEIRLOOM_ALLOW_MISSING_REFS set): {path}: {e}"); return None }
        Err(e) => panic!("{path} is not available ({e}); set SYSTEMLESS_SHOWCASE, or HEIRLOOM_ALLOW_MISSING_REFS=1 to skip knowingly"),
    };
    let o = d.windows(8).position(|w| w == b"Joy!peff").expect("a PEF in the archive");
    // container length = the furthest section end (section headers are 28 bytes after the 40-byte container header)
    let n = u16::from_be_bytes([d[o + 32], d[o + 33]]) as usize;
    let end = (0..n).map(|i| { let h = o + 40 + 28 * i; be32(&d, h + 20) + be32(&d, h + 16) }).max().unwrap() as usize;
    // the resource fork ends where the data fork begins: find the start whose map ends exactly there
    let start = (100..2000).find(|&s| { let (off, mo, dl, ml) = (be32(&d, s), be32(&d, s + 4), be32(&d, s + 8), be32(&d, s + 12)); off == 0x100 && off + dl == mo && (mo + ml) as usize == o - s }).expect("the resource fork");
    Some((d[o..o + end].to_vec(), d[start..o].to_vec()))
}

fn load(pef: &[u8], rsrc: Option<&[u8]>) -> (MacPpc<RecordingGuestUi>, heirloom_macppc_engine::cfm::Fragment) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    if let Some(r) = rsrc { m.attach_resources(r).unwrap(); }
    let frag = m.instantiate("Toolbox Showcase", &Pef::parse(pef).unwrap()).unwrap();
    m.start(&frag).unwrap();
    (m, frag)
}

#[test]
fn the_real_application_starts_up_loads_its_menus_and_stops_at_tenew() {
    let Some((pef, rsrc)) = showcase() else { return };
    let (mut m, frag) = load(&pef, Some(&rsrc));
    let c = Census::of(&frag);
    assert_eq!(c.lines.len(), 249);
    for lib in ["StdCLib/exit", "StdCLib/__setjmp", "StdCLib/_IntEnv", "StdCLib/__target_for_exit", "InterfaceLib/GetMenu", "InterfaceLib/BeginUpdate"] {
        assert!(!c.next_needed.contains(&lib.to_string()), "{lib} should be bound");
    }
    assert!(c.next_needed.iter().any(|n| n == "QuickDrawª 3D/Q3Exit") && c.next_needed.iter().any(|n| n == "InterfaceLib/LNew"), "genuinely unserved imports stay named");
    let (step, insns) = m.run_loop(1_000_000);
    let log = m.mac.log.join("\n");
    // startup ran for real: the Init calls, then the application's own MBAR 128 and the MENU resources it names
    let at = |needle: &str| log.find(needle).unwrap_or_else(|| panic!("{needle} missing:\n{log}"));
    assert!(at("InitGraf") < at("InitWindows") && at("InitWindows") < at("GetNewMBar 128: menus [128, 129, 130, 131]") && at("GetNewMBar") < at("GetMenu 132 -> menu 132 \"Options\" 6 items"), "{log}");
    assert!(log.contains("GetMenu 142 -> menu 142 \"Renderer\" 3 items") && log.contains("DrawMenuBar: 4 menus"), "{log}");
    // ...and stops at TENew (TextEdit), by name, with the interpreter's instruction count. It used to stop earlier, at SetPalette:
    // the PowerPC import name of NSetPalette (symbols.rs INTERFACELIB_ALIASES); the colour window and its palette now go through.
    // 2026-10-24: NewGWorld is served now (gworld.rs), so the named unserved import above is LNew. heirloom-mac68k-engine's TextEdit (textedit/) answers TENew, so the run goes on to the next import with no handler: the PowerPC
    // List Manager's LNew (a PowerPC name of the _Pack0 selector; macppc's concern).
    assert!(matches!(&step, Step::Fault(f) if f.starts_with("NEXT-NEEDED LNew")), "{step:?}");
    assert!(m.mac.log.iter().any(|l| l.starts_with("[textedit] TENew")), "TENew ran: {:#?}", m.mac.log);
    assert!(insns > 400, "{insns} PowerPC instructions executed: past the SetPalette that used to stop it");
    // negative control: without the resource fork the same code asks for MBAR 128, finds none, and the application quits
    let (mut m2, _) = load(&pef, None);
    let (step2, _) = m2.run_loop(1_000_000);
    assert_eq!(step2, Step::Exited, "{:?}", m2.fault());
    assert!(m2.mac.log.iter().any(|l| l.contains("GetNewMBar 128: no MBAR")), "{:#?}", m2.mac.log);
    assert!(!m2.mac.log.iter().any(|l| l.contains("GetMenu")));
}
