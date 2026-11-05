//! Color QuickDraw on the Macintosh II profile (Inside Macintosh: Imaging With QuickDraw, "Color QuickDraw"; Inside
//! Macintosh V, "Color QuickDraw"): the structures a colour program builds and reads, in REAL GUEST MEMORY at their documented
//! offsets, and the traps over them.
//!
//! * [`color`]: `RGBColor`, the standard colour tables, nearest-colour search.
//! * [`ctab`]: `ColorTable` / `CTabHandle` (`GetCTable`, `DisposeCTable`, `GetCTSeed`) and the Color Manager's
//!   `Index2Color` / `Color2Index` / `InvertColor` / `RealColor`.
//! * [`pixmap`]: `PixMap` / `PixMapHandle` records and reading/writing pixels of a packed guest pixel map (1, 2, 4, 8, 16, 32 bits).
//! * [`gdevice`]: `GDevice` / `GDHandle`: the main screen device (8 bits deep, the standard 8-bit table),
//!   `GetMainDevice`/`GetGDevice`/`SetGDevice`/`GetDeviceList`/`GetNextDevice`/`TestDeviceAttribute`/`GetMaxDevice`.
//! * [`depth`]: `HasDepth` / `SetDepth` of the Palette Manager's dispatch trap.
//! * [`cport`]: `CGrafPort` records: `OpenCPort`, `InitCPort`, `CloseCPort`, and the colour window's port (`NewCWindow`).
//! * [`pens`]: the pen colours, `RGBForeColor`/`RGBBackColor`/`GetForeColor`/`GetBackColor` and the classic `ForeColor`.
//!
//! **One implementation of the drawing ops.** There is no colour code path beside the 1-bit one: the shared QuickDraw ops
//! ([`crate::qd`]) draw into a chunky `heirloom_raster::Bitmap` whose depth is the port's (1 classic; 2-8 indexed; 16/32 direct),
//! the pen is a pair of pixel VALUES (index 1/0 in a 1-bit port; `Color2Index` of `rgbFgColor`/`rgbBkColor` in an indexed
//! one; the colour itself in a direct one), and the Boolean transfer modes act on pixel values on indexed devices (the standard
//! table puts white at 0 and black at 255 so that or/xor/bic read as black-and-white there) and by IWQ's colour definitions on
//! direct ones ([`direct`], `qd::draw::PenOp`).
//!
//! * [`direct`]: 16/32-bit pixels (x1R5G5B5 / x8R8G8B8), the inverse table, the one depth converter, the arithmetic transfer modes.
//!
//! Not modelled, by name (each one is a refusal when called, never a fake): `gdITable` stays NIL (the engine keeps its own inverse
//! tables, `Mac::inverse_table`), device search/complement procedures, gamma, hilite mode 50, colour patterns other than `patType` 0,
//! colour cursors ('crsr'). Colour icons ('cicn') are [`cicon`].
pub mod cicon;
pub mod cport;
pub mod color;
pub mod ctab;
pub mod depth;
pub mod direct;
pub mod gdevice;
pub mod gworld;
pub mod pens;
pub mod pixmap;

use crate::machine::Mac;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;

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
    /// Recently used inverse tables, most recent first: (the colour table, its inverse table). A cache (`Mac::inverse_table`).
    pub itabs: alloc::vec::Vec<(alloc::vec::Vec<color::Rgb>, direct::ITab)>,
}

/// Seeds below this are the standard tables' (their `'clut'` id); `GetCTSeed` starts above it (Imaging With QuickDraw).
pub const FIRST_UNIQUE_SEED: u32 = 1024;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    ctab::register(m)?;
    gdevice::register(m)?;
    depth::register(m)?;
    pixmap::register(m)?;
    cport::register(m)?;
    cicon::register(m)?;
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
