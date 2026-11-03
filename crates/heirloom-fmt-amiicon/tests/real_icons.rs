//! Real Workbench icons from the owner's corpus (never committed). Opt-in: `cargo test -- --ignored`, with
//! `HEIRLOOM_AMIICON_CORPUS` naming the corpus root (default: the corpus lane's /data/build/corpus-bin). Facts asserted were
//! measured on those files on 2026-11-03.
use heirloom_fmt_amiicon::app_icon;
use std::collections::BTreeMap;

fn root() -> String {
    std::env::var("HEIRLOOM_AMIICON_CORPUS").unwrap_or_else(|_| "/data/build/corpus-bin".into())
}

fn read(rel: &str) -> Vec<u8> {
    let p = format!("{}/{rel}", root());
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e} (set HEIRLOOM_AMIICON_CORPUS)"))
}

#[test]
#[ignore = "reads the owner's Amiga corpus (HEIRLOOM_AMIICON_CORPUS); run with --ignored on a host that has it"]
fn directory_opus_5_and_snoopdos_show_their_own_icons() {
    let dopus = app_icon(&read("dopus5/DOpus5/DirectoryOpus.info")).unwrap();
    eprintln!("DirectoryOpus.info: {}x{} {}", dopus.w, dopus.h, dopus.source);
    assert!(dopus.source.starts_with("coloricon"), "DOpus 5 ships an OS3.5 colour icon: {}", dopus.source);
    let snoop = app_icon(&read("aminet-snoopdos/ex/SnoopDos.info")).unwrap();
    eprintln!("SnoopDos.info: {}x{} {}", snoop.w, snoop.h, snoop.source);
    assert_eq!((snoop.w, snoop.h), (54, 22), "the classic image (its Gadget is one row taller)");
    assert!(snoop.source.starts_with("planar"), "{}", snoop.source);
    for img in [&dopus, &snoop] {
        assert!(img.rgba.chunks(4).any(|p| p[3] == 0) && img.rgba.chunks(4).any(|p| p[3] == 0xFF), "a shape on a transparent ground");
    }
}

#[test]
#[ignore = "reads every .info under HEIRLOOM_AMIICON_CORPUS; run with --ignored"]
fn every_icon_in_the_corpus_decodes_or_names_why() {
    let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
    let mut stack = vec![std::path::PathBuf::from(root())];
    let (mut n, mut failed) = (0, Vec::new());
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "info") {
                let Ok(b) = std::fs::read(&p) else { continue };
                if b.len() < 2 || !(b.starts_with(&[0xE3, 0x10]) || b.starts_with(&[0x89, b'P'])) {
                    continue; // not an icon at all (an unrelated file named .info)
                }
                n += 1;
                match app_icon(&b) {
                    Ok(img) => *by_source.entry(img.source.into()).or_default() += 1,
                    Err(e) => failed.push(format!("{}: {e}", p.display())),
                }
            }
        }
    }
    eprintln!("{n} icons: {by_source:?}; {} refused", failed.len());
    for f in failed.iter().take(20) {
        eprintln!("  {f}");
    }
    assert!(n > 800, "the corpus holds ~900 icons, read {n}");
    assert!(failed.len() * 50 < n, "more than 2% refused: {}", failed.len());
    assert!(failed.iter().all(|f| f.contains("deeper than the Workbench pens")), "only deep planar icons (screen palette unknown) may be refused");
}
