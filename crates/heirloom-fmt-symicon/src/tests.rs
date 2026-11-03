//! Synthetic fixtures only (built here from the format facts); the corpus's real apps are tests/real_icons.rs (opt-in).
extern crate std;
use super::*;
use alloc::vec;
use std::collections::BTreeMap;

// ---- binary SVG-T ----

/// A 16.16 number (generations 1 and 2).
fn fx(v: f32) -> [u8; 4] {
    let i = v.floor() as i16;
    let f = ((v - i as f32) * 65535.0).round() as u16;
    let mut b = [0u8; 4];
    b[0..2].copy_from_slice(&f.to_le_bytes());
    b[2..4].copy_from_slice(&i.to_le_bytes());
    b
}

fn attr_id(id: u16) -> [u8; 2] {
    id.to_le_bytes()
}

const END_ATTRS: [u8; 2] = [0xE8, 0x03];

fn ucs2(s: &str) -> Vec<u8> {
    let b: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    [&[b.len() as u8][..], &b].concat()
}

/// `<svg viewBox="0 0 10 10"><g transform=...><path d="M1 2L3 4Z" fill="#102030"/><rect width="2" fill="none"/></g>
/// <text x="..">hi</text><a><circle r="1.5" fill="url(#g1)"/></a></svg>` in generation 2 (fixed point, colours B,G,R).
fn sample() -> Vec<u8> {
    let mut b = 0x03FA_56CEu32.to_le_bytes().to_vec();
    b.push(0); // svg
    b.extend(attr_id(88)); // viewBox
    for v in [0.0, 0.0, 10.0, 10.0] {
        b.extend(fx(v));
    }
    b.extend(attr_id(26)); // width on svg: percent flag + number
    b.push(1);
    b.extend(fx(100.0));
    b.extend(END_ATTRS);
    b.push(11); // g
    b.extend(attr_id(66)); // transform: 6 numbers + i32 type
    for v in [1.0, 0.0, 5.0, 0.0, 1.0, -2.5] {
        b.extend(fx(v));
    }
    b.extend(0i32.to_le_bytes());
    b.extend(END_ATTRS);
    b.push(30); // path
    b.extend(attr_id(79));
    b.extend(3i16.to_le_bytes());
    b.extend([0, 1, 4]);
    b.extend(4i16.to_le_bytes());
    for v in [1.0, 2.0, 3.0, 4.0] {
        b.extend(fx(v));
    }
    b.extend(attr_id(0)); // fill: flag 0 + colour B,G,R,x
    b.extend([0, 0x30, 0x20, 0x10, 0]);
    b.extend(END_ATTRS);
    b.push(0xFE);
    b.push(33); // rect
    b.extend(attr_id(26));
    b.extend(fx(2.0));
    b.extend(attr_id(0));
    b.extend([0, 0xFF, 0xFF, 0xFF, 0x01]); // none
    b.extend(END_ATTRS);
    b.push(0xFE);
    b.push(0xFE); // </g>
    b.push(25); // text: dropped, its x list and character data still consumed
    b.extend(attr_id(49));
    b.push(2);
    b.extend(fx(1.0));
    b.extend(fx(2.0));
    b.extend(END_ATTRS);
    b.push(0xFD);
    b.extend(ucs2("hi"));
    b.push(0xFE); // </text>
    b.push(18); // a -> g
    b.extend(END_ATTRS);
    b.push(27); // circle
    b.extend(attr_id(28));
    b.extend(fx(1.5));
    b.extend(attr_id(0));
    b.push(1);
    b.extend(ucs2("g1"));
    b.extend(END_ATTRS);
    b.push(0xFE);
    b.push(0xFE); // </a>
    b.push(0xFE); // </svg>
    b.push(0xFF);
    b
}

#[test]
fn binary_svg_t_becomes_svg_in_the_subset_svg_datatype_draws() {
    let s = svgb::to_svg(&sample()).unwrap();
    assert!(s.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 10 10\" width=\"100%\">"), "{s}");
    assert!(s.contains("<g transform=\"matrix(1 0 0 1 5 -2.5)\"><path d=\"M1 2L3 4Z\" fill=\"#102030\"/><rect width=\"2\" fill=\"none\"/></g>"), "{s}");
    assert!(!s.contains("text") && !s.contains("hi"), "text is dropped with its character data: {s}");
    assert!(s.contains("<g><circle r=\"1.5\" fill=\"url(#g1)\"/></g></svg>"), "a becomes g: {s}");
}

#[test]
fn generation_0_numbers_are_floats_and_colours_rgb() {
    let mut b = 0x03FA_56CCu32.to_le_bytes().to_vec();
    b.push(0);
    b.extend(END_ATTRS);
    b.push(28); // ellipse
    b.extend(attr_id(29));
    b.extend(2.25f32.to_le_bytes());
    b.extend(attr_id(1)); // stroke
    b.extend([0x10, 0x20, 0x30, 0]);
    b.extend(END_ATTRS);
    b.extend([0xFE, 0xFE, 0xFF]);
    assert!(svgb::to_svg(&b).unwrap().contains("<ellipse rx=\"2.25\" stroke=\"#102030\"/>"));
}

#[test]
fn a_binary_svg_the_decoder_cannot_follow_is_named() {
    assert_eq!(svgb::to_svg(b"<svg/>"), Err(svgb::SvgbError::NotSvgb));
    let mut b = sample();
    b.truncate(b.len() - 3);
    assert_eq!(svgb::to_svg(&b), Err(svgb::SvgbError::Truncated));
    let mut u = 0x03FA_56CEu32.to_le_bytes().to_vec();
    u.push(0);
    u.extend(attr_id(18)); // color-interpolation: no known encoding
    assert_eq!(svgb::to_svg(&u), Err(svgb::SvgbError::Attribute(18)));
    let mut x = 0x03FA_56CEu32.to_le_bytes().to_vec();
    x.push(0xFE);
    assert_eq!(svgb::to_svg(&x), Err(svgb::SvgbError::Unbalanced));
}

// ---- MIF ----

fn mif(data: &[u8], ty: u32) -> Vec<u8> {
    let mut b = b"B##4".to_vec();
    for v in [2u32, 16, 2, 32, 32 + data.len() as u32, 32, 32 + data.len() as u32] {
        b.extend(v.to_le_bytes());
    }
    b.truncate(16);
    b.extend((32u32).to_le_bytes());
    b.extend((32 + data.len() as u32).to_le_bytes());
    b.extend((32u32).to_le_bytes());
    b.extend((32 + data.len() as u32).to_le_bytes());
    b.extend(b"C##4");
    for v in [1u32, 32, data.len() as u32, ty, 6, 0, 4] {
        b.extend(v.to_le_bytes());
    }
    b.extend(data);
    b
}

fn gzip(d: &[u8]) -> Vec<u8> {
    let mut g = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3];
    g.extend(miniz_oxide::deflate::compress_to_vec(d, 6));
    g.extend([0; 8]); // CRC + size: not checked
    g
}

#[test]
fn a_mif_entry_is_binary_svg_t_or_text_svg_and_may_be_gzip_wrapped() {
    let p = mif::icon(&mif(&sample(), 1)).unwrap();
    assert!(matches!(&p, Picture::Svg { source: "mif-svgb", text } if text.contains("<path")), "{p:?}");
    let p = mif::icon(&mif(&gzip(&sample()), 1)).unwrap();
    assert_eq!(p.source(), "mif-svgb");
    let p = mif::icon(&mif(b"<svg/>", 1)).unwrap();
    assert_eq!(p, Picture::Svg { text: "<svg/>".into(), source: "mif-svg" });
    assert!(mif::icon(&mif(b"x", 2)).unwrap_err().contains("NVG"));
    assert!(mif::icon(b"C##4").is_err());
}

// ---- MBM ----

/// An MBM of `bitmaps` = (w, h, bpp, colour flag, compression, data).
fn mbm(bitmaps: &[(u32, u32, u32, u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [0x1000_0037u32, 0x1000_0042, 0, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    let mut offs = Vec::new();
    for (w, h, bpp, color, comp, data) in bitmaps {
        offs.push(b.len() as u32);
        for v in [40 + data.len() as u32, 40, *w, *h, 0, 0, *bpp, *color, 0, *comp] {
            b.extend(v.to_le_bytes());
        }
        b.extend(data);
    }
    let t = b.len() as u32;
    b[16..20].copy_from_slice(&t.to_le_bytes());
    b.extend((offs.len() as u32).to_le_bytes());
    for o in offs {
        b.extend(o.to_le_bytes());
    }
    b
}

#[test]
fn an_mbm_icon_is_its_largest_colour_bitmap_masked_by_the_next() {
    // 2x2 16-bit 565: red, green / blue, white; rows padded to 4 bytes
    let colour: Vec<u8> = [0xF800u16, 0x07E0, 0x001F, 0xFFFF].iter().flat_map(|v| v.to_le_bytes()).collect();
    // the mask, 1 bit, LSB first: row 0 = 1,0  row 1 = 1,1  (each row padded to 4 bytes)
    let mask = vec![0b01, 0, 0, 0, 0b11, 0, 0, 0];
    let small = (1, 1, 24, 1, 0, vec![1, 2, 3, 0]);
    let f = mbm(&[small, (2, 2, 16, 1, 0, colour.clone()), (2, 2, 1, 0, 0, mask)]);
    let img = mbm::icon(&f).unwrap();
    assert_eq!((img.w, img.source), (2, "mbm+mask"));
    assert_eq!((img.px(0, 0), img.px(1, 0)[3], img.px(0, 1), img.px(1, 1)), ([255, 0, 0, 255], 0, [0, 0, 255, 255], [255, 255, 255, 255]));
    // byte RLE: +3 = the next byte 4 times; -2 = 2 literal bytes
    let rle = vec![3u8, 0x80, 0xFE, 0x10, 0x20];
    let g = mbm(&[(6, 1, 8, 0, 1, rle)]);
    let img = mbm::icon(&g).unwrap();
    assert_eq!((img.source, img.px(0, 0), img.px(5, 0)), ("mbm", [0x80, 0x80, 0x80, 255], [0x20, 0x20, 0x20, 255]));
    let c256 = mbm(&[(1, 1, 8, 1, 0, vec![5, 0, 0, 0])]);
    assert!(mbm::icon(&c256).unwrap_err().contains("256-colour"));
    assert!(mbm::icon(b"nope").is_err());
}

// ---- the registration chain ----

/// A plain resource file whose resources are raw bytes (no unicode runs).
fn rsc(res: &[Vec<u8>]) -> Vec<u8> {
    let mut f = Vec::new();
    for v in [0x101F_4A6Bu32, 0, 0, 0] {
        f.extend(v.to_le_bytes());
    }
    f.push(0);
    f.extend([0, 0]);
    f.extend(vec![0u8; res.len().div_ceil(8)]);
    let mut offs = Vec::new();
    for r in res {
        offs.push(f.len() as u16);
        f.extend(r);
    }
    let index = f.len() as u16;
    for o in offs {
        f.extend(o.to_le_bytes());
    }
    f.extend(index.to_le_bytes());
    f
}

/// Append an LTEXT at `b`'s end: the length, a pad to an even offset, UTF-16LE.
fn ltext(b: &mut Vec<u8>, s: &str) {
    b.push(s.encode_utf16().count() as u8);
    if b.len() % 2 == 1 {
        b.push(0xAB);
    }
    b.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
}

fn reg(app: &str, loc: &str, id: u32) -> Vec<u8> {
    let mut r = vec![0u8; 8];
    ltext(&mut r, app);
    r.extend(0u32.to_le_bytes());
    ltext(&mut r, loc);
    r.extend(id.to_le_bytes());
    rsc(&[r])
}

fn loc(icon: &str) -> Vec<u8> {
    let mut r = vec![0u8; 8];
    ltext(&mut r, "App");
    r.extend([0u8; 8]);
    ltext(&mut r, "My App");
    r.extend(1u16.to_le_bytes());
    ltext(&mut r, icon);
    rsc(&[vec![1, 2, 3, 4], r])
}

fn tree(files: &[(&str, Vec<u8>)]) -> BTreeMap<String, Vec<u8>> {
    files.iter().map(|(p, d)| (String::from(*p), d.clone())).collect()
}

fn icon_of(t: &BTreeMap<String, Vec<u8>>, exe: &str) -> Result<Picture, SymIconError> {
    let read = |p: &str| t.get(p).cloned();
    let list = |dir: &str| -> Vec<String> {
        let pre = if dir.is_empty() { String::new() } else { alloc::format!("{dir}/") };
        let mut v: Vec<String> = t.keys().filter_map(|k| k.strip_prefix(&pre)).map(|r| String::from(r.split('/').next().unwrap())).collect();
        v.dedup();
        v
    };
    app_icon(&read, &list, exe)
}

#[test]
fn the_registration_names_the_localisable_resource_which_names_the_icon_file() {
    let t = tree(&[
        ("Private/10003A3F/Import/Apps/other_reg.rsc", reg("other", "\\resource\\apps\\other", 1)),
        ("Private/10003A3F/Import/Apps/myapp_reg.rsc", reg("MyApp", "\\Resource\\Apps\\MyApp", 2)),
        ("resource/apps/myapp.r01", loc("\\resource\\apps\\myapp_icon.mif")),
        ("resource/apps/MYAPP_ICON.MIF", mif(&sample(), 1)),
    ]);
    let p = icon_of(&t, "sys/bin/myapp.exe").unwrap();
    assert_eq!(p.source(), "mif-svgb", "case-insensitive paths, the .r01 localisation, resource 2");
    assert_eq!(icon_of(&t, "nothere.exe"), Err(SymIconError::NoRegistration("nothere".into())));
    let gone = tree(&[("private/10003a3f/import/apps/myapp_reg.rsc", reg("myapp", "\\resource\\apps\\myapp", 2))]);
    assert_eq!(icon_of(&gone, "myapp"), Err(SymIconError::Missing("\\resource\\apps\\myapp.rsc".into())));
    let no_icon = tree(&[("private/10003a3f/import/apps/a_reg.rsc", reg("a", "\\r\\a", 2)), ("r/a.rsc", loc(""))]);
    assert_eq!(icon_of(&no_icon, "a"), Err(SymIconError::NoIconFile));
}
