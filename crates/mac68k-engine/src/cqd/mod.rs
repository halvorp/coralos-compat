//! Color QuickDraw on the Macintosh II profile (Inside Macintosh: Imaging With QuickDraw, "Color QuickDraw"; Inside
//! Macintosh V, "Color QuickDraw"): the structures a colour program builds and reads, in REAL GUEST MEMORY at their documented
//! offsets, and the traps over them.
//!
//! * [`color`]: `RGBColor`, the standard colour tables, nearest-colour search.
//! * [`ctab`]: `ColorTable` / `CTabHandle` (`GetCTable`, `DisposeCTable`, `GetCTSeed`) and the Color Manager's
//!   `Index2Color` / `Color2Index` / `InvertColor` / `RealColor`.
//! * [`pixmap`]: `PixMap` / `PixMapHandle` records and reading/writing pixels of a packed guest pixel map (1, 2, 4, 8 bits).
//! * [`gdevice`]: `GDevice` / `GDHandle`: the main screen device (8 bits deep, the standard 8-bit table),
//!   `GetMainDevice`/`GetGDevice`/`SetGDevice`/`GetDeviceList`/`GetNextDevice`/`TestDeviceAttribute`/`GetMaxDevice`.
//! * [`depth`]: `HasDepth` / `SetDepth` of the Palette Manager's dispatch trap.
//! * [`cport`]: `CGrafPort` records: `OpenCPort`, `InitCPort`, `CloseCPort`, and the colour window's port (`NewCWindow`).
//! * [`pens`]: the pen colours, `RGBForeColor`/`RGBBackColor`/`GetForeColor`/`GetBackColor` and the classic `ForeColor`.
//!
//! **One implementation of the drawing ops.** There is no colour code path beside the 1-bit one: the shared QuickDraw ops
//! ([`crate::qd`]) draw into a chunky `retro_raster::Bitmap` whose depth is 1 for a classic port and 8 for a colour one,
//! the pen is a pair of pen NUMBERS (index 1/0 in a 1-bit port; `Color2Index` of `rgbFgColor`/`rgbBkColor` in an indexed
//! one), and the Boolean transfer modes act on pixel values, which is what QuickDraw does on an indexed device (the
//! standard table puts white at 0 and black at 255 so that or/xor/bic read as black-and-white there).
//!
//! Not modelled, by name (each one is a refusal when called, never a fake): direct (16/32-bit) pixel maps, the inverse
//! table (`gdITable` is NIL: the engine answers `Color2Index` itself), device search/complement procedures, gamma, the
//! arithmetic transfer modes 32..39, colour patterns other than `patType` 0, colour icons and cursors.
pub mod cport;
pub mod color;
pub mod ctab;
pub mod depth;
pub mod gdevice;
pub mod gworld;
pub mod pens;
pub mod pixmap;

use crate::machine::Mac;
use crate::Refusal;
use guestui_abi::GuestUi;

/// Color QuickDraw's host-side state.
#[derive(Default)]
pub struct Cqd {
    /// `QDError` (the last Color QuickDraw result).
    pub qd_error: i16,
    /// The next unique colour-table seed `GetCTSeed` hands out.
    pub next_seed: u32,
    /// The main screen device (`GDHandle`), 0 until `InitGraf` runs on a machine with Color QuickDraw.
    pub main_device: u32,
    /// Every device in the device list, main first.
    pub devices: alloc::vec::Vec<u32>,
    /// PixMap record address -> the colour window's port whose pixels it describes (the canvas lives host-side).
    pub window_pix: alloc::collections::BTreeMap<u32, u32>,
    /// Every live offscreen graphics world by its port ([`gworld`]).
    pub gworlds: alloc::collections::BTreeMap<u32, gworld::Gw>,
}

/// Seeds below this are the standard tables' (their `'clut'` id); `GetCTSeed` starts above it (Imaging With QuickDraw).
pub const FIRST_UNIQUE_SEED: u32 = 1024;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    ctab::register(m)?;
    gdevice::register(m)?;
    depth::register(m)?;
    pixmap::register(m)?;
    cport::register(m)?;
    pens::register(m)
}

impl<U: GuestUi> Mac<U> {
    /// A cleared handle of `size` bytes: (handle, address of its block).
    pub(crate) fn alloc_handle(&mut self, size: u32, what: &str) -> Result<(u32, u32), Refusal> {
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, size, true).map_err(|e| Refusal(alloc::format!("{what}: no memory for {size} bytes (memFullErr {e})")))?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(alloc::format!("{what}: handle {h:#x} has no block")))?;
        Ok((h, p))
    }

    /// The block a handle names, or the refusal that says it is not one.
    pub(crate) fn deref_handle(&self, h: u32, what: &str) -> Result<u32, Refusal> {
        self.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(alloc::format!("{what}: {h:#x} is not a handle")))
    }

    /// The seed `GetCTSeed` returns, then the next one.
    pub(crate) fn unique_seed(&mut self) -> u32 {
        if self.cqd.next_seed < FIRST_UNIQUE_SEED { self.cqd.next_seed = FIRST_UNIQUE_SEED; }
        let s = self.cqd.next_seed;
        self.cqd.next_seed += 1;
        s
    }
}
