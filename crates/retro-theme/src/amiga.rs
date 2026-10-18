//! The Amiga mapping, in ONE place: the Workbench screen's system pens 0..8 and the DrawInfo pen array both read from here.
//!
//! Many programs hard-code pens 0..3 (AmigaOS 2.x look: 0 background, 1 text/shadow, 2 shine, 3 fill), so those four keep
//! their meaning and carry the Gute Form values; pens 4..8 hold the other colours the DrawInfo pens need. The engine
//! writes [`SYSTEM_PENS`] into the screen palette (pens 0..8, never handed out as free) and [`DRI_PENS`] into `dri_Pens`.
//! Colours a guest obtains itself (ObtainPen, SetRGB4) are the guest's own and stay untouched.
use crate::tokens::{Rgb, GUTE_FORM as T};

/// Palette index of each system pen and the Gute Form role it carries.
pub const SYSTEM_PENS: [(u8, &str, Rgb); 8] = [
    (0, "surface: a window's face, BACKGROUNDPEN/BLOCKPEN", T.surface),
    (1, "text: body text and the classic 'black' ink", T.text),
    (2, "shine: the light bevel edge and white text on accent", T.shine),
    (3, "accent: selection and fill, FILLPEN", T.accent),
    (4, "border: the dark bevel edge, SHADOWPEN", T.border),
    (5, "surface_hi: a field's or sheet's fill", T.surface_hi),
    (6, "text_muted: secondary text", T.text_muted),
    (7, "charcoal: latched/current dark chrome", T.charcoal),
];

/// The screen palette entries 0..8 as packed 0x00RRGGBB.
pub const fn system_palette() -> [u32; 8] {
    let mut out = [0u32; 8];
    let mut i = 0;
    while i < 8 { out[i] = SYSTEM_PENS[i].2.packed(); i += 1; }
    out
}

// DrawInfo pen indices (intuition/screens.h `DRIPEN_*`).
pub const DETAILPEN: usize = 0;
pub const BLOCKPEN: usize = 1;
pub const TEXTPEN: usize = 2;
pub const SHINEPEN: usize = 3;
pub const SHADOWPEN: usize = 4;
pub const FILLPEN: usize = 5;
pub const FILLTEXTPEN: usize = 6;
pub const BACKGROUNDPEN: usize = 7;
pub const HIGHLIGHTTEXTPEN: usize = 8;
pub const BARDETAILPEN: usize = 9;
pub const BARBLOCKPEN: usize = 10;
pub const BARTRIMPEN: usize = 11;
pub const NUMDRIPENS: usize = 12;

/// `dri_Pens[DRIPEN]` -> palette index. Gute Form's reading of each pen:
/// * DETAILPEN / TEXTPEN / BARDETAILPEN: text; BLOCKPEN / BACKGROUNDPEN / BARBLOCKPEN: the window surface (Gute Form's
///   windows and their title bars share ONE light face, focus shows as a subtle accent, not as a fill);
/// * SHINEPEN: the light edge; SHADOWPEN / BARTRIMPEN: the border colour (a bevel's dark edge and the bar's rule);
/// * FILLPEN: the accent (selection); FILLTEXTPEN: white on it; HIGHLIGHTTEXTPEN: body text (a highlighted row on the
///   kit's pressed surface keeps `text1`).
pub const DRI_PENS: [u16; NUMDRIPENS] = [1, 0, 1, 2, 4, 3, 2, 0, 1, 1, 0, 4];

/// Name of each DrawInfo pen, in index order (for logs and tests).
pub const DRI_PEN_NAMES: [&str; NUMDRIPENS] = ["DETAIL", "BLOCK", "TEXT", "SHINE", "SHADOW", "FILL", "FILLTEXT", "BACKGROUND", "HIGHLIGHTTEXT", "BARDETAIL", "BARBLOCK", "BARTRIM"];

/// The colour the DrawInfo pen `dri` resolves to through the system palette.
pub const fn dri_color(dri: usize) -> Rgb { SYSTEM_PENS[DRI_PENS[dri] as usize].2 }

/// The AROS defaults this replaces (`intuition_misc.c` DriPens4 and the engine's old system palette): a negative control for
/// tests that must tell Gute Form from the AROS look.
pub const AROS_DRI_PENS: [u16; NUMDRIPENS] = [1, 0, 1, 2, 1, 3, 1, 0, 2, 1, 2, 1];
pub const AROS_SYSTEM_PALETTE: [u32; 8] = [0xAAAAAA, 0x000000, 0xFFFFFF, 0x6688BB, 0x999999, 0xBBBBBB, 0xEEBB99, 0x4488DD];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_coded_pens_0_to_3_are_surface_text_shine_accent() {
        let p = system_palette();
        assert_eq!([p[0], p[1], p[2], p[3]], [T.surface.packed(), T.text.packed(), T.shine.packed(), T.accent.packed()]);
    }
    #[test]
    fn every_dri_pen_points_into_the_system_pens_and_only_white_is_shared_with_aros() {
        let p = system_palette();
        for (i, pen) in DRI_PENS.iter().enumerate() { assert!((*pen as usize) < 8, "{}", DRI_PEN_NAMES[i]); }
        // control: the AROS palette is NOT Gute Form, in any entry but pen 2 (white is white in both)
        for i in (0..8).filter(|&i| i != 2) { assert_ne!(p[i], AROS_SYSTEM_PALETTE[i], "pen {i}"); }
        assert_eq!(p[2], AROS_SYSTEM_PALETTE[2]);
    }
    #[test]
    fn the_roles_read_as_intended() {
        assert_eq!(dri_color(FILLPEN), T.accent);
        assert_eq!(dri_color(FILLTEXTPEN), T.text_on_accent);
        assert_eq!(dri_color(BACKGROUNDPEN), T.surface);
        assert_eq!(dri_color(TEXTPEN), T.text);
        assert_eq!(dri_color(SHADOWPEN), T.border);
        assert_ne!(dri_color(TEXTPEN), dri_color(BACKGROUNDPEN), "text must be readable on the background");
    }
    #[test]
    fn the_pen_table_indices_are_in_order() {
        for (i, (idx, _, _)) in SYSTEM_PENS.iter().enumerate() { assert_eq!(*idx as usize, i); }
    }
}
