//! Host-side bookkeeping for graphics.library: what the guest must not be able to corrupt. The guest's RastPorts, fonts
//! and BitMap STRUCTS live in guest memory and are the authority for their fields; the pixels, regions and colours
//! live here.
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use retro_raster::{font::Font, Bitmap, Rect, Region};

/// A host-backed chunky bitmap behind a guest `struct BitMap` (keyed by the struct's address).
pub struct GBitmap {
    pub bm: Bitmap,
    /// The window whose content canvas this is: drawing into it becomes damage for that window.
    pub window: Option<u32>,
}

/// A window's layer: drawing through its RastPort lands in `canvas` at (x - origin.0, y - origin.1).
pub struct GLayer { pub window: u32, pub canvas: u32, pub origin: (i32, i32) }

/// A guest `struct Region`: the host-side pixel set, and the guest block holding its RegionRectangle chain.
pub struct GRegion { pub region: Region, pub chain: Option<(u32, u32)> }

/// The screen palette: 256 pens of `0x00RRGGBB`, pens 0..8 reserved by the system like Workbench's.
pub struct Palette { pub rgb: [u32; 256], pub refs: [u16; 256], pub exclusive: [bool; 256] }

impl Palette {
    /// The reserved system pens 0..8 (Gute Form: retro_theme::amiga::SYSTEM_PENS) and everything else black.
    pub fn new() -> Palette {
        let mut p = Palette { rgb: [0; 256], refs: [0; 256], exclusive: [false; 256] };
        for (i, c) in retro_theme::amiga::system_palette().into_iter().enumerate() { p.rgb[i] = c; p.refs[i] = 1; }
        for (i, c) in retro_theme::amiga::mirror_palette().into_iter().enumerate() { p.rgb[248 + i] = c; p.refs[248 + i] = 1; }
        p
    }
}

#[derive(Default)]
pub struct State {
    /// The program's own name (the file it was started from, `Amiga::start_hunk_as`): the title of a window the guest opened without one.
    pub program_name: String,
    pub bitmaps: BTreeMap<u32, GBitmap>,
    pub layers: BTreeMap<u32, GLayer>,
    pub regions: BTreeMap<u32, GRegion>,
    /// RastPorts whose drawing is DROPPED by design, with the reason (a window's BorderRPort: the frame is native).
    pub dropped_rps: BTreeMap<u32, &'static str>,
    /// Fonts the guest has AddFont()ed or that the engine registered (guest TextFont addresses), in registration order.
    pub fonts: Vec<u32>,
    /// The parsed renderer fonts of the two resident ROM faces (topaz 8 and the Gute Form face), by guest address.
    pub rom_fonts: BTreeMap<u32, Rc<Font>>,
    pub palette: Option<Palette>,
    /// Per-window damage and first-frame state; flushed by `content::flush` through the shared `retro-present`.
    pub present: retro_present::Presenter,
    /// Images (guest addresses) of default icons that carry the host's artwork for a role: a draw of one is an `Icon` op.
    pub icon_images: BTreeMap<u32, guestui_abi::IconRole>,
    /// Pending AreaMove/AreaDraw polygons by RastPort: (points, closed-at).
    pub areas: BTreeMap<u32, Vec<(i32, i32)>>,
}

impl State {
    /// The plane route is the default (ruling 17 piece 5): window content is published as a display list, with
    /// the canvas route as the host-refused fallback.
    pub fn new() -> State { let mut s = State::default(); s.present.set_plane_route(true); s }
    pub fn palette(&mut self) -> &mut Palette { self.palette.get_or_insert_with(Palette::new) }
    pub fn add_damage(&mut self, win: u32, r: Rect) {
        self.present.add_damage(win, r);
    }
}
