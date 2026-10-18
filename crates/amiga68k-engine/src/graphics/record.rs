//! What a drawing call MEANS, for the plane route (ruling 17, piece 5): the graphics layer keeps the guest bitmap
//! authoritative and, next to it, records a display list per window in terms the host can replay with its own typography
//! and theme. A pen is a theme ROLE, found through the DrawInfo pen table the guest itself reads (`dri_Pens`: which pen
//! number is TEXTPEN, SHINEPEN, ...); a pen number outside the table has no role, so the call is recorded as a HOLE (pixels).
//! Nothing is inferred from how the picture looks. A call whose effect is not exactly a named op (a write mask, COMPLEMENT /
//! INVERSVID, an AreaPtrn, soft styles, an installed clip region) is a hole too.
use super::layout::*;
use super::rastport::Rp;
use crate::{machine::Amiga, Refusal};
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::{DrawOp, FontRole, GuestPoint, GuestRect, GuestUi, IconRole, PenRole};
use retro_raster::Rect;

/// Pen number -> theme role, from `dri_Pens` (graphics/layout.rs `SCREEN_DRAWINFO`). One pen number can serve several DrawInfo
/// slots; the guest's own table then says those roles are ONE colour, so the order below only decides which of the equal-coloured
/// roles the host paints it with: TEXT first, then SHADOW before SHINE (a shared hairline IS the shaded-edge colour, the border;
/// the light edge is the one a theme gives its own pen), then the rest. Because the order is documented and the pens equal, nothing
/// is guessed about the picture; and a Shine and a Shadow that share a pen are a FLAT edge: they never coalesce into a bevel.
pub struct PenRoles([Option<PenRole>; 256]);

const BY_SLOT: [(usize, PenRole); 7] = [
    (2, PenRole::Text), (4, PenRole::Shadow), (3, PenRole::Shine), (5, PenRole::Fill), (6, PenRole::FillText), (7, PenRole::Background), (8, PenRole::Highlight),
];

impl PenRoles {
    pub fn read<U: GuestUi>(a: &Amiga<U>) -> Result<PenRoles, Refusal> {
        let m = Refusal::mem("DrawInfo.dri_Pens");
        let table = a.mem.read_u32(SCREEN_DRAWINFO + DRI_PENS).map_err(&m)?;
        let mut roles = [None; 256];
        for (slot, role) in BY_SLOT {
            let pen = a.mem.read_u16(table + 2 * slot as u32).map_err(&m)? as usize;
            if pen < 256 && roles[pen].is_none() { roles[pen] = Some(role); }
        }
        Ok(PenRoles(roles))
    }
    pub fn role(&self, pen: u8) -> Option<PenRole> { self.0[pen as usize] }
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
}

impl Prim {
    /// The recorded ops in CANVAS coordinates (`origin` is the layer's canvas origin) and the canvas box each touches;
    /// `None` = some pen has no role, so the call is a hole.
    pub fn ops(&self, roles: &PenRoles, (ox, oy): (i32, i32), hit: Rect) -> Option<Vec<(DrawOp, Rect)>> {
        let g = |r: &Rect| GuestRect { left: r.x0 - ox, top: r.y0 - oy, width: r.width(), height: r.height() };
        Some(match self {
            Prim::Fill { pen } => alloc::vec![(DrawOp::Fill { rect: GuestRect { left: hit.x0, top: hit.y0, width: hit.width(), height: hit.height() }, pen_role: roles.role(*pen)? }, hit)],
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
                alloc::vec![(DrawOp::Text { at: GuestPoint { x: r.x0, y: r.y0 }, string: text.clone(), font_role: *font, px: (*h).max(1) as u16, pen_role: roles.role(*fg)?, bg_role }, r)]
            }
            Prim::Icon { rect, role } => alloc::vec![(DrawOp::Icon { rect: g(rect), role: *role }, rect.translate(-ox, -oy))],
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
pub fn text(st: &Rp, font: &retro_raster::font::Font, bytes: &[u8], (x, y): (i32, i32)) -> Option<Prim> {
    if !(st.mask == 0xFF && matches!(st.mode, JAM1 | JAM2)) || st.algo != 0 { return None; }
    let role = if font.flags & retro_raster::font::FPF_PROPORTIONAL != 0 { FontRole::Ui } else { FontRole::Mono }; // FPF_PROPORTIONAL
    Some(Prim::Text { x, top: y - font.baseline as i32, w: font.text_length(bytes), h: font.y_size as i32, text: bytes.iter().map(|&b| b as char).collect(), fg: st.fg, bg: (st.mode == JAM2).then_some(st.bg), font: role })
}

/// Record one drawing call of window `win` whose effect (canvas coordinates) is `hit`: the named ops of `prim`, or a hole
/// when there is no prim, a pen has no role, or the DrawInfo cannot be read.
pub fn note<U: GuestUi>(a: &mut Amiga<U>, win: u32, prim: Option<Prim>, hit: Rect, origin: (i32, i32)) {
    if hit.is_empty() || !a.gfx.present.plane_route() { return; }
    let ops = prim.and_then(|p| PenRoles::read(a).ok().and_then(|roles| p.ops(&roles, origin, hit)));
    match ops {
        Some(v) => for (op, bbox) in v { a.gfx.present.record(win, op, bbox); },
        None => a.gfx.present.record_hole(win, hit),
    }
}
