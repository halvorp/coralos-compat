//! Classic QuickDraw drawing, on `retro-raster` (the same raster crate the Amiga engine's graphics.library uses) and
//! published through `retro-present` (the same damage/`present_content` flush).
//!
//! The GrafPort, the BitMap, the Region and the Pattern are REAL STRUCTS IN GUEST MEMORY at their Inside Macintosh
//! offsets ([`port`]); the pixels live host-side:
//!
//! * a window's content is a host canvas (1-bit chunky [`retro_raster::Bitmap`], pen 0 = white, pen 1 = black, the
//!   palette `[white, black]`). The window's `portBits` still names the screen (a program that reads it is not lied to),
//!   but drawing goes to the canvas, in local coordinates (`pixel = local - portRect.topleft`).
//! * any other port (an `OpenPort`ed offscreen one with `SetPortBits`) draws into a REAL 1-bit packed BitMap in guest
//!   memory: it is unpacked to a chunky bitmap, drawn into, and packed back ([`target`]).
//!
//! Substitutions, named once here: text is drawn in `retro-raster`'s topaz-8 for EVERY font id and size (Chicago and
//! Geneva are not bundled), see [`text`]; each window has its own canvas, so `visRgn` is the window's portRect and
//! never carves out overlapping windows; color (CGrafPort/PixMap) is refused by name; `SetOrigin` is not implemented.
pub mod copybits;
pub mod draw;
pub mod lifecycle;
pub mod port;
pub mod regions;
pub mod shapes;
pub mod target;
pub mod text;
pub mod update;

use crate::machine::Mac;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_present::{Presenter, Source};
use retro_raster::{font::Font, Bitmap, Region};

/// pen 0 = white, pen 1 = black.
pub const PALETTE: [u32; 2] = [0x00FF_FFFF, 0x0000_0000];

/// Host-side QuickDraw state.
pub struct Qd {
    /// WindowPtr (the window's embedded GrafPort) -> its content canvas.
    pub canvases: BTreeMap<u32, Bitmap>,
    pub present: Presenter,
    pub font: Font,
    /// Ports between `OpenRgn` and `CloseRgn`: the region collected so far (local coordinates).
    pub rgn_open: BTreeMap<u32, Region>,
    /// Per window: the update region (local coordinates).
    pub update: BTreeMap<u32, Region>,
    /// Per window between BeginUpdate and EndUpdate: the visRgn to restore.
    pub saved_vis: BTreeMap<u32, Region>,
}

impl Default for Qd {
    fn default() -> Self {
        Qd { canvases: BTreeMap::new(), present: Presenter::default(), font: Font::topaz8(), rgn_open: BTreeMap::new(), update: BTreeMap::new(), saved_vis: BTreeMap::new() }
    }
}

/// Register a Pascal-convention trap after checking that `name` really is what the generated table calls `opcode`:
/// a mistyped trap word fails `Mac::new`, it does not silently bridge another routine.
pub(crate) fn reg<U: GuestUi>(m: &mut Mac<U>, opcode: u16, name: &str, args: &'static [crate::pascal::Arg], result: Option<crate::pascal::Arg>, h: crate::machine::Handler<U>) -> Result<(), Refusal> {
    match crate::traps::trap_name(opcode) {
        Some(n) if n == name => m.register(opcode, args, result, h),
        other => Err(Refusal(alloc::format!("qd register: ${opcode:04X} is {other:?}, expected {name}"))),
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    draw::register(m)?;
    regions::register(m)?;
    copybits::register(m)?;
    text::register(m)?;
    update::register(m)?;
    lifecycle::register(m)
}

/// Publish every window's accumulated drawing: one `present_content` per damaged window. Called by `Mac::step`.
pub fn flush<U: GuestUi>(m: &mut Mac<U>) {
    let mut presenter = core::mem::take(&mut m.qd.present);
    let (windows, canvases) = (&m.windows, &m.qd.canvases);
    presenter.flush(&mut m.ui, "[quickdraw]", &mut m.log, &mut m.refused, |win| {
        Some(Source { native: windows.get(&win)?.native?, canvas: canvases.get(&win)?, palette: &PALETTE })
    });
    m.qd.present = presenter;
}


/// A one-line description for logs.
pub(crate) fn rect_str(r: &retro_raster::Rect) -> String { alloc::format!("({},{},{},{})", r.y0, r.x0, r.y1, r.x1) }
