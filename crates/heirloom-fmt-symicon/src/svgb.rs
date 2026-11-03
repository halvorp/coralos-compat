//! Symbian binary SVG-T ("SVGB", the form `svgtbinencode` writes into MIF files) to SVG text in the subset svg.datatype draws.
//!
//! FORMAT FACTS (read from EKA2L1 `src/emu/loader/src/svgb.cpp`, itself from svgb2svg / svgb_decoder; GPL-3: facts only, this
//! code is written from them, and checked against the corpus's real MIF icons by tests/real_icons.rs):
//! * A u32 LE magic `0x03FA56CC..=0x03FA56CF` (generations 0..3). Generations 1 and 2 store every number as 16.16 fixed point
//!   (u16 fraction, then i16 integer), 0 and 3 as f32 LE; generations 0 and 1 store colours R,G,B, 2 and 3 B,G,R (a 4th byte).
//! * Then elements: one byte, the element code ([`ELEMENTS`]); its attributes, each a u16 LE id ([`ATTRS`]) and a value whose
//!   encoding the attribute decides; the attribute list ends with the bytes `E8 03`, followed by `FE` when the element has no
//!   children (else its first child follows). `FE` alone closes the open element, `FD` is character data (u8 byte count,
//!   UCS-2), `FF` ends the file.
//!
//! NORMALISED to what svg.datatype (nanosvg) draws: shapes, paths, groups, gradients and their stops keep their attributes;
//! `a` and `switch` become `g`; text, images, `use`, fonts, scripts, animation and metadata are dropped with their whole
//! subtree (their values are still decoded, so the stream stays in step). An attribute whose encoding is not known stops the
//! conversion by name - a guessed length would misread everything after it.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// Element codes 0..=42 in order.
const ELEMENTS: [&str; 43] = [
    "svg", "altglyph", "altglyphdef", "defs", "desc", "foreignObject", "metadata", "title", "fontfacename", "fontfacesrc",
    "fontfaceuri", "g", "glyphref", "vkern", "script", "switch", "view", "hkern", "a", "font", "fontface", "glyph", "image",
    "missingglyph", "style", "text", "use", "circle", "ellipse", "line", "path", "polygon", "polyline", "rect", "animate",
    "animateColor", "animateMotion", "animateTransform", "set", "mpath", "linearGradient", "radialGradient", "stop",
];

/// How an attribute's value is encoded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Enc {
    Fill,
    Color,
    Float,
    /// A float; x / y on `text` are a u8 count then that many floats.
    Coord,
    /// A float, preceded on the root `svg` by a percent flag byte.
    Size,
    Str,
    /// An i32 indexing the names.
    CssEnum(&'static [&'static str]),
    /// An i8 indexing the names.
    Enum(&'static [&'static str]),
    Dash,
    Matrix,
    GradMatrix,
    Points,
    Path,
    Rect,
    Aspect,
    Href,
    LongHref,
    /// One byte, nothing written.
    Skip1,
    Unknown,
}

const DISPLAY: [&str; 17] = ["inline", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "", "none"];

/// `(name, encoding)` for attribute ids 0..=136 (ids with no known encoding are `Unknown`).
fn attr(id: u16) -> Option<(&'static str, Enc)> {
    use Enc::*;
    Some(match id {
        0 => ("fill", Fill),
        1 => ("stroke", Color),
        2 => ("stroke-width", Float),
        3 => ("visibility", CssEnum(&["visible", "hidden"])),
        4 => ("font-family", Str),
        5 => ("font-size", Float),
        6 => ("font-style", CssEnum(&["normal", "italic", "oblique", "inherit"])),
        7 => ("font-weight", CssEnum(&["normal", "bold", "bolder", "lighter", "100", "200", "300", "400", "500", "600", "700", "800", "900", "inherit"])),
        8 => ("stroke-dasharray", Dash),
        9 => ("display", CssEnum(&DISPLAY)),
        10 => ("fill-rule", Str),
        11 => ("stroke-linecap", Str),
        12 => ("stroke-linejoin", Str),
        13 => ("stroke-dashoffset", Float),
        14 => ("stroke-miterlimit", Float),
        15 => ("color", Color),
        16 => ("text-anchor", CssEnum(&["start", "middle", "end"])),
        17 => ("text-decoration", CssEnum(&["none", "underline", "overline", "line-through"])),
        22 => ("opacity", Float),
        23 => ("stroke-opacity", Float),
        24 => ("fill-opacity", Float),
        26 => ("width", Size),
        27 => ("height", Size),
        28 => ("r", Float),
        29 => ("rx", Float),
        30 => ("ry", Float),
        46 => ("cx", Float),
        47 => ("cy", Float),
        48 => ("y", Coord),
        49 => ("x", Coord),
        50 => ("y1", Float),
        51 => ("y2", Float),
        52 => ("x1", Float),
        53 => ("x2", Float),
        66 => ("transform", Matrix),
        74 => ("attributeName", Skip1),
        76 => ("version", Float),
        78 => ("points", Points),
        79 => ("d", Path),
        80 => ("type", Str),
        81 => ("stop-color", Color),
        82 => ("fx", Float),
        83 => ("fy", Float),
        84 => ("offset", Float),
        85 => ("spreadMethod", Enum(&["pad", "reflect", "repeat"])),
        86 => ("gradientUnits", Enum(&["userSpaceOnUse", "objectBoundingBox"])),
        87 => ("stop-opacity", Float),
        88 => ("viewBox", Rect),
        89 => ("baseProfile", Str),
        90 => ("zoomAndPan", Skip1),
        91 => ("preserveAspectRatio", Aspect),
        92 => ("id", Str),
        93 => ("xml:base", Str),
        94 => ("xml:lang", Str),
        95 => ("xml:space", Str),
        103 => ("xlink:actuate", Str),
        106 => ("xlink:show", Str),
        108 => ("xlink:type", Str),
        109 => ("xlink:href", Href),
        123 => ("gradientTransform", GradMatrix),
        136 => ("xlink:href", LongHref),
        0..=136 => ("?", Unknown),
        _ => return None,
    })
}

/// Why a binary SVG could not be converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgbError {
    NotSvgb,
    Truncated,
    /// An attribute id whose value encoding is not known (the stream cannot be followed past it).
    Attribute(u16),
    Element(u8),
    /// The file closed an element it never opened.
    Unbalanced,
}

struct In<'a> {
    b: &'a [u8],
    at: usize,
    gen: u8,
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SvgbError> {
        let s = self.b.get(self.at..self.at + n).ok_or(SvgbError::Truncated)?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, SvgbError> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, SvgbError> {
        let s = self.take(2)?;
        Ok(i16::from_le_bytes([s[0], s[1]]))
    }
    fn i32(&mut self) -> Result<i32, SvgbError> {
        let s = self.take(4)?;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn num(&mut self) -> Result<f32, SvgbError> {
        let s = self.take(4)?;
        Ok(if matches!(self.gen, 1 | 2) {
            i16::from_le_bytes([s[2], s[3]]) as f32 + u16::from_le_bytes([s[0], s[1]]) as f32 / 65535.0
        } else {
            f32::from_le_bytes([s[0], s[1], s[2], s[3]])
        })
    }
    fn ucs2(&mut self, bytes: usize) -> Result<String, SvgbError> {
        let s = self.take(bytes)?;
        Ok(char::decode_utf16(s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]))).map(|r| r.unwrap_or('\u{FFFD}')).collect())
    }
    fn color(&mut self) -> Result<String, SvgbError> {
        let c = self.take(4)?;
        let (r, g, b) = if self.gen <= 1 { (c[0], c[1], c[2]) } else { (c[2], c[1], c[0]) };
        Ok(format!("#{r:02x}{g:02x}{b:02x}"))
    }
}

/// A number as SVG text: at most 4 decimals, no trailing zeros.
fn num(v: f32) -> String {
    let mut s = format!("{:.4}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { String::from("0") } else { s }
}

fn nums(inp: &mut In, n: usize, sep: &str) -> Result<String, SvgbError> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(num(inp.num()?));
    }
    Ok(v.join(sep))
}

/// One attribute's value as text (None = nothing to write).
fn value(inp: &mut In, enc: Enc, elem: u8) -> Result<Option<String>, SvgbError> {
    use Enc::*;
    Ok(match enc {
        Fill => match inp.u8()? {
            0 => {
                let c = inp.b.get(inp.at..inp.at + 4).ok_or(SvgbError::Truncated)?;
                if c == [0xFF, 0xFF, 0xFF, 0x01] {
                    inp.at += 4;
                    Some("none".into())
                } else if c == [0xFF, 0xFF, 0xFF, 0x02] {
                    inp.at += 4;
                    Some("currentColor".into())
                } else {
                    Some(inp.color()?)
                }
            }
            1 => {
                let n = inp.u8()? as usize;
                Some(format!("url(#{})", inp.ucs2(n)?))
            }
            _ => return Err(SvgbError::Attribute(0)),
        },
        Color => Some(inp.color()?),
        Float => Some(num(inp.num()?)),
        Coord if elem == 25 => {
            let n = inp.u8()? as usize;
            Some(nums(inp, n, " ")?)
        }
        Coord => Some(num(inp.num()?)),
        Size => {
            let pct = if elem == 0 { inp.u8()? != 0 } else { false };
            Some(format!("{}{}", num(inp.num()?), if pct { "%" } else { "" }))
        }
        Str => {
            let n = inp.u8()? as usize;
            Some(inp.ucs2(n)?)
        }
        CssEnum(names) => names.get(inp.i32()? as usize).filter(|n| !n.is_empty()).map(|n| String::from(*n)),
        Enum(names) => names.get(inp.u8()? as i8 as isize as usize).map(|n| String::from(*n)),
        Dash => {
            let n = inp.u8()? as usize;
            Some(nums(inp, n, ",")?)
        }
        Matrix | GradMatrix => {
            let mut m = [0f32; 6];
            for v in &mut m {
                *v = inp.num()?;
                if inp.gen == 0 && enc == GradMatrix {
                    *v /= 65535.0;
                }
            }
            if enc == Matrix {
                inp.i32()?; // the transform's type
            }
            Some(format!("matrix({} {} {} {} {} {})", num(m[0]), num(m[3]), num(m[1]), num(m[4]), num(m[2]), num(m[5])))
        }
        Points => {
            let nf = inp.i16()?.max(0) as usize;
            inp.take(nf)?;
            let nv = inp.i16()?.max(0) as usize;
            Some(nums(inp, nv / 2 * 2, " ")?)
        }
        Path => {
            let ns = inp.i16()?.max(0) as usize;
            let segs = inp.take(ns)?.to_vec();
            let nv = inp.i16()?.max(0) as usize;
            let (mut d, mut used) = (String::new(), 0usize);
            for s in segs {
                let (c, k) = match s { 0 => ('M', 2), 1 => ('L', 2), 2 => ('Q', 4), 3 => ('C', 6), 4 => ('Z', 0), _ => return Err(SvgbError::Attribute(79)) };
                if used + k > nv {
                    return Err(SvgbError::Attribute(79));
                }
                d.push(c);
                d.push_str(&nums(inp, k, " ")?);
                used += k;
            }
            Some(d)
        }
        Rect => Some(nums(inp, 4, " ")?),
        Aspect if elem == 0 || elem == 47 => {
            inp.take(2)?;
            None
        }
        Aspect => {
            let n = inp.u8()? as usize;
            Some(inp.ucs2(n)?)
        }
        Href => {
            let n = inp.u8()? as usize;
            let s = inp.ucs2(n)?;
            if !s.is_empty() {
                let skip = inp.u8()? as usize;
                inp.take(skip)?;
            }
            Some(s)
        }
        LongHref => {
            let n = inp.i32()?.max(0) as usize;
            Some(inp.take(n)?.iter().map(|&c| c as char).collect())
        }
        Skip1 => {
            inp.u8()?;
            None
        }
        Unknown => return Err(SvgbError::Truncated),
    })
}

/// The element names svg.datatype draws (and the renames); None = dropped with its subtree.
fn kept(code: u8) -> Option<&'static str> {
    match code {
        0 | 3 | 11 | 27..=33 | 40..=42 => ELEMENTS.get(code as usize).copied(),
        15 | 18 => Some("g"),
        _ => None,
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

/// The SVG document a binary SVG-T icon is.
pub fn to_svg(b: &[u8]) -> Result<String, SvgbError> {
    let magic = b.get(0..4).map(|m| u32::from_le_bytes([m[0], m[1], m[2], m[3]])).ok_or(SvgbError::NotSvgb)?;
    if !(0x03FA_56CC..=0x03FA_56CF).contains(&magic) {
        return Err(SvgbError::NotSvgb);
    }
    let mut inp = In { b, at: 4, gen: (magic - 0x03FA_56CC) as u8 };
    let mut out = String::new();
    // the open elements: (name written, or None when dropped)
    let mut open: Vec<Option<&'static str>> = Vec::new();
    loop {
        let c = inp.u8()?;
        match c {
            0xFF => break,
            0xFE => match open.pop().ok_or(SvgbError::Unbalanced)? {
                Some(n) => {
                    let _ = write!(out, "</{n}>");
                }
                None => {}
            },
            0xFD => {
                let n = inp.u8()? as usize;
                inp.take(n)?; // character data: only text has it, and text is dropped
            }
            code if (code as usize) < ELEMENTS.len() => {
                let hidden = open.iter().any(Option::is_none);
                let name = if hidden { None } else { kept(code) };
                if let Some(n) = name {
                    out.push('<');
                    out.push_str(n);
                    if code == 0 {
                        out.push_str(" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
                    }
                }
                loop {
                    let mark = inp.b.get(inp.at..inp.at + 2).ok_or(SvgbError::Truncated)?;
                    if mark == [0xE8, 0x03] {
                        inp.at += 2;
                        let next = *inp.b.get(inp.at).ok_or(SvgbError::Truncated)?;
                        if next == 0xFE {
                            inp.at += 1;
                            if name.is_some() {
                                out.push_str("/>");
                            }
                        } else {
                            if name.is_some() {
                                out.push('>');
                            }
                            open.push(name);
                        }
                        break;
                    }
                    let id = inp.i16()? as u16;
                    let (an, enc) = attr(id).ok_or(SvgbError::Attribute(id))?;
                    if enc == Enc::Unknown {
                        return Err(SvgbError::Attribute(id));
                    }
                    if let Some(v) = value(&mut inp, enc, code)? {
                        if name.is_some() && !(v.is_empty() && an == "xlink:href") {
                            let _ = write!(out, " {an}=\"{}\"", escape(&v));
                        }
                    }
                }
            }
            other => return Err(SvgbError::Element(other)),
        }
    }
    if !open.is_empty() {
        return Err(SvgbError::Unbalanced);
    }
    out.push('\n');
    Ok(out)
}
