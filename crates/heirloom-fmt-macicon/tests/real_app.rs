//! A real application: the owner's Warcraft III (Mac PowerPC, MacBinary). It is NOT in this repository and never will be (corpus
//! test inputs never ship). Opt-in: `cargo test -- --ignored` with `HEIRLOOM_MACICON_CORPUS` naming the file (default: the corpus
//! lane's copy on the dev host). Facts asserted were measured on that file on 2026-11-03: BNDL 128 -> FREF 'APPL' -> family 128,
//! whose 'icns' holds it32 + t8mk (the 128 px orc portrait).
use heirloom_fmt_macicon::app_icon;

#[test]
#[ignore = "reads the owner's Warcraft III (HEIRLOOM_MACICON_CORPUS); run with --ignored on a host that has it"]
fn warcraft_iii_shows_its_128_px_icns_portrait() {
    let path = std::env::var("HEIRLOOM_MACICON_CORPUS").unwrap_or_else(|_| "/data/build/corpus-bin/mac/warcraft3/apps/roc-Warcraft_III.bin".into());
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e} (set HEIRLOOM_MACICON_CORPUS to the Warcraft III MacBinary)"));
    let img = app_icon(&bytes).unwrap();
    assert_eq!((img.w, img.h, img.source), (128, 128, "it32+t8mk"));
    let centre = &img.rgba[(64 * 128 + 64) * 4..][..4];
    assert_eq!(centre[3], 0xFF, "the portrait is opaque in the middle");
    let mut colours: Vec<[u8; 4]> = img.rgba.chunks(4).map(|p| p.try_into().unwrap()).collect();
    colours.sort_unstable();
    colours.dedup();
    assert!(colours.len() > 1000, "a painted 32-bit portrait, not a flat or mis-decoded plane: {} colours", colours.len());
}
