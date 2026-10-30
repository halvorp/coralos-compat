//! Content planes as display lists (ruling 17, piece 5): guest drawing the engine could not map to a control, kept as DATA the
//! host replays with the kit's own typography and theme. A pen is a theme ROLE, never a guest pen number: the engine resolves
//! guest pens to roles (or `Pixels` for what it cannot name), so the host needs no palette semantics.
use alloc::string::String;
use alloc::vec::Vec;
use crate::model::{GuestRect, UiRefusal};
use crate::IconRole;

/// The closed set of theme colour roles a plane draws with (the kit maps each to its Gute Form token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PenRole { Surface, Text, TextMuted, Shine, Shadow, Fill, FillText, Accent, Highlight, Background, Selection }

/// The closed set of font roles; the host picks the face, the plane gives the pixel size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontRole { Ui, Mono, Title }

/// A point in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestPoint { pub x: i32, pub y: i32 }

/// One recorded drawing operation. All coordinates are window coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawOp {
    Fill { rect: GuestRect, pen_role: PenRole },
    /// A text run with its top-left at `at`; `bg_role` `Some` paints the run's box first (an opaque text mode). `px` is the guest
    /// font's HEIGHT (the cell), `width` the run's advance in the GUEST's font (`TextLength`): the box `width x px` is where the guest
    /// drew it. The host replays with the kit's typography at a kit size (>= `px`, TEXT-FITS-BY-WIDTH) and FITS the run into `width`,
    /// so columns stay aligned and nothing overlaps; `width == 0` = the engine has no guest advance (the host then does not fit).
    Text { at: GuestPoint, string: String, font_role: FontRole, px: u16, width: i32, pen_role: PenRole, bg_role: Option<PenRole> },
    Line { a: GuestPoint, b: GuestPoint, pen_role: PenRole },
    /// A bevel pair (Shine/Shadow edges) the kit draws as its own bevel.
    Bevel { rect: GuestRect, raised: bool },
    /// A recognised icon resource, drawn from the host's artwork for `role`.
    Icon { rect: GuestRect, role: IconRole },
    /// Guest pixels the engine could not name (`0x00RRGGBB`, exactly `rect.width * rect.height`, row-major): the raw route.
    Pixels { rect: GuestRect, rgb: Vec<u32> },
    /// A solid fill in an EXACT guest colour (`0x00RRGGBB`) the theme has no role for: the user's own configuration (Directory
    /// Opus's coloured button bank). The kit paints it as a plain surface; it is named, not raw, so text and edges on it replay.
    Swatch { rect: GuestRect, rgb: u32 },
}

impl DrawOp {
    /// The refusal for an op that cannot be replayed (negative size, a pixel count that is not the area), else `Ok`.
    pub fn validate(&self) -> Result<(), UiRefusal> {
        let bad = |r: &GuestRect| r.width < 0 || r.height < 0;
        match self {
            DrawOp::Fill { rect, .. } | DrawOp::Bevel { rect, .. } | DrawOp::Icon { rect, .. } | DrawOp::Swatch { rect, .. } if bad(rect) =>
                Err(UiRefusal::InvalidGeometry("draw op with a negative size")),
            DrawOp::Pixels { rect, rgb } if bad(rect) || rgb.len() != (rect.width as usize) * (rect.height as usize) =>
                Err(UiRefusal::InvalidGeometry("pixel op: count is not the rect's area")),
            DrawOp::Text { px: 0, .. } => Err(UiRefusal::InvalidGeometry("text with a zero pixel size")),
            DrawOp::Text { width, .. } if *width < 0 => Err(UiRefusal::InvalidGeometry("text with a negative run width")),
            _ => Ok(()),
        }
    }
    /// Does this op carry guest pixels (the raw route)?
    pub fn is_raw(&self) -> bool { matches!(self, DrawOp::Pixels { .. }) }
}

/// A content plane frame: `ops` is the whole display list for `rect` (window coordinates, the region to repaint); the host
/// clips to `rect`, and a later frame for the same `id` replaces the earlier one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane { pub id: u32, pub rect: GuestRect, pub ops: Vec<DrawOp> }

impl Plane {
    /// The refusal for a plane that cannot be replayed (empty rect, or any bad op), else `Ok`.
    pub fn validate(&self) -> Result<(), UiRefusal> {
        if self.rect.width <= 0 || self.rect.height <= 0 { return Err(UiRefusal::InvalidGeometry("plane rect must be positive")); }
        self.ops.iter().try_for_each(DrawOp::validate)
    }
    /// Does any op carry guest pixels (the plane then counts on the raw route)?
    pub fn is_raw(&self) -> bool { self.ops.iter().any(DrawOp::is_raw) }
}

/// How a window's visible content reached the host (ruling 17, piece 9: native is a MEASURED claim). Counts of units:
/// `native` = live controls drawn by kit parts, `replayed` = planes shown purely from display lists, `raw` = guest-pixel
/// canvases (`present_content`) plus planes holding any `Pixels` op. A host computes the same from what it instantiated:
/// kit parts for controls, planes replayed through the kit's typography, and pixel blits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteCensus { pub native: u32, pub replayed: u32, pub raw: u32 }
