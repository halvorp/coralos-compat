//! heirloom-appicon - the install step's icon extractor: a guest application's OWN icon as the SVG CoralOS's icon set loads
//! (System/Assets/Icons, rendered by svg.datatype at the toolkit's sizes; icon.library loads nothing else).
//!
//!   heirloom-appicon mac <mac-file>                     a MacBinary / AppleSingle / AppleDouble / raw-fork application
//!   heirloom-appicon ami <program.info>                 the program's Workbench icon
//!   heirloom-appicon sym <install-tree> <program.exe>   a Symbian app: its registration resource names its icon file
//!   ... --svg <out.svg> [--grid <8..256>]
//!
//! A raster becomes pixel squares (heirloom-icon): kept at its own size up to 64 px, else averaged down to --grid px (default
//! 48); an SVG-T icon stays a vector.
//! Prints one line, `[APPICON family=<f> source=<member> size=<w>x<h> colours=<n>]`; exits 1 with `[APPICON NONE] <reason>`
//! when the program carries no icon the decoders read (the caller keeps the generic one).
use heirloom_icon::{to_svg, Picture};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut pos, mut svg, mut grid, mut explicit) = (Vec::new(), None, 48u32, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--svg" => svg = it.next().cloned(),
            "--grid" => {
                grid = it.next().and_then(|g| g.parse().ok()).filter(|g| (8..=256).contains(g)).unwrap_or_else(|| usage());
                explicit = true;
            }
            s if !s.starts_with("--") => pos.push(s.to_string()),
            _ => usage(),
        }
    }
    let read = |p: &str| std::fs::read(p).unwrap_or_else(|e| fail(&format!("{p}: {e}")));
    let pic = match pos.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["mac", f] => heirloom_fmt_macicon::app_icon(&read(f)).map(Picture::Raster).map_err(|e| format!("{f}: {e}")),
        ["ami", f] => heirloom_fmt_amiicon::app_icon(&read(f)).map(Picture::Raster).map_err(|e| format!("{f}: {e}")),
        ["sym", tree, exe] => heirloom_fmt_symicon::app_icon(&|rel: &str| std::fs::read(format!("{tree}/{rel}")).ok(), &|dir: &str| list(tree, dir), exe)
            .map_err(|e| format!("{tree} {exe}: {e}")),
        _ => usage(),
    }
    .unwrap_or_else(|e| fail(&e));
    // A small classic icon (up to 64 px: Amiga planar, Mac 32 px) keeps every pixel; a larger one is averaged down to --grid.
    let grid = match &pic { Picture::Raster(i) if i.w.max(i.h) <= 64 && !explicit => i.w.max(i.h), _ => grid };
    let (text, colours) = to_svg(&pic, grid);
    if let Some(p) = &svg {
        std::fs::write(p, &text).unwrap_or_else(|e| fail(&format!("{p}: {e}")));
    }
    let size = match &pic { Picture::Raster(i) => format!("{}x{}", i.w, i.h), Picture::Svg { .. } => "vector".into() };
    println!("[APPICON family={} source={} size={size} colours={colours}]", pos[0], pic.source());
}

/// The file names in `tree/dir` (a Symbian install tree's directory, for the case-insensitive lookups the platform does).
fn list(tree: &str, dir: &str) -> Vec<String> {
    std::fs::read_dir(format!("{tree}/{dir}")).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect()
}

fn usage() -> ! {
    eprintln!("usage: heirloom-appicon mac <file> | ami <program.info> | sym <install-tree> <program.exe>  [--svg <out.svg>] [--grid <8..256>]");
    exit(2)
}

fn fail(why: &str) -> ! {
    println!("[APPICON NONE] {why}");
    exit(1)
}
