//! Host-side bookkeeping for graphics.library: what the guest must not be able to corrupt. The guest's RastPorts, fonts
//! and BitMap STRUCTS live in guest memory and are the authority for their fields; the pixels, regions and colours
//! live here.
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
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
    /// The reserved system pens (the classic Workbench 2.x 4-colour look, then four more) and everything else black.
    pub fn new() -> Palette {
        let mut p = Palette { rgb: [0; 256], refs: [0; 256], exclusive: [false; 256] };
        let sys = [0xAAAAAA, 0x000000, 0xFFFFFF, 0x6688BB, 0x999999, 0xBBBBBB, 0xEEBB99, 0x4488DD];
        for (i, c) in sys.into_iter().enumerate() { p.rgb[i] = c; p.refs[i] = 1; }
        p
    }
}

#[derive(Default)]
pub struct State {
    pub bitmaps: BTreeMap<u32, GBitmap>,
    pub layers: BTreeMap<u32, GLayer>,
    pub regions: BTreeMap<u32, GRegion>,
    /// RastPorts whose drawing is DROPPED by design, with the reason (a window's BorderRPort: the frame is native).
    pub dropped_rps: BTreeMap<u32, &'static str>,
    /// Fonts the guest has AddFont()ed or that the engine registered (guest TextFont addresses), in registration order.
    pub fonts: Vec<u32>,
    pub rom_font: Option<Rc<Font>>,
    pub palette: Option<Palette>,
    /// Per window: the canvas rectangle changed since the last `present_content`.
    pub damage: BTreeMap<u32, Rect>,
    /// Windows that have had their first (full-canvas) frame.
    pub presented: BTreeSet<u32>,
    /// A host without a canvas part refused a frame; logged once.
    pub content_refused: bool,
    /// Pending AreaMove/AreaDraw polygons by RastPort: (points, closed-at).
    pub areas: BTreeMap<u32, Vec<(i32, i32)>>,
}

impl State {
    pub fn palette(&mut self) -> &mut Palette { self.palette.get_or_insert_with(Palette::new) }
    pub fn add_damage(&mut self, win: u32, r: Rect) {
        if r.is_empty() { return; }
        let e = self.damage.entry(win).or_default();
        *e = e.union(&r);
    }
}
