//! The Gute Form palette by role. Every value is `strata_theme.rs::palette::<NAME>` of the canonical kit (see the crate
//! doc); the line numbers are that file's.

/// 8-bit RGB. `packed()` is the engines' canvas format (0x00RRGGBB).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn packed(self) -> u32 { (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32 }
}

/// The theme: colours by role. Alpha-bearing kit tokens (glaze, hairline, shade) are not here: a guest's pens are opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuteForm {
    /// The desktop behind windows. `palette::BACKGROUND` (line 53).
    pub desktop: Rgb,
    /// A window's face: the flat surface everything sits on. `STONE` (55); the kit's WINDOW recipe fills with it.
    pub surface: Rgb,
    /// Raised, not pressable: cards, groups. `SURFACE` (56).
    pub surface_raised: Rgb,
    /// A field's / content sheet's fill (lightest surface). `SURFACE_HI` (57).
    pub surface_hi: Rgb,
    /// A pressed key, a chosen sidebar row. `SURFACE_PRESS` (58).
    pub surface_press: Rgb,
    /// Sunken wells and the dark side of a bevel. `INSET` (59).
    pub inset: Rgb,
    /// Body text. `TEXT1` (60).
    pub text: Rgb,
    /// Secondary text. `TEXT2` (61).
    pub text_muted: Rgb,
    /// Tertiary text, hints. `TEXT3` (62).
    pub text_faint: Rgb,
    /// Text on an accent fill. `TEXT_ON_ACCENT` (63).
    pub text_on_accent: Rgb,
    /// 1 px borders and rings (the dark edge of a bevel). `BORDER` (64).
    pub border: Rgb,
    /// Hairlines between rows. `BORDER_SOFT` (65).
    pub border_soft: Rgb,
    /// Signal orange: selection fill, focus border, the one thing to press. `ACCENT` (66).
    pub accent: Rgb,
    pub accent_hover: Rgb,
    pub accent_press: Rgb,
    /// The light edge of a bevel and white text. `RIM` (76).
    pub shine: Rgb,
    /// A latched/current key, dark chrome. `CHARCOAL` (84).
    pub charcoal: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
}

/// `strata_theme.rs` palette, lines 53-84.
pub const GUTE_FORM: GuteForm = GuteForm {
    desktop: Rgb(220, 207, 197),
    surface: Rgb(239, 230, 223),
    surface_raised: Rgb(244, 236, 230),
    surface_hi: Rgb(250, 245, 241),
    surface_press: Rgb(233, 223, 215),
    inset: Rgb(211, 198, 188),
    text: Rgb(34, 27, 24),
    text_muted: Rgb(95, 85, 80),
    text_faint: Rgb(136, 124, 118),
    text_on_accent: Rgb(255, 255, 255),
    border: Rgb(209, 195, 184),
    border_soft: Rgb(229, 218, 210),
    accent: Rgb(204, 74, 24),
    accent_hover: Rgb(214, 85, 34),
    accent_press: Rgb(178, 63, 18),
    shine: Rgb(255, 255, 255),
    charcoal: Rgb(48, 41, 38),
    success: Rgb(78, 138, 58),
    warning: Rgb(211, 155, 28),
    error: Rgb(184, 51, 42),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kit_values_are_what_the_specimen_css_says() {
        // CoralOS_UIKit.html line 1524: --background:#DCCFC5 --stone:#EFE6DF --surface:#F4ECE6 --surface-hi:#FAF5F1 --surface-press:#E9DFD7
        let t = GUTE_FORM;
        assert_eq!([t.desktop, t.surface, t.surface_raised, t.surface_hi, t.surface_press].map(Rgb::packed), [0xDCCFC5, 0xEFE6DF, 0xF4ECE6, 0xFAF5F1, 0xE9DFD7]);
        assert_eq!(t.accent.packed(), 0xCC4A18, "gadget.library's default accent");
    }
    #[test]
    fn packed_is_rrggbb_and_distinguishes_values() {
        assert_eq!(Rgb(1, 2, 3).packed(), 0x010203);
        assert_ne!(GUTE_FORM.surface.packed(), GUTE_FORM.surface_raised.packed(), "control: neighbouring surfaces are different tokens");
    }
}
