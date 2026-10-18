//! The content-plane fallback (RETRO-NATIVE-IS-A-MEASURED-CLAIM, FALLBACK-IS-THE-DESIGN): a control the kit has no part for
//! (a `procID` with no row in the Control Manager table, an
//! application's own CDEF) must never just vanish. The engine draws it the way the Control Manager's default definition would
//! (a frame and the title) into the owner window's port, through the SAME QuickDraw path the program's own drawing
//! takes, so it lands in the window's content plane, the route census counts it as replayed, and the program can read the
//! pixels back. It is the role's default look, not a guess at an application's art: a custom CDEF's own drawing is not run.
use super::controls::has_kit_part;
use crate::machine::Mac;
use crate::qd::draw::paint;
use crate::qd::port::{raster, set_pen_loc, PAT_COPY};
use crate::qd::shapes::{rect, Shape};
use crate::qd::text::draw_on;
use crate::Refusal;
use guestui_abi::GuestUi;

const WHITE: [u8; 8] = [0; 8];
const BLACK: [u8; 8] = [0xFF; 8];
const DIMMED: [u8; 8] = [0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55];
/// systemless/IM: the default font draws 8 rows tall (ascent 7, descent 1).
const GLYPH_H: i32 = 8;

impl<U: GuestUi> Mac<U> {
    /// Draw (or, when hidden, erase) control `ctl` in its window's port IF the kit has no part for it.
    pub(crate) fn draw_fallback(&mut self, ctl: u32) -> Result<(), Refusal> {
        let f = self.control_facts(ctl)?;
        if has_kit_part(&f) || !self.qd.canvases.contains_key(&f.owner) { return Ok(()); }
        let port = f.owner;
        let r = raster(f.rect);
        paint(self, port, &rect(r), PAT_COPY, &WHITE)?;
        if !f.vis { return Ok(()); }
        let ink = if f.hilite == 0xFF { &DIMMED } else { &BLACK };
        let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
        let title: alloc::vec::Vec<u8> = f.title.chars().map(|c| c as u8).collect();
        let text_w = self.qd.font.text_length(&title) as i32;
        paint(self, port, &Shape::Rect.frame(r, 1, 1), PAT_COPY, ink)?;
        let (h0, v0) = (r.x0 + (w - text_w).max(0) / 2, r.y0 + (h - GLYPH_H) / 2 + GLYPH_H);
        if !title.is_empty() {
            set_pen_loc(&mut self.mem, port, v0 as i16, h0 as i16)?;
            draw_on(self, port, &title, "default control title")?;
        }
        Ok(())
    }
}
