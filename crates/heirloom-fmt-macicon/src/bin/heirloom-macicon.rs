//! heirloom-macicon - the install step's icon extractor: a Mac application file (MacBinary, AppleSingle, AppleDouble or a raw
//! resource fork) -> the icon the Finder shows for it (heirloom_fmt_macicon: BNDL -> FREF 'APPL' -> the icon family, best member).
//!
//!   heirloom-macicon <mac-file> [--svg <out.svg>] [--grid <px>]
//!
//! --svg writes the picture as pixel squares, one `<path>` per colour, on a grid of at most --grid px (default 48; a larger
//! member is area-averaged down to it): CoralOS's icon set is SVG (System/Assets/Icons, rendered by svg.datatype at the sizes
//! the toolkit draws; icon.library loads nothing else), and a raster in an SVG is the form that set takes without a second icon
//! pipeline or a PNG writer here.
//! Prints one line, `[MACICON family=<id> source=<member> size=<w>x<h> grid=<g> colours=<n>]`; exit 1 with the named reason
//! when the file has no application icon (the caller keeps the generic one).
use heirloom_fmt_macicon::IconImage;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut input, mut svg, mut grid) = (None, None, 48u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--svg" => svg = it.next().cloned(),
            "--grid" => grid = it.next().and_then(|g| g.parse().ok()).filter(|&g| (8..=256).contains(&g)).unwrap_or_else(|| usage()),
            s if !s.starts_with("--") && input.is_none() => input = Some(s.to_string()),
            _ => usage(),
        }
    }
    let input = input.unwrap_or_else(|| usage());
    let bytes = std::fs::read(&input).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
    let img = heirloom_fmt_macicon::app_icon(&bytes).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
    let family = family_of(&bytes);
    let g = if img.w > grid { grid } else { img.w };
    let small = if g == img.w { img.clone() } else { area_average(&img, g) };
    let (text, colours) = to_svg(&small);
    if let Some(p) = &svg {
        std::fs::write(p, text).unwrap_or_else(|e| fail(&format!("{p}: {e}")));
    }
    println!("[MACICON family={family} source={} size={}x{} grid={g} colours={colours}]", img.source, img.w, img.h);
}

fn family_of(bytes: &[u8]) -> i16 {
    let o = heirloom_fmt_resfork::container::open(bytes).ok();
    o.and_then(|o| heirloom_fmt_resfork::ResFork::parse(bytes[o.rsrc.clone()].to_vec()).ok().and_then(|f| heirloom_fmt_macicon::app_family_id(&f, o.creator).ok())).unwrap_or(0)
}

fn usage() -> ! {
    eprintln!("usage: heirloom-macicon <mac-file> [--svg <out.svg>] [--grid <8..256>]");
    exit(2)
}

fn fail(why: &str) -> ! {
    eprintln!("[MACICON NONE] {why}");
    exit(1)
}

/// `img` reduced to `g` x `g` by area averaging in premultiplied alpha (each source pixel weighted by its overlap).
fn area_average(img: &IconImage, g: u32) -> IconImage {
    let (w, s) = (img.w as f64, img.w as f64 / g as f64);
    let mut rgba = Vec::with_capacity((g * g * 4) as usize);
    for gy in 0..g {
        for gx in 0..g {
            let (x0, y0) = (gx as f64 * s, gy as f64 * s);
            let mut acc = [0f64; 4];
            let mut y = y0.floor();
            while y < y0 + s {
                let wy = (y + 1.0).min(y0 + s) - y.max(y0);
                let mut x = x0.floor();
                while x < x0 + s {
                    let wx = (x + 1.0).min(x0 + s) - x.max(x0);
                    let i = ((y * w + x) as usize) * 4;
                    let a = img.rgba[i + 3] as f64 / 255.0;
                    for c in 0..3 {
                        acc[c] += img.rgba[i + c] as f64 * a * wx * wy;
                    }
                    acc[3] += a * wx * wy;
                    x += 1.0;
                }
                y += 1.0;
            }
            let area = s * s;
            let a = acc[3] / area;
            for c in 0..3 {
                rgba.push(if acc[3] > 0.0 { (acc[c] / acc[3]).round().min(255.0) as u8 } else { 0 });
            }
            rgba.push((a * 255.0).round().min(255.0) as u8);
        }
    }
    IconImage { w: g, h: g, rgba, source: img.source }
}

/// The picture as pixel squares: one `<path>` per distinct colour (horizontal runs merged), transparent pixels omitted.
fn to_svg(img: &IconImage) -> (String, usize) {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    let mut paths: BTreeMap<[u8; 4], String> = BTreeMap::new();
    for y in 0..img.h {
        let mut x = 0;
        while x < img.w {
            let at = |x: u32| -> [u8; 4] { let i = ((y * img.w + x) * 4) as usize; img.rgba[i..i + 4].try_into().unwrap() };
            let c = at(x);
            let mut n = 1;
            while x + n < img.w && at(x + n) == c {
                n += 1;
            }
            if c[3] != 0 {
                let _ = write!(paths.entry(c).or_default(), "M{x} {y}h{n}v1h-{n}z");
            }
            x += n;
        }
    }
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{0}\" height=\"{1}\" viewBox=\"0 0 {0} {1}\" shape-rendering=\"crispEdges\">\n", img.w, img.h);
    for (c, d) in &paths {
        let op = if c[3] == 255 { String::new() } else { format!(" fill-opacity=\"{:.3}\"", c[3] as f64 / 255.0) };
        let _ = writeln!(s, "<path fill=\"#{:02x}{:02x}{:02x}\"{op} d=\"{d}\"/>", c[0], c[1], c[2]);
    }
    s.push_str("</svg>\n");
    (s, paths.len())
}
