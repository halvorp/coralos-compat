//! What a drawing call MEANS, for the plane route (ruling 17, piece 5): the graphics layer keeps the guest bitmap
//! authoritative and, next to it, records a display list per window in terms the host can replay with its own typography
//! and theme. A pen is a theme ROLE, found through the DrawInfo pen table the guest itself reads (`dri_Pens`: which pen
//! number is TEXTPEN, SHINEPEN, ...); a pen number outside the table has no role: fills become [`DrawOp::Swatch`] (exact guest colour),
//! text is quantised to the two kit inks by Rec.709 luminance ([`PenRoles::ink`]), and lines and roleless text backgrounds remain holes (pixels).
//! Nothing is inferred from how the picture looks. A call whose effect is not exactly a named op (a write mask, COMPLEMENT /
//! INVERSVID, an AreaPtrn, soft styles, an installed clip region) is a hole too. Off-screen BitMaps keep the same list for a later plain-copy splice.
use super::layout::*;
use super::rastport::Rp;
use crate::{machine::Amiga, Refusal};
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_present::plane::Rec;
use heirloom_ui_abi::{DrawOp, FontRole, GuestPoint, GuestRect, GuestUi, IconRole, PenRole};
use heirloom_raster::{Rect, Region};

/// Pen number -> theme role, from `dri_Pens` (graphics/layout.rs `SCREEN_DRAWINFO`). One pen number can serve several DrawInfo
/// slots; the guest's own table then says those roles are ONE colour, so the order below only decides which of the equal-coloured
/// roles the host paints it with: TEXT first, then SHADOW before SHINE (a shared hairline IS the shaded-edge colour, the border;
/// the light edge is the one a theme gives its own pen), then the rest. Because the order is documented and the pens equal, nothing
/// is guessed about the picture; and a Shine and a Shadow that share a pen are a FLAT edge: they never coalesce into a bevel.
pub struct PenRoles { by_pen: [Option<PenRole>; 256], rgb: [u32; 256] }

const BY_SLOT: [(usize, PenRole); 7] = [
    (2, PenRole::Text), (4, PenRole::Shadow), (3, PenRole::Shine), (5, PenRole::Fill), (6, PenRole::FillText), (7, PenRole::Background), (8, PenRole::Highlight),
];

/// The INK roles: what a text run may be drawn in, with the theme colour each one is (the kit's Gute Form tokens). Text on a surface is
/// TEXT (dark ink) or FILLTEXT (light ink, for text on a fill); `Shine` (a DrawInfo role, the emboss light) and `Highlight`
/// (HIGHLIGHTTEXTPEN) are ink only when the guest's own DrawInfo says so, they are never chosen by colour.
const INK: [(PenRole, heirloom_theme::Rgb); 2] = [(PenRole::Text, heirloom_theme::GUTE_FORM.text), (PenRole::FillText, heirloom_theme::GUTE_FORM.text_on_accent)];
/// A pen the guest's DrawInfo does not list is classified by colour only when it is this close to an ink colour (squared RGB distance,
/// 3 * 90^2): a colour further than that from every ink stays a hole (pixels), because naming it would be a guess.
const INK_MAX_DIST: u32 = 3 * 90 * 90;

fn dist(a: u32, b: heirloom_theme::Rgb) -> u32 {
    let d = |x: u32, y: u8| (x as i32 - y as i32).unsigned_abs();
    let (r, g, bl) = (d(a >> 16 & 0xFF, b.0), d(a >> 8 & 0xFF, b.1), d(a & 0xFF, b.2));
    r * r + g * g + bl * bl
}

/// Rec.709 luma of a `0x00RRGGBB` colour, 0..=255: Y = 0.2126 R + 0.7152 G + 0.0722 B.
fn luma709(c: u32) -> u32 {
    (2126 * ((c >> 16) & 0xff) + 7152 * ((c >> 8) & 0xff) + 722 * (c & 0xff)) / 10000
}

impl PenRoles {
    pub fn read<U: GuestUi>(a: &mut Amiga<U>) -> Result<PenRoles, Refusal> {
        let m = Refusal::mem("DrawInfo.dri_Pens");
        let table = a.mem.read_u32(SCREEN_DRAWINFO + DRI_PENS).map_err(&m)?;
        let mut dri = [0u16; 9];
        for (slot, d) in dri.iter_mut().enumerate() { *d = a.mem.read_u16(table + 2 * slot as u32).map_err(&m)?; }
        Ok(Self::from_parts(&dri, &a.gfx.palette().rgb))
    }
    /// The roles of a DrawInfo pen array (`dri_Pens[0..9]`) over a screen palette (`0x00RRGGBB`, 256 entries).
    pub fn from_parts(dri: &[u16; 9], palette: &[u32]) -> PenRoles {
        let mut by_pen = [None; 256];
        for (slot, role) in BY_SLOT {
            let pen = dri[slot] as usize;
            if pen < 256 && by_pen[pen].is_none() { by_pen[pen] = Some(role); }
        }
        let mut rgb = [0u32; 256];
        for (d, s) in rgb.iter_mut().zip(palette) { *d = *s; }
        PenRoles { by_pen, rgb }
    }
    /// The role of a pen the guest's DrawInfo lists, for FILLS, LINES and backgrounds: exact, `None` = no DrawInfo slot.
    pub fn role(&self, pen: u8) -> Option<PenRole> { self.by_pen[pen as usize] }
    /// The guest-visible RGB (`0x00RRGGBB`) of `pen` in the screen palette, whether or not the theme has a role for it.
    pub fn rgb(&self, pen: u8) -> u32 { self.rgb[pen as usize] }
    /// The kit ink for `pen`: a DrawInfo ink slot, else the nearer kit ink by colour, else (a far colour with no slot) the ink
    /// picked by Rec.709 luminance of the pen's palette colour. Every pen has an ink: the guest's text is kept as text in the
    /// kit's typography; only its colour is quantised to the two kit inks.
    pub fn ink(&self, pen: u8) -> PenRole {
        let rgb = self.rgb[pen as usize];
        let (near, d) = INK.iter().map(|(r, c)| (*r, dist(rgb, *c))).min_by_key(|(_, d)| *d).unwrap_or((PenRole::Text, u32::MAX));
        match self.by_pen[pen as usize] {
            Some(r @ (PenRole::Text | PenRole::FillText | PenRole::Shine | PenRole::Highlight)) => r,
            Some(_) => near,
            None if d <= INK_MAX_DIST => near,
            None => if luma709(rgb) < 128 { PenRole::Text } else { PenRole::FillText },
        }
    }
}

/// A drawing call as the graphics layer describes it, in RASTPORT coordinates and guest pens.
pub enum Prim {
    /// A solid fill of exactly what the call painted (its damage rectangle, after clipping).
    Fill { pen: u8 },
    /// Segments joined end to end, one pen.
    Lines { pts: Vec<(i32, i32)>, pen: u8 },
    /// A text run; `top` is the top of the cell, `w` the advance. `bg` Some = the run paints its cell first (JAM2).
    Text { x: i32, top: i32, w: i32, h: i32, text: String, fg: u8, bg: Option<u8>, font: FontRole },
    Icon { rect: Rect, role: IconRole },
    /// A bevel box the kit draws (a native system-UI op: the call puts no pixels in the canvas), in RASTPORT coordinates.
    Bevel { rect: Rect, raised: bool },
    /// Solid rectangles of one pen (the runs of a mask), in RASTPORT coordinates: each is a fill, clipped to what the call touched.
    Fills { rects: Vec<Rect>, pen: u8 },
}

impl Prim {
    /// The recorded ops in CANVAS coordinates (`origin` is the layer's canvas origin) and the canvas box each touches;
    /// `None` = some pen has no role, so the call is a hole.
    pub fn ops(&self, roles: &PenRoles, (ox, oy): (i32, i32), hit: Rect) -> Option<Vec<(DrawOp, Rect)>> {
        let g = |r: &Rect| GuestRect { left: r.x0 - ox, top: r.y0 - oy, width: r.width(), height: r.height() };
        Some(match self {
            Prim::Fill { pen } => {
                let rect = GuestRect { left: hit.x0, top: hit.y0, width: hit.width(), height: hit.height() };
                let op = match roles.role(*pen) {
                    Some(pen_role) => DrawOp::Fill { rect, pen_role },
                    None => DrawOp::Swatch { rect, rgb: roles.rgb(*pen) },
                };
                alloc::vec![(op, hit)]
            }
            Prim::Lines { pts, pen } => {
                let role = roles.role(*pen)?;
                if pts.len() == 1 { return Some(alloc::vec![(DrawOp::Fill { rect: GuestRect { left: pts[0].0 - ox, top: pts[0].1 - oy, width: 1, height: 1 }, pen_role: role }, Rect::from_size(pts[0].0 - ox, pts[0].1 - oy, 1, 1))]); }
                pts.windows(2).map(|w| {
                    let (a, b) = ((w[0].0 - ox, w[0].1 - oy), (w[1].0 - ox, w[1].1 - oy));
                    (DrawOp::Line { a: GuestPoint { x: a.0, y: a.1 }, b: GuestPoint { x: b.0, y: b.1 }, pen_role: role }, Rect::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0) + 1, a.1.max(b.1) + 1))
                }).collect()
            }
            Prim::Text { x, top, w, h, text, fg, bg, font } => {
                let bg_role = match bg { Some(p) => Some(roles.role(*p)?), None => None };
                let r = Rect::from_size(x - ox, top - oy, *w, *h);
                alloc::vec![(DrawOp::Text { at: GuestPoint { x: r.x0, y: r.y0 }, string: text.clone(), font_role: *font, px: (*h).max(1) as u16, width: *w, pen_role: roles.ink(*fg), bg_role }, r)]
            }
            Prim::Fills { rects, pen } => {
                let (role, rgb) = (roles.role(*pen), roles.rgb(*pen));
                rects.iter().map(|r| r.translate(-ox, -oy).intersect(&hit)).filter(|r| !r.is_empty())
                    .map(|r| {
                        let rect = GuestRect { left: r.x0, top: r.y0, width: r.width(), height: r.height() };
                        (match role { Some(pen_role) => DrawOp::Fill { rect, pen_role }, None => DrawOp::Swatch { rect, rgb } }, r)
                    }).collect()
            }
            Prim::Icon { rect, role } => alloc::vec![(DrawOp::Icon { rect: g(rect), role: *role }, rect.translate(-ox, -oy))],
            Prim::Bevel { rect, raised } => alloc::vec![(DrawOp::Bevel { rect: g(rect), raised: *raised }, rect.translate(-ox, -oy))],
        })
    }
}

/// Is a solid, full-mask, JAM1 / JAM2 drawing state: the only one whose result is exactly "the foreground pen".
pub fn plain(st: &Rp) -> bool { st.mask == 0xFF && matches!(st.mode, JAM1 | JAM2) }

/// RectFill's call as a prim: only without an AreaPtrn.
pub fn fill(st: &Rp) -> Option<Prim> { (plain(st) && (st.area_ptrn == 0 || st.area_pt_sz == 0)).then_some(Prim::Fill { pen: st.fg }) }

/// A call that paints `pen` solidly whatever the mode (EraseRect, SetRast, ClearEOL): any full-mask state.
pub fn solid(st: &Rp, pen: u8) -> Option<Prim> { (st.mask == 0xFF).then_some(Prim::Fill { pen }) }

/// Draw's call: a solid line pattern only.
pub fn line(st: &Rp, from: (i32, i32), to: (i32, i32)) -> Option<Prim> {
    (plain(st) && st.line_ptrn == 0xFFFF).then(|| Prim::Lines { pts: alloc::vec![from, to], pen: st.fg })
}

/// Text's call: no soft style; JAM2 paints the cell in the BgPen.
pub fn text(st: &Rp, font: &heirloom_raster::font::Font, bytes: &[u8], (x, y): (i32, i32)) -> Option<Prim> {
    if !(st.mask == 0xFF && matches!(st.mode, JAM1 | JAM2)) || st.algo != 0 { return None; }
    let role = if font.flags & heirloom_raster::font::FPF_PROPORTIONAL != 0 { FontRole::Ui } else { FontRole::Mono }; // FPF_PROPORTIONAL
    Some(Prim::Text { x, top: y - font.baseline as i32, w: font.text_length(bytes), h: font.y_size as i32, text: bytes.iter().map(|&b| b as char).collect(), fg: st.fg, bg: (st.mode == JAM2).then_some(st.bg), font: role })
}

/// Where a recorded call goes: a window's display list, or an off-screen bitmap's (surface.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink { Window(u32), Surface(u32) }

pub(crate) fn emit<U: GuestUi>(a: &mut Amiga<U>, sink: Sink, rec: Rec) {
    match sink {
        Sink::Window(win) => match rec {
            Rec::Op { op, bbox } => a.gfx.present.record(win, op, bbox),
            Rec::Hole(hit) => a.gfx.present.record_hole(win, hit),
        },
        Sink::Surface(bm) => if a.gfx.present.plane_route() { a.gfx.surfaces.entry(bm).add_rect(rec); },
    }
}

/// Record one drawing call (`what`, its name) whose effect (bitmap/canvas coordinates) is `hit`: the named ops of `prim`, or a hole
/// when there is no prim (`Err` says why), a pen has no role, or the DrawInfo cannot be read. A hole is counted under its reason
/// (`heirloom_present::Presenter::why_hole`).
pub fn note<U: GuestUi>(a: &mut Amiga<U>, sink: Sink, what: &str, prim: Result<Prim, &'static str>, region: Option<&Region>, hit: Rect, origin: (i32, i32)) {
    if hit.is_empty() || !a.gfx.present.plane_route() { return; }
    let ops = match prim {
        Err(why) => Err(why),
        Ok(p) => match PenRoles::read(a) {
            Err(_) => Err("the DrawInfo pens cannot be read"),
            Ok(roles) => clipped(p, region, origin).ops(&roles, origin, hit).ok_or("a pen with no theme role")
                .and_then(|v| if region.is_none_or(|g| v.iter().all(|(_, b)| inside(g, b))) { Ok(v) } else { Err("a clip region cuts the op") }),
        },
    };
    match ops {
        Ok(v) => for (op, bbox) in v { emit(a, sink, Rec::Op { op, bbox }); },
        Err(why) => {
            let place = match sink { Sink::Window(_) => "", Sink::Surface(_) => "off-screen " };
            a.gfx.present.why_hole(&alloc::format!("{place}{what}: {why}"), hit);
            emit(a, sink, Rec::Hole(hit));
        }
    }
}

/// A fill under an installed clip region (`region`: bitmap coordinates, as the ops) paints exactly its pen over each of the region's
/// rectangles: it becomes [`Prim::Fills`] over them (in RastPort coordinates, so `origin` is added back). Any other prim is unchanged,
/// and [`note`] keeps it only when the region does not cut it.
fn clipped(p: Prim, region: Option<&Region>, (ox, oy): (i32, i32)) -> Prim {
    match (p, region) {
        (Prim::Fill { pen }, Some(g)) => Prim::Fills { rects: g.rects().iter().map(|r| r.translate(ox, oy)).collect(), pen },
        (p, _) => p,
    }
}

/// Does `b` lie wholly inside one rectangle of `g` (so the region leaves the op as drawn)?
fn inside(g: &Region, b: &Rect) -> bool { b.is_empty() || g.rects().iter().any(|r| r.intersect(b) == *b) }

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_theme::amiga::{system_palette, DRI_PENS};

    fn roles(palette: &[u32]) -> PenRoles { PenRoles::from_parts(&DRI_PENS[..9].try_into().unwrap(), palette) }
    fn palette_with(pen: usize, rgb: u32) -> [u32; 256] { let mut p = [0u32; 256]; p[..8].copy_from_slice(&system_palette()); p[pen] = rgb; p }

    #[test]
    fn text_in_the_system_pens_is_text_ink_whatever_drawinfo_calls_the_pen() {
        let r = roles(&palette_with(9, 0));
        assert_eq!(r.ink(1), PenRole::Text, "TEXTPEN");
        assert_eq!(r.ink(2), PenRole::FillText, "FILLTEXTPEN: white on the accent");
        // pen 3 is FILLPEN (accent) in DrawInfo, yet a program that prints in it prints on its surface: dark ink, Text, not the accent
        assert_eq!(r.role(3), Some(PenRole::Fill), "as a FILL the pen stays exact");
        assert_eq!(r.ink(3), PenRole::Text, "as INK the area pen 3 is the nearest ink colour");
        assert_eq!(r.ink(4), PenRole::FillText, "SHADOWPEN is the light border colour: light ink stays light (a guest that prints in it meant it faint)");
        assert_eq!(r.ink(0), PenRole::FillText, "pen 0 (the light surface) as ink is light ink");
    }

    #[test]
    fn a_pen_the_program_obtained_itself_is_named_by_colour_and_a_far_colour_is_quantised_by_luminance() {
        // ObtainPen(RGB) pens have no DrawInfo slot: black-ish -> Text, white -> FillText.
        assert_eq!(roles(&palette_with(9, 0x101010)).ink(9), PenRole::Text);
        assert_eq!(roles(&palette_with(9, 0xFAFAFA)).ink(9), PenRole::FillText);
        // Pure red is nowhere near an ink colour: quantised by Rec.709 luminance (luma ~54 < 128 -> Text), while fill role remains None.
        let r = roles(&palette_with(9, 0xFF0000));
        assert_eq!(r.ink(9), PenRole::Text);
        assert_eq!(r.role(9), None, "and the exact fill role of an unlisted pen is none either");
    }

    #[test]
    fn a_rect_fill_in_an_unlisted_pen_reaches_the_host_as_a_named_swatch() {
        const P_FILLED: u8 = 3;
        const P_ROLELESS: u8 = 9;
        let mut dri = [0u16; 9];
        dri[5] = P_FILLED as u16;
        let palette = palette_with(P_ROLELESS as usize, 0xC03020);
        let roles = PenRoles::from_parts(&dri, &palette);
        let hit = Rect::from_size(10, 20, 30, 40);

        let v = Prim::Fill { pen: P_ROLELESS }.ops(&roles, (0, 0), hit).unwrap();
        assert_eq!(v.len(), 1);
        assert!(matches!(v[0].0, DrawOp::Swatch { rgb: 0xC03020, .. }));

        let v_ctrl = Prim::Fill { pen: P_FILLED }.ops(&roles, (0, 0), hit).unwrap();
        assert_eq!(v_ctrl.len(), 1);
        assert!(matches!(v_ctrl[0].0, DrawOp::Fill { pen_role: PenRole::Fill, .. }));
    }

    #[test]
    fn ink_of_a_far_colour_is_quantised_by_luminance() {
        const P_TEXT: u8 = 1;
        const P_RED: u8 = 9;
        const P_YELLOW: u8 = 10;
        let mut dri = [0u16; 9];
        dri[2] = P_TEXT as u16;
        let mut palette = [0u32; 256];
        palette[P_TEXT as usize] = 0xFFFF00;
        palette[P_RED as usize] = 0xFF0000;
        palette[P_YELLOW as usize] = 0xFFFF00;
        let roles = PenRoles::from_parts(&dri, &palette);

        assert_eq!(roles.ink(P_RED), PenRole::Text);
        assert_eq!(roles.ink(P_YELLOW), PenRole::FillText);
        assert_eq!(roles.ink(P_TEXT), PenRole::Text);
    }
}
