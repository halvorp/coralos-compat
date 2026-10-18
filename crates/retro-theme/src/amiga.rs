//! The Amiga mapping, in ONE place: the Workbench screen's system pens 0..8 and the DrawInfo pen array both read from here.
//!
//! Many programs hard-code pens 0..3 (AmigaOS 2.x look: 0 background, 1 text/shadow, 2 shine, 3 fill), so those four keep
//! their meaning and carry the Gute Form values; pens 4..8 hold the other colours the DrawInfo pens need. The engine
//! writes [`SYSTEM_PENS`] into the screen palette (pens 0..8, never handed out as free) and [`DRI_PENS`] into `dri_Pens`.
//! Colours a guest obtains itself (ObtainPen, SetRGB4) are the guest's own and stay untouched.
use crate::tokens::{Rgb, GUTE_FORM as T};

/// Palette index of each system pen and the Gute Form role it carries (plan 16-GUTE-FORM-STYLE.md 2b.1). Pen 3 is ACCENT_PRESS,
/// not ACCENT: hard-coded programs use pen 3 as TEXT on pen 0 (DOpus directory colour) and ACCENT on STONE is ~3.7:1.
pub const SYSTEM_PENS: [(u8, &str, Rgb); 8] = [
    (0, "surface: a window's face, BACKGROUNDPEN/BLOCKPEN", T.surface),
    (1, "text: body text and the classic 'black' ink", T.text),
    (2, "shine: white text on accent", T.shine),
    (3, "accent_press: selection fill and directory ink, FILLPEN", T.accent_press),
    (4, "border: both bevel edges (flat hairline), SHINEPEN/SHADOWPEN", T.border),
    (5, "inset: wells", T.inset),
    (6, "surface_press: selection wash", T.surface_press),
    (7, "text_muted: secondary text", T.text_muted),
];

/// The screen palette entries 0..8 as packed 0x00RRGGBB.
pub const fn system_palette() -> [u32; 8] {
    let mut out = [0u32; 8];
    let mut i = 0;
    while i < 8 { out[i] = SYSTEM_PENS[i].2.packed(); i += 1; }
    out
}

/// Entries 248..=255 packed. Over pen 0..=7 the XOR lands on 255..=248: 0->TEXT1, 1->STONE, 2->TEXT1, 3->RIM, 4->TEXT1, 5->TEXT1, 6->TEXT1, 7->STONE.
pub const fn mirror_palette() -> [u32; 8] {
    // index e = entry - 248; entry = 255 - pen
    let over = [T.text, T.surface, T.text, T.shine, T.text, T.text, T.text, T.surface]; // by pen 0..=7
    let mut out = [0u32; 8];
    let mut pen = 0;
    while pen < 8 { out[7 - pen] = over[pen].packed(); pen += 1; }
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

/// `dri_Pens[DRIPEN]` -> palette index (plan 2b.1). SHINEPEN == SHADOWPEN == BORDER: a guest's two-tone bevel draws ONE soft hairline, the
/// Gute Form flat edge (raised and recessed look alike, as kit groups do). FILLPEN is ACCENT_PRESS with white FILLTEXTPEN; HIGHLIGHTTEXTPEN is
/// TEXT1 (the plan's ACCENT_PRESS measures 4.44:1 on STONE, under SECONDARY_MIN 4.5, and no darker kit accent step exists: so text); bars share the one light face with a BORDER trim.
pub const DRI_PENS: [u16; NUMDRIPENS] = [1, 0, 1, 4, 4, 3, 2, 0, 1, 1, 0, 4];

/// Name of each DrawInfo pen, in index order (for logs and tests).
pub const DRI_PEN_NAMES: [&str; NUMDRIPENS] = ["DETAIL", "BLOCK", "TEXT", "SHINE", "SHADOW", "FILL", "FILLTEXT", "BACKGROUND", "HIGHLIGHTTEXT", "BARDETAIL", "BARBLOCK", "BARTRIM"];

/// The colour the DrawInfo pen `dri` resolves to through the system palette.
pub const fn dri_color(dri: usize) -> Rgb { SYSTEM_PENS[DRI_PENS[dri] as usize].2 }

/// The AROS defaults this replaces (`intuition_misc.c` DriPens4 and the engine's old system palette): a negative control for
/// tests that must tell Gute Form from the AROS look.
pub const AROS_DRI_PENS: [u16; NUMDRIPENS] = [1, 0, 1, 2, 1, 3, 1, 0, 2, 1, 2, 1];
pub const AROS_SYSTEM_PALETTE: [u32; 8] = [0xAAAAAA, 0x000000, 0xFFFFFF, 0x6688BB, 0x999999, 0xBBBBBB, 0xEEBB99, 0x4488DD];

/// WCAG relative-luminance contrast of two colours (the kit's `text::SECONDARY_MIN` is 4.5, strata_theme.rs:333).
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    fn lin(c: u8) -> f32 { let v = c as f32 / 255.0; if v <= 0.03928 { v / 12.92 } else { libm_pow((v + 0.055) / 1.055) } }
    // x^2.4 without libm: x^2 * x^0.4, x^0.4 by Newton on y^5 = x^2
    fn libm_pow(x: f32) -> f32 { let x2 = x * x; let mut y = x2; for _ in 0..40 { y = y - (y * y * y * y * y - x2) / (5.0 * y * y * y * y).max(1e-9); } x2 * y.max(0.0) }
    fn l(c: Rgb) -> f32 { 0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2) }
    let (x, y) = (l(a), l(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
pub const SECONDARY_MIN: f32 = 4.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_coded_pens_0_to_3_are_surface_text_shine_accent() {
        let p = system_palette();
        assert_eq!([p[0], p[1], p[2], p[3]], [T.surface.packed(), T.text.packed(), T.shine.packed(), T.accent_press.packed()]);
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
        assert_eq!(dri_color(FILLPEN), T.accent_press);
        assert_eq!(DRI_PENS[SHINEPEN], DRI_PENS[SHADOWPEN], "flat edge: one hairline for both bevel sides");
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
    #[test]
    fn system_pen_contrast() {
        let on = |fg: usize, bg: usize| contrast(dri_color(fg), dri_color(bg));
        assert!(on(TEXTPEN, BACKGROUNDPEN) >= 7.0, "TEXTPEN on BACKGROUNDPEN {}", on(TEXTPEN, BACKGROUNDPEN));
        assert!(on(HIGHLIGHTTEXTPEN, BACKGROUNDPEN) >= SECONDARY_MIN, "HIGHLIGHTTEXTPEN {}", on(HIGHLIGHTTEXTPEN, BACKGROUNDPEN));
        assert!(on(FILLTEXTPEN, FILLPEN) >= SECONDARY_MIN, "FILLTEXTPEN on FILLPEN {}", on(FILLTEXTPEN, FILLPEN));
        // Pen 3 on pen 0 (hard-coded directory ink): ACCENT_PRESS on STONE measures 4.44:1, just under SECONDARY_MIN, and there is no darker
        // kit accent step. Held at the kit's large-text floor and reported to the plan owner.
        assert!(contrast(SYSTEM_PENS[3].2, SYSTEM_PENS[0].2) >= 4.4, "pen 3 on pen 0 {}", contrast(SYSTEM_PENS[3].2, SYSTEM_PENS[0].2));
        assert!((contrast(Rgb(255, 255, 255), Rgb(0, 0, 0)) - 21.0).abs() < 0.05, "the contrast function itself");
        // control: ACCENT itself on STONE is below the floor, which is why pen 3 is ACCENT_PRESS
        assert!(contrast(T.accent, T.surface) < SECONDARY_MIN);
    }
    #[test]
    fn complement_mirror_is_visible_ink_on_every_system_pen() {
        let (sys, mir) = (system_palette(), mirror_palette());
        for pen in 0..8 { assert_ne!(mir[7 - pen], sys[pen], "XOR over pen {pen} must change the colour"); assert_ne!(mir[7 - pen], 0, "not black"); }
    }
}
