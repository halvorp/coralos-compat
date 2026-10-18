//! The QuickDraw oracle application (fixtures/testapp/oracle.s), run on this engine, against the cells recorded from systemless
//! (an independent classic-Mac runtime, run as an external binary; see tests/oracle/systemless/README.md to reproduce).
//!
//! `cells.txt` holds, per 40x40 cell, an FNV-1a hash of the oracle's 1-bit pixels (`oracle=`) and of ours when it was recorded
//! (`ours=`). The test re-draws the window, and requires (1) our cell hashes to be exactly the recorded ones (a regression
//! gate for every primitive) and (2) our hash to EQUAL the oracle's for every cell except [`DIVERGENCES`], each of which
//! says why we side with Atkinson's QuickDraw source and not with systemless.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::fixture::{oracle_app_macbinary, ORACLE_H, ORACLE_W};
use mac68k_engine::Mac;
use std::collections::BTreeMap;

const CELLS: &str = include_str!("oracle/systemless/cells.txt");

/// (row, col) -> why mac68k-engine and systemless differ and who is right.
const DIVERGENCES: &[((usize, usize), &str)] = &[
    ((0, 2), "patXor line: systemless stamps the pen at every step, so pixels the 3x2 pen covers twice are XORed twice; DrawLine.a draws one slab per row, every pixel once"),
    ((0, 4), "notPatCopy: systemless draws modes 12..15 as patCopy (the oracle row 0 cells 4..7 are byte-identical to cell 0); DrawLine.a handles the invert bit (BCLR #2 / NOT.L D7)"),
    ((0, 5), "notPatOr: as above"),
    ((0, 6), "notPatXor: as above"),
    ((0, 7), "notPatBic: as above"),
    ((4, 6), "StretchBits shrink: Stretch.a ORs the source pixels that fall in one destination pixel and starts its DDA half a step in (verror = -srcHeight/2); systemless samples one pixel"),
    ((6, 7), "the 45-degree '\\' line: DrawLine.a sends slope == 1.0 down the BGE branch (right edge trimmed), systemless one pixel right of that and past the end point"),
    ((5, 0), "text: the stand-in font differs by design (topaz8 for Chicago); placement is checked in text_placement_matches_the_oracle"),
    ((5, 1), "text"), ((5, 2), "text"), ((5, 3), "text"), ((5, 4), "text"), ((5, 5), "text"), ((5, 6), "text"), ((5, 7), "text"),
];

fn fnv(bits: impl Iterator<Item = bool>) -> u64 {
    bits.fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3))
}

fn our_window() -> (i32, Vec<bool>) {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.launch(&oracle_app_macbinary()).unwrap();
    let r = m.run_loop(50_000_000).unwrap();
    assert!(r.stats.blocked && m.refused.is_empty(), "the oracle app runs to its event loop with no refusal: {:?} {:?}", r.stats, m.refused);
    let c = m.qd.canvases.values().next().expect("the window canvas");
    assert_eq!((c.width(), c.pixels().len() as i32 / c.width()), (i32::from(ORACLE_W), i32::from(ORACLE_H)));
    (c.width(), c.pixels().iter().map(|&p| p == 1).collect())
}

fn recorded() -> BTreeMap<(usize, usize), (String, u64, u64)> {
    CELLS.lines().filter(|l| !l.starts_with("text slot")).map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let hex = |k: &str| u64::from_str_radix(f.iter().find_map(|s| s.strip_prefix(k)).unwrap(), 16).unwrap();
        ((f[0].parse().unwrap(), f[1].parse().unwrap()), (f[2].to_string(), hex("oracle="), hex("ours=")))
    }).collect()
}

fn cell_hash(w: i32, px: &[bool], r: usize, c: usize) -> u64 {
    fnv((0..40).flat_map(|y| (0..40).map(move |x| (x, y))).map(|(x, y)| px[((r * 40 + y) * w as usize) + c * 40 + x]))
}

#[test]
fn every_cell_matches_systemless_except_the_documented_divergences() {
    let (w, px) = our_window();
    let rec = recorded();
    assert_eq!(rec.len(), 56, "7 rows x 8 cells recorded");
    let mut agree = 0;
    for (&(r, c), (name, oracle, ours)) in &rec {
        let now = cell_hash(w, &px, r, c);
        assert_eq!(now, *ours, "{name} ({r},{c}) changed since it was recorded: re-run tests/oracle/systemless/README.md and review the diff");
        match DIVERGENCES.iter().find(|d| d.0 == (r, c)) {
            None => { assert_eq!(now, *oracle, "{name} ({r},{c}) differs from systemless and is not a documented divergence"); agree += 1; }
            Some((_, why)) => assert_ne!(now, *oracle, "{name} ({r},{c}) now AGREES with systemless: drop it from DIVERGENCES ({why})"),
        }
    }
    assert_eq!(agree, 56 - DIVERGENCES.len());
    assert!(agree >= 40, "the bulk of the primitives are pixel-identical: {agree}");
}

/// The text cells are compared by placement: the string's baseline (no-descender glyphs end on row baseline-1) and the
/// pen's advance, which the program shows with a tick under the pen's end point. topaz8 is 8 px/char; the oracle's font is proportional.
#[test]
fn text_placement_matches_the_oracle() {
    let (w, px) = our_window();
    let at = |x: usize, y: usize| px[y * w as usize + x];
    for (slot, chars) in [(0usize, 5usize), (1, 7), (2, 9), (3, 6)] {
        let x0 = slot * 80;
        let rows: Vec<usize> = (200..222).filter(|&y| (x0..x0 + 80).any(|x| at(x, y))).collect();
        let tick: Vec<usize> = (x0..x0 + 80).filter(|&x| (224..228).all(|y| at(x, y))).collect();
        assert_eq!(*rows.iter().max().unwrap() <= 218, true, "slot {slot}: ink ends at or above baseline+0 (descender row), got {rows:?}");
        assert!(rows.iter().any(|&y| y == 217), "slot {slot}: glyphs without descender end on row baseline-1 = 217 (the oracle: 217)");
        assert_eq!(tick.len(), 1, "slot {slot}: one tick");
        assert_eq!(tick[0], x0 + 4 + 8 * chars, "slot {slot}: the pen advanced {chars} cells of 8 from x {}", x0 + 4);
    }
}
