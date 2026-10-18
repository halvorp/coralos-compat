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
}
