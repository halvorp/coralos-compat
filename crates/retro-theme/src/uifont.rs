//! The Gute Form UI face as a classic Amiga bitmap font: Nunito Sans Regular (the kit's `--font-ui`), rasterised 1-bit at
//! 12 px (the smallest size of the kit's UI scale) by `tools/gen-gute-form-font.py` into `uifont_data.rs`.
//!
//! WHY 12 px (YSize 11, baseline 8): the kit sanctions 12/14/16/20/24 for UI text, and 12 is the smallest. Below it a 1-bit
//! rendering of a humanist sans loses its x-height (at 10 px the lower case collapsed to 4 rows and spaced unevenly
//! in the experiment of 2026-10-18); well-behaved Amiga programs size everything from `tf_YSize`/`tf_XSize`, and the ones
//! that hard-code an 8-px line get a line three pixels taller, not unreadable text. The face is PROPORTIONAL: `tf_CharSpace`
//! and `tf_CharKern` carry the real advances and left bearings, so TextLength is the true width of what is drawn.
//! `tf_XSize` is the advance of '0' (the kit's `ch`).
//!
//! Licence: Nunito Sans is SIL OFL 1.1, Copyright 2016 The Nunito Sans Project Authors; the bitmap strip is a derivative
//! work under the same licence (LICENSES.md, NOTICE). It is data only.
pub use crate::uifont_data::{BASELINE, BOLD_SMEAR, CHAR_DATA, CHAR_KERN, CHAR_LOC, CHAR_SPACE, EM_PX, HI_CHAR, LO_CHAR, MODULO, X_SIZE, Y_SIZE};

/// The name a guest sees (`ln_Name` of the TextFont, `ta_Name` of a TextAttr naming it).
pub const NAME: &str = "guteform.font";
/// The name AmigaOS programs ask for when they want "the system font": kept resident as its own 8x8 fixed face, see the engine.
pub const TOPAZ_NAME: &str = "topaz.font";

/// The glyph strip is `Y_SIZE` rows of `MODULO` bytes; `CHAR_LOC`/`CHAR_SPACE`/`CHAR_KERN` have `HI - LO + 2` entries
/// (the last is the default glyph).
pub const fn glyph_slots() -> usize { (HI_CHAR - LO_CHAR) as usize + 2 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_consistent_with_the_strip() {
        assert_eq!(CHAR_DATA.len(), Y_SIZE as usize * MODULO as usize);
        assert_eq!((CHAR_LOC.len(), CHAR_SPACE.len(), CHAR_KERN.len()), (glyph_slots(), glyph_slots(), glyph_slots()));
        let end = CHAR_LOC.iter().map(|l| (l >> 16) + (l & 0xFFFF)).max().unwrap();
        assert!(end <= MODULO as u32 * 8, "every glyph lies inside the strip");
        assert!(BASELINE < Y_SIZE);
    }
    #[test]
    fn it_is_proportional() {
        let w = |c: u8| CHAR_SPACE[(c - LO_CHAR) as usize] + CHAR_KERN[(c - LO_CHAR) as usize];
        assert!(w(b'i') < w(b'm'), "i {} < m {}", w(b'i'), w(b'm'));
        assert!(w(b' ') > 0);
        assert_eq!(w(b'0') as u16, X_SIZE);
    }

    /// Ink rows of glyph `c` as a bitmask per strip row (bit x = column x of the glyph).
    fn ink_rows(c: u8) -> [u32; Y_SIZE as usize] {
        let loc = CHAR_LOC[(c - LO_CHAR) as usize];
        let (off, w) = ((loc >> 16) as usize, (loc & 0xFFFF) as usize);
        core::array::from_fn(|y| (0..w).filter(|x| CHAR_DATA[y * MODULO as usize + (off + x) / 8] >> (7 - (off + x) % 8) & 1 != 0).fold(0, |m, x| m | 1 << x))
    }
    fn top_bottom(c: u8) -> (usize, usize) {
        let r = ink_rows(c);
        (r.iter().position(|&m| m != 0).unwrap(), r.iter().rposition(|&m| m != 0).unwrap())
    }

    /// `tf_Baseline` is the row of the last ink row above the baseline, so every glyph with no descender ends exactly on it
    /// and the plain ascenders (which define the strip's height above the baseline) start on row 0. Before 2026-10-19 the
    /// generator mapped strip rows one scratch row too low, which dropped the top ink row of every ascender and capital.
    #[test]
    fn ascenders_and_capitals_span_row_0_or_cap_height_to_the_baseline_row() {
        for c in *b"bdhkl" {
            assert_eq!(top_bottom(c), (0, BASELINE as usize), "{}", c as char);
        }
        for c in b'A'..=b'Z' {
            let (top, bottom) = top_bottom(c);
            if !b"JQ".contains(&c) { assert_eq!(bottom, BASELINE as usize, "{} sits on the baseline row", c as char); }
            // Nunito Sans cap height is 705/1000 em: 8.5 px at 12 px, so 8 or 9 rows, never the 7 a clipped bar leaves.
            let height = BASELINE as usize + 1 - top;
            assert!((8..=9).contains(&height), "{} is {height} rows tall", c as char);
        }
    }

    #[test]
    fn the_bars_of_t_f_e_and_z_are_drawn() {
        for c in *b"TFEZ" {
            let rows = ink_rows(c);
            let top = rows.iter().find(|&&m| m != 0).unwrap();
            assert!(top.count_ones() >= 4, "{} has a bar in its top ink row, got {top:#b}", c as char);
        }
    }
}
