//! Real Symbian apps and bitmaps (never committed). Opt-in: `cargo test -- --ignored`, with `HEIRLOOM_SYMICON_CORPUS` naming the
//! corpus root (default /data/build/corpus-symbian: `drive/<app>/` install trees and the SDK's `.mbm` files). Facts asserted
//! were measured on those files on 2026-11-03.
use heirloom_fmt_symicon::{app_icon, mbm, Picture};

fn root() -> String {
    std::env::var("HEIRLOOM_SYMICON_CORPUS").unwrap_or_else(|_| "/data/build/corpus-symbian".into())
}

fn tree_icon(app: &str, exe: &str) -> Picture {
    let tree = format!("{}/drive/{app}", root());
    assert!(std::path::Path::new(&tree).is_dir(), "{tree} missing (set HEIRLOOM_SYMICON_CORPUS)");
    let read = |rel: &str| std::fs::read(format!("{tree}/{rel}")).ok();
    let list = |dir: &str| std::fs::read_dir(format!("{tree}/{dir}")).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    app_icon(&read, &list, exe).unwrap_or_else(|e| panic!("{app}: {e}"))
}

#[test]
#[ignore = "reads the owner's Symbian corpus (HEIRLOOM_SYMICON_CORPUS); run with --ignored"]
fn the_file_browser_and_hero_of_sparta_show_their_svg_t_icons() {
    for (app, exe) in [("nokiafilebrowser", "sys/bin/filebrowser.exe"), ("hero-of-sparta", "sys/bin/heroofsparta.exe")] {
        let Picture::Svg { text, source } = tree_icon(app, exe) else { panic!("{app}: expected a vector icon") };
        eprintln!("{app}: {source}, {} bytes: {}", text.len(), &text[..text.len().min(300)]);
        assert_eq!(source, "mif-svgb");
        assert!(text.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""), "{app}");
        assert!(text.contains("<path") || text.contains("<rect") || text.contains("<circle"), "{app}: no shape");
        assert_eq!(text.matches("<g").count(), text.matches("</g>").count(), "{app}: balanced groups");
    }
}

#[test]
#[ignore = "reads every .mbm under HEIRLOOM_SYMICON_CORPUS; run with --ignored"]
fn the_sdks_bitmaps_decode_or_name_why() {
    let mut stack = vec![std::path::PathBuf::from(root())];
    let (mut ok, mut refused) = (0, Vec::new());
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mbm")) {
                match mbm::bitmaps(&std::fs::read(&p).unwrap()) {
                    Ok(b) if !b.is_empty() => ok += 1,
                    Ok(_) => refused.push(format!("{}: empty", p.display())),
                    Err(e) => refused.push(format!("{}: {e}", p.display())),
                }
            }
        }
    }
    eprintln!("{ok} multi-bitmap files read, {} refused", refused.len());
    for r in refused.iter().take(10) {
        eprintln!("  {r}");
    }
    assert!(ok >= 60, "the SDK ships ~87 .mbm files, read {ok}");
}
