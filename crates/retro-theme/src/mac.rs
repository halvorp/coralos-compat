//! The classic Mac role map. TODO (no Mac code yet; the mac68k engine adopts it after the Amiga one is proven): QuickDraw
//! draws in black and white (1-bit) or the Color Manager's default 4/8-bit CLUT, so the Gute Form colours reach a Mac guest
//! through the colour ports only; a 1-bit GrafPort keeps black/white and gets the Gute Form UI font and bevel geometry.
//!
//! | Mac system colour / pattern            | Gute Form role ([`crate::tokens::GuteForm`]) |
//! |----------------------------------------|-----------------------------------------------|
//! | window content background (white)      | `surface_hi` (fields, sheets) or `surface`    |
//! | window frame / title bar fill          | `surface`                                     |
//! | black ink (text, frames)               | `text`                                        |
//! | menu bar / dialog 50% gray pattern     | `surface_press` (flat, no dither)             |
//! | highlight (hilite colour, `HiliteMode`)| `accent` fill, `text_on_accent` text          |
//! | disabled item gray                     | `text_faint`                                  |
//! | control frame, scroll bar track        | `border`, `inset`                             |
//! | Chicago / Geneva system font           | [`crate::uifont`] (bitmap) at the size asked  |
//!
//! Open questions for that step: the CLUT's index 0 (white) and 255 (black) are hard-wired in QuickDraw, so they stay;
//! the Mac theme is applied to indices the toolbox owns (the 8 system CLUT slots), as [`crate::amiga`] does for pens 0..8.
