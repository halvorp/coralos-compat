//! A guest application's own icon, whatever family it comes from (heirloom-fmt-macicon, -amiicon, -symicon all return a
//! [`Picture`]), and the one place it becomes SVG - the only form CoralOS's icon set loads (System/Assets/Icons, rendered by
//! svg.datatype at the sizes the toolkit draws).
//!
//! * [`IconImage`]: straight-alpha RGBA8, row-major, with the member(s) it was decoded from.
//! * [`Picture`]: a raster, or an SVG document a vector format already is (Symbian SVG-T).
//! * [`to_svg`]: a raster as pixel squares, one `<path>` per colour (horizontal runs merged), on a square grid of at most
//!   `max_grid` px: a larger picture is area-averaged down to it (premultiplied), a non-square one is centred on the square.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

/// One decoded icon: `w` x `h` straight-alpha RGBA8, row-major, and which member(s) it came from (`"it32+t8mk"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconImage {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub source: &'static str,
}

impl IconImage {
    /// The pixel at (x, y).
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.w + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

/// A guest app's icon as its format gives it.
#[derive(Debug, Clone, PartialEq)]
pub enum Picture {
    Raster(IconImage),
    /// An SVG document (already in the subset svg.datatype draws: shapes, paths, solid and gradient fills), and its source.
    Svg { text: String, source: &'static str },
}

impl Picture {
    pub fn source(&self) -> &'static str {
        match self {
            Picture::Raster(i) => i.source,
            Picture::Svg { source, .. } => source,
        }
    }
}

/// `img` reduced to fit `g` x `g` by area averaging in premultiplied alpha, centred (a non-square picture keeps its aspect).
pub fn fit(img: &IconImage, g: u32) -> IconImage {
    let side = img.w.max(img.h);
    let s = if side > g { side as f64 / g as f64 } else { 1.0 };
    let out = if side > g { g } else { side };
    let (ow, oh) = (((img.w as f64 / s + 0.5) as u32).clamp(1, out), ((img.h as f64 / s + 0.5) as u32).clamp(1, out));
    let (dx, dy) = ((out - ow) / 2, (out - oh) / 2);
    let mut rgba = vec![0u8; (out * out * 4) as usize];
    for gy in 0..oh {
        for gx in 0..ow {
            let (x0, y0) = (gx as f64 * s, gy as f64 * s);
            let mut acc = [0f64; 4];
            let mut y = libm_floor(y0);
            while y < y0 + s && (y as u32) < img.h {
                let wy = (y + 1.0).min(y0 + s) - y.max(y0);
                let mut x = libm_floor(x0);
                while x < x0 + s && (x as u32) < img.w {
                    let wx = (x + 1.0).min(x0 + s) - x.max(x0);
                    let p = img.px(x as u32, y as u32);
                    let a = p[3] as f64 / 255.0;
                    for c in 0..3 {
                        acc[c] += p[c] as f64 * a * wx * wy;
                    }
                    acc[3] += a * wx * wy;
                    x += 1.0;
                }
                y += 1.0;
            }
            let i = (((gy + dy) * out + gx + dx) * 4) as usize;
            for c in 0..3 {
                rgba[i + c] = if acc[3] > 0.0 { round_u8(acc[c] / acc[3]) } else { 0 };
            }
            rgba[i + 3] = round_u8(acc[3] / (s * s) * 255.0);
        }
    }
    IconImage { w: out, h: out, rgba, source: img.source }
}

fn libm_floor(v: f64) -> f64 {
    let t = v as i64 as f64;
    if t > v { t - 1.0 } else { t }
}

fn round_u8(v: f64) -> u8 {
    (v + 0.5).clamp(0.0, 255.0) as u8
}

/// The picture as an SVG document: a raster as pixel squares on a grid of at most `max_grid` px ([`fit`]); an SVG as it is.
/// The second value is the number of distinct colours (paths) a raster became, 0 for an SVG.
pub fn to_svg(p: &Picture, max_grid: u32) -> (String, usize) {
    let img = match p {
        Picture::Svg { text, .. } => return (text.clone(), 0),
        Picture::Raster(i) => fit(i, max_grid),
    };
    let mut paths: BTreeMap<[u8; 4], String> = BTreeMap::new();
    for y in 0..img.h {
        let mut x = 0;
        while x < img.w {
            let c = img.px(x, y);
            let mut n = 1;
            while x + n < img.w && img.px(x + n, y) == c {
                n += 1;
            }
            if c[3] != 0 {
                let _ = write!(paths.entry(c).or_default(), "M{x} {y}h{n}v1h-{n}z");
            }
            x += n;
        }
    }
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{0}\" height=\"{1}\" viewBox=\"0 0 {0} {1}\">\n", img.w, img.h);
    for (c, d) in &paths {
        let op = if c[3] == 255 { String::new() } else { format!(" fill-opacity=\"{:.3}\"", c[3] as f64 / 255.0) };
        let _ = writeln!(s, "<path fill=\"#{:02x}{:02x}{:02x}\"{op} d=\"{d}\"/>", c[0], c[1], c[2]);
    }
    s.push_str("</svg>\n");
    (s, paths.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> IconImage {
        IconImage { w, h, rgba: c.repeat((w * h) as usize), source: "t" }
    }

    #[test]
    fn a_small_picture_keeps_its_pixels_and_a_wide_one_is_centred_on_the_square() {
        let f = fit(&solid(4, 2, [9, 8, 7, 255]), 48);
        assert_eq!((f.w, f.h), (4, 4));
        assert_eq!((f.px(0, 0)[3], f.px(0, 1), f.px(3, 2), f.px(0, 3)[3]), (0, [9, 8, 7, 255], [9, 8, 7, 255], 0));
    }

    #[test]
    fn a_large_picture_is_area_averaged_in_premultiplied_alpha() {
        let mut img = solid(4, 4, [255, 0, 0, 255]);
        for p in img.rgba.chunks_mut(8) {
            p[4..8].copy_from_slice(&[0, 0, 255, 0]); // every second pixel transparent blue
        }
        let f = fit(&img, 2);
        assert_eq!((f.w, f.px(0, 0)), (2, [255, 0, 0, 128]), "transparent pixels add no colour, half the coverage");
    }

    #[test]
    fn runs_of_one_colour_are_one_path_and_transparent_pixels_none() {
        let mut img = solid(3, 1, [1, 2, 3, 255]);
        img.rgba[8..12].copy_from_slice(&[0, 0, 0, 0]);
        let (svg, n) = to_svg(&Picture::Raster(img), 48);
        assert_eq!(n, 1);
        assert!(svg.contains("<path fill=\"#010203\" d=\"M0 1h2v1h-2z\"/>"), "the 3x1 row centred on a 3x3 square: {svg}");
        let (t, n) = to_svg(&Picture::Svg { text: "<svg/>".into(), source: "s" }, 48);
        assert_eq!((t.as_str(), n), ("<svg/>", 0));
    }
}
