//! Synthetic fixtures only (built with heirloom-fmt-resfork's writer); the real application is tests/real_app.rs (opt-in).
use super::*;
use alloc::vec;
use heirloom_fmt_resfork::writer::{macbinary, ForkBuilder};

/// A BNDL: signature, then `(type, [(local id, resource id)])` tables.
fn bndl(sig: &[u8; 4], tables: &[(&[u8; 4], &[(i16, i16)])]) -> Vec<u8> {
    let mut b = sig.to_vec();
    b.extend_from_slice(&0i16.to_be_bytes());
    b.extend_from_slice(&((tables.len() as i16) - 1).to_be_bytes());
    for (ty, pairs) in tables {
        b.extend_from_slice(&ty[..]);
        b.extend_from_slice(&((pairs.len() as i16) - 1).to_be_bytes());
        for (l, r) in pairs.iter() {
            b.extend_from_slice(&l.to_be_bytes());
            b.extend_from_slice(&r.to_be_bytes());
        }
    }
    b
}

fn fref(ty: &[u8; 4], local: i16) -> Vec<u8> {
    let mut f = ty.to_vec();
    f.extend_from_slice(&local.to_be_bytes());
    f.push(0);
    f
}

/// An app's bundle: FREF 128 = APPL local 0, FREF 129 = TEXT local 1; ICN# local 0 -> family 128, local 1 -> 129.
fn app_bundle(sig: &[u8; 4]) -> ForkBuilder {
    ForkBuilder::new()
        .add(b"BNDL", 128, None, &bndl(sig, &[(b"ICN#", &[(0, 128), (1, 129)]), (b"FREF", &[(0, 129), (1, 128)])]))
        .add(b"FREF", 128, None, &fref(b"APPL", 0))
        .add(b"FREF", 129, None, &fref(b"TEXT", 1))
}

/// A 32x32 ICN#: icon plane all black on the left half, mask = the top half.
fn icn_sharp(side: usize) -> Vec<u8> {
    let row = side / 8;
    let mut icon = vec![0u8; row * side];
    let mut mask = vec![0u8; row * side];
    for y in 0..side {
        for x in 0..side / 2 {
            icon[y * row + x / 8] |= 0x80 >> (x % 8);
        }
        if y < side / 2 {
            mask[y * row..(y + 1) * row].fill(0xFF);
        }
    }
    [icon, mask].concat()
}

fn px(img: &IconImage, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.w + x) * 4) as usize;
    img.rgba[i..i + 4].try_into().unwrap()
}

fn icns(elements: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let body: Vec<u8> = elements.iter().flat_map(|(t, d)| [&t[..], &((d.len() + 8) as u32).to_be_bytes(), d].concat()).collect();
    [&b"icns"[..], &((body.len() + 8) as u32).to_be_bytes(), &body].concat()
}

/// Pack `plane` as literals of at most 128 bytes (the scheme's plain form).
fn literals(plane: &[u8]) -> Vec<u8> {
    plane.chunks(128).flat_map(|c| [&[(c.len() - 1) as u8][..], c].concat()).collect()
}

#[test]
fn the_system_tables_have_apples_layout_at_their_landmarks() {
    assert_eq!((clut::CLUT8[0], clut::CLUT8[1], clut::CLUT8[35], clut::CLUT8[214]), (0xFFFFFF, 0xFFFFCC, 0xFF0000, 0x000033));
    assert_eq!((clut::CLUT8[215], clut::CLUT8[225], clut::CLUT8[245], clut::CLUT8[254], clut::CLUT8[255]), (0xEE0000, 0x00EE00, 0xEEEEEE, 0x111111, 0));
    assert_eq!((clut::CLUT4[0], clut::CLUT4[3], clut::CLUT4[15]), (0xFFFFFF, 0xDD0806, 0));
}

#[test]
fn the_bundle_lookup_follows_bndl_fref_appl_to_the_family_id() {
    let fork = ResFork::parse(app_bundle(b"TEST").build()).unwrap();
    assert_eq!(app_family_id(&fork, Some(*b"TEST")), Ok(128), "APPL's local 0 maps to family 128, not the document's 129");
    assert_eq!(app_family_id(&fork, None), Ok(128));
}

#[test]
fn the_bundle_whose_signature_is_the_creator_wins() {
    let fork = ResFork::parse(
        ForkBuilder::new()
            .add(b"BNDL", 128, None, &bndl(b"OTHR", &[(b"ICN#", &[(0, 200)]), (b"FREF", &[(0, 130)])]))
            .add(b"BNDL", 129, None, &bndl(b"MINE", &[(b"ICN#", &[(0, 300)]), (b"FREF", &[(0, 130)])]))
            .add(b"FREF", 130, None, &fref(b"APPL", 0))
            .build(),
    )
    .unwrap();
    assert_eq!(app_family_id(&fork, Some(*b"MINE")), Ok(300));
    assert_eq!(app_family_id(&fork, Some(*b"????")), Ok(200), "an unknown creator reads the bundles in order");
}

#[test]
fn a_missing_link_in_the_lookup_is_named() {
    let none = ResFork::parse(ForkBuilder::new().add(b"ICN#", 128, None, &icn_sharp(32)).build()).unwrap();
    assert_eq!(app_family_id(&none, None), Err(IconError::NoBundle));
    let doc_only = ResFork::parse(
        ForkBuilder::new().add(b"BNDL", 128, None, &bndl(b"TEST", &[(b"ICN#", &[(1, 129)]), (b"FREF", &[(1, 129)])])).add(b"FREF", 129, None, &fref(b"TEXT", 1)).build(),
    )
    .unwrap();
    assert_eq!(app_family_id(&doc_only, None), Err(IconError::NoAppFref));
    let unmapped = ResFork::parse(
        ForkBuilder::new().add(b"BNDL", 128, None, &bndl(b"TEST", &[(b"ICN#", &[(1, 129)]), (b"FREF", &[(0, 128)])])).add(b"FREF", 128, None, &fref(b"APPL", 0)).build(),
    )
    .unwrap();
    assert_eq!(app_family_id(&unmapped, None), Err(IconError::NoIconMapping(0)));
}

#[test]
fn icl8_is_the_system_table_masked_by_icn_sharp() {
    let mut icl8 = vec![0u8; 1024];
    icl8[0] = 35; // red
    icl8[31] = 255; // black
    let fork = ResFork::parse(app_bundle(b"TEST").add(b"icl8", 128, None, &icl8).add(b"ICN#", 128, None, &icn_sharp(32)).add(b"ics#", 128, None, &icn_sharp(16)).build()).unwrap();
    let img = family_image(&fork, 128).unwrap();
    assert_eq!((img.w, img.h, img.source), (32, 32, "icl8"));
    assert_eq!(px(&img, 0, 0), [0xFF, 0, 0, 0xFF]);
    assert_eq!(px(&img, 31, 0), [0, 0, 0, 0xFF]);
    assert_eq!(px(&img, 1, 0), [0xFF, 0xFF, 0xFF, 0xFF], "index 0 is white");
    assert_eq!(px(&img, 1, 31)[3], 0, "outside the mask is transparent");
}

#[test]
fn the_icns_32_bit_member_and_its_8_bit_mask_win_over_icl8() {
    let n = 128 * 128;
    let (mut r, g, b) = (vec![0x10u8; n], vec![0x20u8; n], vec![0x30u8; n]);
    r[1] = 0x99;
    // it32's four zero bytes; red = two 1-byte literals (0x10, 0x99) then runs of 0x10 (c - 125 copies, at most 130 each);
    // green and blue as literals
    let mut packed = vec![0u8; 4];
    packed.extend_from_slice(&[0, 0x10, 0, 0x99]);
    let mut rest = n - 2;
    while rest > 0 {
        let k = rest.min(130);
        if k >= 3 {
            packed.extend_from_slice(&[(k + 125) as u8, 0x10]);
        } else {
            packed.extend_from_slice(&[(k - 1) as u8]);
            packed.extend(core::iter::repeat_n(0x10u8, k));
        }
        rest -= k;
    }
    packed.extend(literals(&g));
    packed.extend(literals(&b));
    let mut alpha = vec![0xFFu8; n];
    alpha[2] = 0x40;
    r[2] = 0x10;
    let el = icns(&[(b"it32", &packed), (b"t8mk", &alpha)]);
    let fork = ResFork::parse(app_bundle(b"TEST").add(b"icns", 128, None, &el).add(b"icl8", 128, None, &[0; 1024]).add(b"ICN#", 128, None, &icn_sharp(32)).build()).unwrap();
    let img = family_image(&fork, 128).unwrap();
    assert_eq!((img.w, img.source), (128, "it32+t8mk"));
    assert_eq!(px(&img, 0, 0), [0x10, 0x20, 0x30, 0xFF]);
    assert_eq!(px(&img, 1, 0), [0x99, 0x20, 0x30, 0xFF]);
    assert_eq!(px(&img, 2, 0), [r[2], 0x20, 0x30, 0x40], "alpha from t8mk");
    assert_eq!(px(&img, 127, 127), [0x10, 0x20, 0x30, 0xFF]);
}

#[test]
fn a_truncated_32_bit_member_is_skipped_for_the_next_one() {
    let el = icns(&[(b"il32", &[0x7F, 1, 2])]); // a 128-byte literal with 2 bytes behind it
    let fork = ResFork::parse(app_bundle(b"TEST").add(b"icns", 128, None, &el).add(b"ICN#", 128, None, &icn_sharp(32)).build()).unwrap();
    let img = family_image(&fork, 128).unwrap();
    assert_eq!(img.source, "ICN#");
    assert_eq!(px(&img, 0, 0), [0, 0, 0, 0xFF], "black where the icon plane is set");
    assert_eq!(px(&img, 31, 0), [0xFF, 0xFF, 0xFF, 0xFF], "white inside the mask");
    assert_eq!(px(&img, 31, 31)[3], 0, "transparent outside it");
}

#[test]
fn a_standalone_member_resource_and_unpacked_argb_are_read() {
    let n = 32 * 32;
    let argb: Vec<u8> = (0..n).flat_map(|_| [0u8, 1, 2, 3]).collect();
    let fork = ResFork::parse(app_bundle(b"TEST").add(b"il32", 128, None, &argb).add(b"ICN#", 128, None, &icn_sharp(32)).build()).unwrap();
    let img = family_image(&fork, 128).unwrap();
    assert_eq!((img.w, img.source), (32, "il32"));
    assert_eq!(px(&img, 0, 0), [1, 2, 3, 0xFF], "inside the ICN# mask");
    assert_eq!(px(&img, 0, 31), [1, 2, 3, 0], "no 8-bit mask: the 1-bit one");
}

#[test]
fn icl4_reads_the_high_nibble_first() {
    let mut icl4 = vec![0u8; 512];
    icl4[0] = 0x3F; // pixel 0 red (3), pixel 1 black (15)
    let fork = ResFork::parse(app_bundle(b"TEST").add(b"icl4", 128, None, &icl4).build()).unwrap();
    let img = family_image(&fork, 128).unwrap();
    assert_eq!(img.source, "icl4");
    assert_eq!((px(&img, 0, 0), px(&img, 1, 0)), ([0xDD, 0x08, 0x06, 0xFF], [0, 0, 0, 0xFF]), "no mask member: opaque");
}

#[test]
fn a_family_with_nothing_readable_is_none_and_app_icon_names_it() {
    let fork = app_bundle(b"TEST").add(b"icl8", 128, None, &[0; 10]).build();
    assert!(family_image(&ResFork::parse(fork.clone()).unwrap(), 128).is_none());
    let file = macbinary("App", b"APPL", b"TEST", &[], &fork);
    assert_eq!(app_icon(&file), Err(IconError::NoFamily(128)));
}

#[test]
fn app_icon_reads_a_macbinary_application() {
    let mut icl8 = vec![0u8; 1024];
    icl8[0] = 255;
    let fork = app_bundle(b"TEST").add(b"icl8", 128, None, &icl8).add(b"ICN#", 128, None, &icn_sharp(32)).build();
    let img = app_icon(&macbinary("App", b"APPL", b"TEST", b"data", &fork)).unwrap();
    assert_eq!((img.w, img.source, px(&img, 0, 0)), (32, "icl8", [0, 0, 0, 0xFF]));
    assert!(matches!(app_icon(b"not a mac file"), Err(IconError::Container(_) | IconError::Fork(_))));
}

#[test]
fn the_run_length_scheme() {
    assert_eq!(unpack(&[0x80, 7], 3), Some(vec![7, 7, 7]));
    assert_eq!(unpack(&[1, 5, 6, 0x81, 9], 6), Some(vec![5, 6, 9, 9, 9, 9]));
    assert_eq!(unpack(&[2, 5], 3), None, "a literal past the end");
    assert_eq!(unpack(&[0x80], 3), None, "a run without its byte");
}
