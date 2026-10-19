//! Classic QuickDraw drawing, on `retro-raster` (the same raster crate the Amiga engine's graphics.library uses) and
//! published through `retro-present` (the same damage/`present_content` flush).
//!
//! The GrafPort, the BitMap, the Region and the Pattern are REAL STRUCTS IN GUEST MEMORY at their Inside Macintosh
//! offsets ([`port`]); the pixels live host-side:
//!
//! * a window's content is a host canvas (a chunky [`retro_raster::Bitmap`]). A classic window's is 1 bit deep, pen 0 = white,
//!   pen 1 = black, the palette `[white, black]`; a colour window's (`NewCWindow`) is 8 bits deep and its pen numbers are the
//!   entries of its PixMap's colour table ([`crate::cqd`]). The window's `portBits` (or `portPixMap`) still names the screen (a
//!   program that reads it is not lied to), but drawing goes to the canvas, in local coordinates (`pixel = local - portRect.topleft`).
//! * any other port (an `OpenPort`ed offscreen one with `SetPortBits`, an `OpenCPort`ed one over a PixMap) draws into a REAL
//!   packed BitMap / PixMap in guest memory (1, 2, 4 or 8 bits): it is unpacked to a chunky bitmap, drawn into, and packed back
//!   ([`target`]).
//!
//! Substitutions, named once here: text is drawn in `retro-raster`'s topaz-8 for EVERY font id and size (Chicago and
//! Geneva are not bundled), see [`text`]; each window has its own canvas, so `visRgn` is the window's portRect and
//! never carves out overlapping windows; direct (16/32-bit) pixel maps and colour patterns are refused by name.
pub mod colorpen;
pub mod copybits;
pub mod draw;
pub mod fontmetrics;
pub mod inset;
pub mod origin;
pub mod picture;
pub mod lifecycle;
pub mod plane;
pub mod port;
pub mod regions;
pub mod shapes;
pub mod stretch;
pub mod target;
pub mod text;
pub mod update;

use crate::machine::Mac;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
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
    /// The Font Manager's global width table (a guest handle, 0 until `FontMetrics` first builds it; [`fontmetrics`]).
    pub width_table: u32,
    /// Ports between `OpenRgn` and `CloseRgn`: the region collected so far (local coordinates).
    pub rgn_open: BTreeMap<u32, Region>,
    /// Per window: the update region (local coordinates).
    pub update: BTreeMap<u32, Region>,
    /// Per window between BeginUpdate and EndUpdate: the visRgn to restore.
    pub saved_vis: BTreeMap<u32, Region>,
    /// Per window: the retained display list the host replays (plane.rs).
    pub plane: plane::Planes,
    /// Per port: the picture being recorded (picture.rs).
    pub pictures: BTreeMap<u32, picture::OpenPic>,
    /// Per colour window: the host colours of its colour table as last presented (a change repaints the window).
    pub palettes: BTreeMap<u32, alloc::vec::Vec<u32>>,
}

impl Default for Qd {
    fn default() -> Self {
        Qd { canvases: BTreeMap::new(), present: Presenter::default(), font: Font::topaz8(), width_table: 0, rgn_open: BTreeMap::new(), update: BTreeMap::new(), saved_vis: BTreeMap::new(), plane: plane::Planes::default(), pictures: BTreeMap::new(), palettes: BTreeMap::new() }
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
    origin::register(m)?;
    picture::register(m)?;
    colorpen::register(m)?;
    text::register(m)?;
    update::register(m)?;
    lifecycle::register(m)
}

/// Publish every window's accumulated drawing: one `present_plane` per changed window (the display list), or, for a host
/// without plane replay, one `present_content` per damaged window. Called by `Mac::step`.
pub fn flush<U: GuestUi>(m: &mut Mac<U>) {
    refresh_palettes(m);
    plane::flush(m);
    let mut presenter = core::mem::take(&mut m.qd.present);
    let (windows, canvases, palettes) = (&m.windows, &m.qd.canvases, &m.qd.palettes);
    presenter.flush(&mut m.ui, "[quickdraw]", &mut m.log, &mut m.refused, |win| {
        Some(Source { native: windows.get(&win)?.native?, canvas: canvases.get(&win)?, palette: palettes.get(&win).map_or(&PALETTE[..], Vec::as_slice) })
    });
    m.qd.present = presenter;
}

/// A colour window shows its canvas through its pixel map's colour table; when the table changes, the whole window is shown again.
fn refresh_palettes<U: GuestUi>(m: &mut Mac<U>) {
    let wins: Vec<u32> = m.qd.canvases.iter().filter(|(_, c)| c.depth() > 1).map(|(&w, _)| w).collect();
    for win in wins {
        let depth = m.qd.canvases[&win].depth();
        let table = port::bits_addr(&m.mem, win).and_then(|pm| m.pixmap_clut(pm, depth));
        let host: Vec<u32> = match table { Ok(t) => t.iter().map(|c| c.to_host()).collect(), Err(e) => { m.refused.push(alloc::format!("[quickdraw] window {win:#x}: colour table: {}", e.0)); continue } };
        if m.qd.palettes.get(&win) == Some(&host) { continue; }
        if m.qd.palettes.insert(win, host.clone()).is_some() {
            plane::palette_changed(&mut m.qd.plane, win, &m.qd.canvases[&win], &host);
            let r = m.qd.canvases[&win].rect();
            m.qd.present.add_damage(win, r);
        }
    }
}

/// A one-line description for logs.
pub(crate) fn rect_str(r: &retro_raster::Rect) -> String { alloc::format!("({},{},{},{})", r.y0, r.x0, r.y1, r.x1) }
