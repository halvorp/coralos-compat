//! The three regions a WindowRecord carries (Inside Macintosh I "The Window Manager"): `strucRgn` (114: the whole window, frame and
//! title bar included), `contRgn` (118: the content) and `updateRgn` (122: what needs redrawing), all in GLOBAL coordinates, each a real
//! region handle a program may read (`PtInRgn(pt, (WindowPeek(w))->contRgn)`, `CopyRgn(contRgn, ...)`, `RectInRgn`). Programs build their
//! hit tests, drag limits and clip regions out of them; with the fields NIL SimpleText 1.3 copied NIL regions and WriteNow called
//! `PtInRgn` on one.
//!
//! The frame is the host's, so the structure region is a model: the content rectangle grown by the 1-pixel frame and, for a titled window,
//! the 18-row title bar above it (the same geometry `FindWindow` uses, toolbox.rs). The regions are rewritten whenever the window's
//! rectangle or its update region changes, and freed with the window (`qd::lifecycle::window_disposed`).
use crate::machine::Mac;
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_raster::{Rect, Region};

pub const WR_STRUC_RGN: u32 = 114;
pub const WR_CONT_RGN: u32 = 118;
pub const WR_UPDATE_RGN: u32 = 122;
const TITLE_BAR: i32 = 18;

/// Does a window of this `procID` have a title bar? Asked of the ONE WDEF binding table ([`crate::toolbox::role_of`]): a document
/// has a title bar, a dialog box does not, and a procID the table does not bind is a named refusal, never a guess from its bits.
fn titled(proc_id: u16) -> Result<bool, Refusal> { Ok(crate::toolbox::role_of(proc_id)?.0 == guestui_abi::WindowRole::Document) }

impl<U: GuestUi> Mac<U> {
    /// Store `g` in the region handle held at `win + off`, making the handle if the record has none (or a stale one).
    fn put_window_region(&mut self, win: u32, off: u32, g: &Region) -> Result<(), Refusal> {
        let e = Refusal::mem("WindowRecord");
        let h = self.mem.read_u32(win.wrapping_add(off)).map_err(&e)?;
        if h != 0 && self.mm.block(h).is_some() { return self.write_region(h, g); }
        let h = self.new_region(g)?;
        self.mem.write_u32(win.wrapping_add(off), h).map_err(&e)
    }

    /// Rewrite `strucRgn`, `contRgn` and `updateRgn` of `win` from its rectangle and its update region.
    pub fn sync_window_regions(&mut self, win: u32) -> Result<(), Refusal> {
        let Some(w) = self.windows.get(&win) else { return Ok(()) };
        let r = Rect::new(i32::from(w.rect.left), i32::from(w.rect.top), i32::from(w.rect.right), i32::from(w.rect.bottom));
        let top = if titled(w.proc_id)? { TITLE_BAR + 1 } else { 1 };
        let struc = Region::from_rect(Rect::new(r.x0 - 1, r.y0 - top, r.x1 + 1, r.y1 + 1));
        let mut upd = self.qd.update.get(&win).cloned().unwrap_or_default();
        upd.translate(r.x0, r.y0); // local -> global: the content's local origin is (0, 0)
        self.put_window_region(win, WR_STRUC_RGN, &struc)?;
        self.put_window_region(win, WR_CONT_RGN, &Region::from_rect(r))?;
        self.put_window_region(win, WR_UPDATE_RGN, &upd)
    }

    /// The window is going: free the three region handles.
    pub(crate) fn free_window_regions(&mut self, win: u32) {
        for off in [WR_STRUC_RGN, WR_CONT_RGN, WR_UPDATE_RGN] {
            if let Ok(h) = self.mem.read_u32(win.wrapping_add(off)) { if h != 0 && self.mm.block(h).is_some() { let _ = self.dispose_handle(h); } }
        }
    }
}
