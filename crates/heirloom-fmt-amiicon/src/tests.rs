//! Synthetic fixtures only (built here from the format facts); the corpus's real icons are tests/real_icons.rs (opt-in).
use super::*;
use alloc::vec;
use miniz_oxide::deflate::compress_to_vec_zlib;

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// A planar `Image` header and planes for a `w` x `h` picture whose pen at (x, y) is `pen(x, y)`.
fn image(w: u16, h: u16, depth: u16, pen: impl Fn(usize, usize) -> u8) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(&be16(w));
    b.extend_from_slice(&be16(h));
    b.extend_from_slice(&be16(depth));
    b.extend_from_slice(&be32(1));
    b.push(((1u16 << depth) - 1) as u8);
    b.push(0);
    b.extend_from_slice(&be32(0));
    let row = (w as usize).div_ceil(16) * 2;
    for k in 0..depth {
        for y in 0..h as usize {
            let mut r = vec![0u8; row];
            for x in 0..w as usize {
                if pen(x, y) & (1 << k) != 0 {
                    r[x / 8] |= 0x80 >> (x % 8);
                }
            }
            b.extend_from_slice(&r);
        }
    }
    b
}

/// A DiskObject string: ISO-8859-1 (one byte per char), NUL-terminated, its length first.
fn string(s: &str) -> Vec<u8> {
    let b: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
    [&be32(b.len() as u32 + 1)[..], &b, &[0]].concat()
}

/// A DiskObject: revision, the normal image, a selected image, tool types, then `tail` (a FORM ICON).
fn info(revision: u8, normal: &[u8], selected: Option<&[u8]>, tool_types: &[&str], tail: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 78];
    b[0..2].copy_from_slice(&be16(0xE310));
    b[2..4].copy_from_slice(&be16(1));
    b[22..26].copy_from_slice(&be32(1)); // GadgetRender
    if selected.is_some() {
        b[26..30].copy_from_slice(&be32(1));
    }
    b[44..48].copy_from_slice(&be32(revision as u32));
    b[48] = 3; // WBTOOL
    b[50..54].copy_from_slice(&be32(1)); // a default tool
    if !tool_types.is_empty() {
        b[54..58].copy_from_slice(&be32(1));
    }
    b.extend_from_slice(normal);
    if let Some(s) = selected {
        b.extend_from_slice(s);
    }
    b.extend(string("SYS:Utilities/MultiView"));
    if !tool_types.is_empty() {
        b.extend_from_slice(&be32((tool_types.len() as u32 + 1) * 4));
        for t in tool_types {
            b.extend(string(t));
        }
    }
    b.extend_from_slice(tail);
    b
}

#[test]
fn a_planar_icon_takes_the_workbench_pens_of_its_revision_and_pen_0_is_transparent() {
    let normal = image(17, 2, 2, |x, y| ((x + y) % 4) as u8);
    let selected = image(17, 2, 2, |_, _| 3);
    let img = app_icon(&info(1, &normal, Some(&selected), &[], &[])).unwrap();
    assert_eq!((img.w, img.h, img.source), (17, 2, "planar+os2"));
    assert_eq!(img.px(0, 0), [0xAA, 0xAA, 0xAA, 0], "pen 0 is the background");
    assert_eq!((img.px(1, 0), img.px(2, 0), img.px(3, 0)), ([0, 0, 0, 255], [255, 255, 255, 255], [0x66, 0x88, 0xBB, 255]));
    assert_eq!(img.px(16, 1), [0, 0, 0, 255], "past the first 16-bit word of the row; the selected image (all pen 3) is ignored");
    let old = app_icon(&info(0, &normal, None, &[], &[])).unwrap();
    assert_eq!((old.source, old.px(3, 0)), ("planar+os1", [0xFF, 0x88, 0x00, 255]), "OS1.x orange");
    let eight = app_icon(&info(1, &image(8, 1, 3, |x, _| x as u8), None, &[], &[])).unwrap();
    assert_eq!((eight.source, eight.px(7, 0)), ("planar+magicwb", [0xFF, 0xA9, 0x97, 255]));
    assert!(matches!(app_icon(&info(1, &image(8, 1, 4, |x, _| x as u8), None, &[], &[])), Err(AmiIconError::NoMember(_))), "no pens known for 16 colours");
}

#[test]
fn a_cut_short_or_foreign_file_is_named() {
    assert_eq!(app_icon(b"\0\0not an icon"), Err(AmiIconError::NotAnIcon));
    let b = info(1, &image(16, 4, 2, |_, _| 1), None, &[], &[]);
    assert_eq!(app_icon(&b[..100]), Err(AmiIconError::Truncated("Image planes")));
}

/// NewIcons' 7-bit text for `values` of `bits` bits (no zero runs).
fn encode7(values: &[u8], bits: u32) -> Vec<u8> {
    let (mut acc, mut n, mut out) = (0u32, 0u32, Vec::new());
    for &v in values {
        acc = acc << bits | v as u32;
        n += bits;
        while n >= 7 {
            out.push(((acc >> (n - 7)) & 0x7F) as u8);
            n -= 7;
        }
    }
    if n > 0 {
        out.push(((acc << (7 - n)) & 0x7F) as u8);
    }
    out.into_iter().map(|v| if v < 0x50 { v + 0x20 } else { v + 0x51 }).collect()
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

#[test]
fn a_newicon_is_read_from_its_tool_types_line_by_line() {
    let palette = [10, 20, 30, 200, 100, 50, 1, 2, 3];
    let pixels = [0u8, 1, 2, 1, 0, 2];
    let head = [b'B', 3 + 0x21, 2 + 0x21, 0x21, 3 + 0x21];
    let l1 = latin1(&[&b"IM1="[..], &head, &encode7(&palette, 8)].concat());
    let l2 = latin1(&[&b"IM1="[..], &encode7(&pixels, 2)].concat());
    let l3 = latin1(b"IM2=Bxxxx"); // the selected image: ignored
    let tts = ["DONOTWAIT", "*** DON'T EDIT THE FOLLOWING LINES!! ***", &l1, &l2, &l3];
    let img = app_icon(&info(1, &image(16, 1, 2, |_, _| 1), None, &tts, &[])).unwrap();
    assert_eq!((img.w, img.h, img.source), (3, 2, "newicon"), "the NewIcon wins over the planar image");
    assert_eq!((img.px(0, 0)[3], img.px(1, 0), img.px(2, 1)), (0, [200, 100, 50, 255], [1, 2, 3, 255]));
    // a zero run: 0xD1 + k is k+1 zero groups
    let zl = latin1(&[&b"IM1="[..], &[0xD3]].concat()); // 3 zero groups = 21 bits = ten 2-bit zeros
    let tts = ["*** DON'T EDIT THE FOLLOWING LINES!! ***", &l1, &zl];
    let img = newicons::decode(&tts.iter().map(|s| String::from(*s)).collect::<Vec<_>>()).unwrap().unwrap();
    assert!(img.rgba.chunks(4).all(|p| p == [10, 20, 30, 0]));
    let cut = ["*** DON'T EDIT THE FOLLOWING LINES!! ***", l1.as_str()];
    assert!(newicons::decode(&cut.iter().map(|s| String::from(*s)).collect::<Vec<_>>()).is_err(), "no pixel line");
}

fn chunk(id: &[u8; 4], d: &[u8]) -> Vec<u8> {
    let mut c = [&id[..], &be32(d.len() as u32), d].concat();
    if d.len() % 2 == 1 {
        c.push(0);
    }
    c
}

fn form(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = [&b"ICON"[..], &chunks.concat()].concat();
    [&b"FORM"[..], &be32(body.len() as u32), &body].concat()
}

fn face(w: u8, h: u8) -> Vec<u8> {
    chunk(b"FACE", &[w - 1, h - 1, 0, 0, 0, 0])
}

fn imag(transparent: u8, pens: u8, flags: u8, img_fmt: u8, depth: u8, image: &[u8], palette: &[u8]) -> Vec<u8> {
    let mut d = vec![transparent, pens - 1, flags, img_fmt, 0, depth];
    d.extend_from_slice(&be16(image.len() as u16 - 1));
    d.extend_from_slice(&be16(palette.len().max(1) as u16 - 1));
    d.extend_from_slice(image);
    d.extend_from_slice(palette);
    chunk(b"IMAG", &d)
}

#[test]
fn a_coloricon_imag_raw_and_run_length_with_its_transparent_pen() {
    let pal = [9, 9, 9, 255, 0, 0, 0, 255, 0];
    let raw = form(&[face(2, 2), imag(0, 3, 3, 0, 2, &[0, 1, 2, 1], &pal)]);
    let img = app_icon(&info(1, &image(16, 1, 2, |_, _| 1), None, &[], &raw)).unwrap();
    assert_eq!((img.w, img.source, img.px(0, 0)[3], img.px(1, 0), img.px(0, 1)), (2, "coloricon", 0, [255, 0, 0, 255], [0, 255, 0, 255]));
    // run-length, 2-bit values: control 129 (repeat 128 times? no: 257-129 = 128) - here 255 = repeat 2 times, then literal 1 value
    // bits: 11111111 01 | 00000001 10 01 ... built by hand: [ctl 255][v=1][ctl 1][v=2][v=1]
    let bitsv: [(u32, u32); 5] = [(255, 8), (1, 2), (1, 8), (2, 2), (1, 2)];
    let (mut acc, mut n, mut packed) = (0u64, 0u32, Vec::new());
    for (v, w) in bitsv {
        acc = acc << w | v as u64;
        n += w;
    }
    acc <<= (8 - n % 8) % 8;
    n += (8 - n % 8) % 8;
    for i in (0..n / 8).rev() {
        packed.push((acc >> (i * 8)) as u8);
    }
    assert_eq!(coloricon::unpack(&packed, 2, 4), Some(vec![1, 1, 2, 1]));
    let rle = form(&[face(2, 2), imag(9, 3, 2, 1, 2, &packed, &pal)]);
    let img = app_icon(&info(1, &image(16, 1, 2, |_, _| 1), None, &[], &rle)).unwrap();
    assert_eq!((img.px(0, 0), img.px(0, 1)), ([255, 0, 0, 255], [0, 255, 0, 255]), "flags bit 0 clear: no transparent pen");
    assert_eq!(coloricon::unpack(&[0x80, 0x00, 0x00], 8, 1), Some(vec![0]), "128 is a no-op control");
}

#[test]
fn an_os4_argb_chunk_wins_over_the_palette_image() {
    let argb: Vec<u8> = [[0u8, 1, 2, 3], [255, 4, 5, 6]].concat();
    let z = compress_to_vec_zlib(&argb, 6);
    let mut d = vec![0u8; 10];
    d[4..8].copy_from_slice(&be32(z.len() as u32 - 1));
    d.extend_from_slice(&z);
    let f = form(&[face(2, 1), imag(0, 2, 2, 0, 1, &[1, 1], &[7, 7, 7, 8, 8, 8]), chunk(b"ARGB", &d)]);
    let img = app_icon(&info(1, &image(16, 1, 2, |_, _| 1), None, &[], &f)).unwrap();
    assert_eq!((img.source, img.px(0, 0), img.px(1, 0)), ("coloricon-argb", [1, 2, 3, 0], [4, 5, 6, 255]));
}

/// A PNG of `rows` (each a filter byte and its samples).
fn png_file(w: u32, h: u32, depth: u8, ctype: u8, rows: &[u8], plte: &[u8], trns: &[u8]) -> Vec<u8> {
    let mut ihdr = [&be32(w)[..], &be32(h)].concat();
    ihdr.extend_from_slice(&[depth, ctype, 0, 0, 0]);
    let ch = |id: &[u8; 4], d: &[u8]| [&be32(d.len() as u32)[..], id, d, &[0, 0, 0, 0]].concat(); // the CRC is not checked
    let mut b = png::SIGNATURE.to_vec();
    b.extend(ch(b"IHDR", &ihdr));
    if !plte.is_empty() {
        b.extend(ch(b"PLTE", plte));
    }
    if !trns.is_empty() {
        b.extend(ch(b"tRNS", trns));
    }
    b.extend(ch(b"IDAT", &compress_to_vec_zlib(rows, 6)));
    b.extend(ch(b"IEND", &[]));
    b
}

#[test]
fn a_png_icon_is_unfiltered_rgba_or_palette() {
    // RGBA 2x2: row 0 filter Sub, row 1 filter Paeth
    let rows = [1, 10, 20, 30, 255, 5, 5, 5, 0, 4, 1, 1, 1, 1, 2, 2, 2, 2];
    let img = app_icon(&png_file(2, 2, 8, 6, &rows, &[], &[])).unwrap();
    assert_eq!((img.source, img.px(0, 0), img.px(1, 0)), ("png", [10, 20, 30, 255], [15, 25, 35, 255]));
    assert_eq!((img.px(0, 1), img.px(1, 1)), ([11, 21, 31, 0], [17, 27, 37, 2]));
    // palette, 1 bit: pixels 1, 0, 1
    let img = app_icon(&png_file(3, 1, 1, 3, &[0, 0b1010_0000], &[1, 1, 1, 2, 2, 2], &[0])).unwrap();
    assert_eq!((img.px(0, 0), img.px(1, 0), img.px(2, 0)), ([2, 2, 2, 255], [1, 1, 1, 0], [2, 2, 2, 255]));
    let mut interlaced = png_file(1, 1, 8, 6, &[0, 1, 2, 3, 4], &[], &[]);
    interlaced[8 + 8 + 12] = 1;
    assert!(app_icon(&interlaced).is_err(), "interlace is refused by name");
}
