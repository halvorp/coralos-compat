//! MUI's own layout for a window's tree (ruling 17 piece 7: declarative layouts stay the toolkit's): every object gets a size
//! from the REAL metrics of the screen font the engine installs (`tf_YSize`, `tf_XSize`, `TextLength`), groups arrange their
//! children in a grid, spare space goes to the objects that stretch, and the result is one rectangle per object that is a
//! control ([`super::roles`]). Facts: Zune's group.c (a group is a grid of columns/rows; `Horiz` = one row; `SameWidth/Height/Size`;
//! spacing `MUIA_Group_*Spacing`; `PageMode` shows one child), area.c (weights, fixed sizes, centring of what does not stretch),
//! string.c / slider.c / text.c AskMinMax (a string's default width is 12 `tf_XSize`, a text is its `TextLength`), and the default
//! prefs of configdata.c: group spacing 6/3, window inner spacing 4/4/3/3, radio spacing 4/1, frame inner spacing 2/2/1/1.
//! Objects open at their DEFAULT size; a resized window hands the spare space to the objects that stretch.
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use super::roles;
use crate::boopsi::Objects;
use guestui_abi::{ControlKind, GuestRect};
use retro_raster::font::Font;

/// Window inner spacing (left, right, top, bottom): configdata.c defaults.
pub const WIN_PAD: (i32, i32, i32, i32) = (4, 4, 3, 3);
const GROUP_HSPACE: i32 = 6;
const GROUP_VSPACE: i32 = 3;
const RADIO_HSPACE: i32 = 4;
const RADIO_VSPACE: i32 = 1;
/// A drawn frame: 2 px of border plus the default inner spacing 2/2/1/1 (configdata.c frame specs).
const FRAME_W: i32 = 8;
const FRAME_H: i32 = 6;
/// A group frame: 1 px border + inner 4 on every side (configdata.c "314444").
const GROUP_FRAME: i32 = 5;
const ARROW: i32 = 16;

/// What the layout decided for one control.
#[derive(Debug, Clone)]
pub struct Elem { pub obj: u32, pub kind: ControlKind, pub label: String, pub rect: GuestRect, pub enabled: bool }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Size { w: i32, h: i32, gw: bool, gh: bool }

/// What `place` produces: the controls, and the rectangle of every self-drawing object ([`super::drawn`]).
#[derive(Default)]
pub struct Placed { pub elems: Vec<Elem>, pub drawn: Vec<(u32, GuestRect)> }

pub struct Layout { pub width: i32, pub height: i32, pub elems: Vec<Elem>, pub drawn: Vec<(u32, GuestRect)> }

/// The host's control metrics (ruling 17 piece 7: a declarative layout opts into host metrics as a MEASURED profile, never silently).
/// Floors and paddings in window pixels that the host's native controls need; the default is all zero = Zune's own sizes. A host whose
/// kit draws a toggle 56x28 or a 28 px field sets them, and MUI's layout then sizes every cell, group and the window for those
/// controls, so nothing overlaps; the guest never sees the profile (its tree, attributes and notifications are untouched).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metrics {
    /// Smallest check box / toggle (width, height).
    pub toggle: (i32, i32),
    /// Smallest height of a button, string field, cycle, slider and gauge.
    pub control_h: i32,
    /// Extra width added to a button, field and cycle (the kit's inset on both sides together).
    pub pad_w: i32,
    /// The host draws a check box's label to the RIGHT of its toggle, `gap` px away (Gute Form order), instead of in MUI's label column to
    /// the left: the label's own cell collapses and the box's cell holds toggle + gap + label, so the text never lands in a neighbour.
    pub label_right: Option<i32>,
}

struct Ctx<'a> { o: &'a Objects, font: &'a Font, m: Metrics }
impl Ctx<'_> {
    fn fy(&self) -> i32 { self.font.y_size as i32 }
    fn text_w(&self, s: &str) -> i32 { self.font.text_length(s.as_bytes()) }
    fn num(&self, obj: u32, attr: u32) -> Option<i32> { self.o.num(obj, attr).map(|v| v as i32) }
}

/// How an object arranges what it holds.
#[derive(PartialEq, Eq)]
enum Shape { Leaf, Grid, Wrapper, Pair }
fn shape(k: Kind) -> Shape {
    match k {
        Kind::Group | Kind::Virtgroup | Kind::Register => Shape::Grid,
        Kind::Scrollgroup | Kind::Listview => Shape::Wrapper,
        Kind::Popstring | Kind::Popasl | Kind::Popobject => Shape::Pair,
        _ => Shape::Leaf,
    }
}

/// The children that take part in the layout: shown, and for a paged group only the active page. Menus and the like are not areas.
fn members(c: &Ctx, g: u32) -> Vec<u32> {
    let Some(kind) = c.o.kind(g) else { return Vec::new() };
    let all: Vec<u32> = c.o.children(g).iter().copied().filter(|&k| c.o.kind(k).is_some_and(|x| x.is_area()) && roles::shown(c.o, k)).collect();
    if (kind == Kind::Register || c.o.flag(g, MUIA_Group_PageMode)) && kind != Kind::Popstring {
        let active = c.o.num_or(g, MUIA_Group_ActivePage, 0) as usize;
        let all_pages: Vec<u32> = c.o.children(g).iter().copied().filter(|&k| c.o.kind(k).is_some_and(|x| x.is_area())).collect();
        return all_pages.get(active).copied().filter(|k| all.contains(k)).into_iter().collect();
    }
    all
}

struct Grid { cols: usize, col_w: Vec<i32>, row_h: Vec<i32>, col_g: Vec<bool>, row_g: Vec<bool>, hsp: i32, vsp: i32, pad: (i32, i32, i32, i32), kids: Vec<u32> }

fn spacing(c: &Ctx, g: u32) -> (i32, i32) {
    let both = c.num(g, MUIA_Group_Spacing);
    (c.num(g, MUIA_Group_HorizSpacing).or(both).unwrap_or(GROUP_HSPACE), c.num(g, MUIA_Group_VertSpacing).or(both).unwrap_or(GROUP_VSPACE))
}

/// The inner padding a group adds around its children (left, right, top, bottom): its frame and title, and a register's tab strip.
fn group_pad(c: &Ctx, g: u32) -> (i32, i32, i32, i32) {
    let kind = c.o.kind(g).unwrap_or(Kind::Group);
    if kind == Kind::Register { return (GROUP_FRAME, GROUP_FRAME, tab_h(c) + GROUP_FRAME, GROUP_FRAME); }
    if roles::frame_group(c.o, g).is_none() { return (0, 0, 0, 0); }
    let title = if c.o.text(g, MUIA_FrameTitle).is_empty() { 0 } else { c.fy() };
    (GROUP_FRAME, GROUP_FRAME, GROUP_FRAME + title, GROUP_FRAME)
}

fn tab_h(c: &Ctx) -> i32 { c.fy() + 8 }

fn grid(c: &Ctx, g: u32) -> Grid {
    let kids = members(c, g);
    let kind = c.o.kind(g).unwrap_or(Kind::Group);
    let n = kids.len().max(1);
    let (mut hsp, mut vsp) = spacing(c, g);
    let pages = kind == Kind::Register || c.o.flag(g, MUIA_Group_PageMode);
    let cols = if pages { 1 }
        else if let Some(r) = c.num(g, MUIA_Group_Rows).filter(|&r| r > 0) { n.div_ceil(r as usize) }
        else if let Some(cc) = c.num(g, MUIA_Group_Columns).filter(|&x| x > 0) { cc as usize }
        else if c.o.flag(g, MUIA_Group_Horiz) { n } else { 1 };
    if pages { (hsp, vsp) = (0, 0); }
    let rows = n.div_ceil(cols);
    let (mut col_w, mut row_h, mut col_g, mut row_g) = (alloc::vec![0; cols], alloc::vec![0; rows], alloc::vec![false; cols], alloc::vec![false; rows]);
    for i in 0..kids.len() {
        let s = measure_cell(c, &kids, i);
        let (ci, ri) = (i % cols, i / cols);
        col_w[ci] = col_w[ci].max(s.w);
        row_h[ri] = row_h[ri].max(s.h);
        col_g[ci] |= s.gw;
        row_g[ri] |= s.gh;
    }
    let same_w = c.o.flag(g, MUIA_Group_SameWidth) || c.o.flag(g, MUIA_Group_SameSize);
    let same_h = c.o.flag(g, MUIA_Group_SameHeight) || c.o.flag(g, MUIA_Group_SameSize);
    if same_w { let m = col_w.iter().copied().max().unwrap_or(0); col_w.iter_mut().for_each(|w| *w = m); }
    if same_h { let m = row_h.iter().copied().max().unwrap_or(0); row_h.iter_mut().for_each(|h| *h = m); }
    if pages { let (w, h) = (col_w.iter().copied().max().unwrap_or(0), row_h.iter().copied().max().unwrap_or(0)); col_w = alloc::vec![w; 1]; row_h = alloc::vec![h; 1]; }
    Grid { cols, col_w, row_h, col_g, row_g, hsp, vsp, pad: group_pad(c, g), kids }
}

/// The label MUI pairs with the check box `kids[i]` when the host draws it to the right: the MUIO_Label just before it.
fn toggle_label(c: &Ctx, kids: &[u32], i: usize) -> Option<String> {
    c.m.label_right?;
    if i >= kids.len() { return None; }
    let is_box = |k: u32| shape(c.o.kind(k).unwrap_or(Kind::Area)) == Shape::Leaf && matches!(roles::leaf(c.o, k), Some(ControlKind::Checkbox { .. }));
    (i > 0 && is_box(kids[i]) && roles::is_label(c.o, kids[i - 1])).then(|| roles::display_text(c.o, kids[i - 1], MUIA_Text_Contents))
}

/// The size of `kids[i]` as a cell of its group: a label that the host draws with its check box has no cell of its own, and the box's
/// cell is wide enough for the toggle, the gap and the text.
fn measure_cell(c: &Ctx, kids: &[u32], i: usize) -> Size {
    let mut s = measure(c, kids[i]);
    if let (Some(gap), Some(text)) = (c.m.label_right, toggle_label(c, kids, i)) { s.w += gap + c.text_w(&text); }
    if roles::is_label(c.o, kids[i]) && toggle_label(c, kids, i + 1).is_some() { s.w = 0; }
    s
}

fn total(v: &[i32], gap: i32) -> i32 { v.iter().sum::<i32>() + gap * (v.len() as i32 - 1).max(0) }

/// An object's size: its default size, and whether it takes spare space on either axis.
fn measure(c: &Ctx, o: u32) -> Size {
    let Some(kind) = c.o.kind(o) else { return Size::default() };
    let mut s = match shape(kind) {
        Shape::Grid => {
            let g = grid(c, o);
            let (w, h) = (total(&g.col_w, g.hsp) + g.pad.0 + g.pad.1, total(&g.row_h, g.vsp) + g.pad.2 + g.pad.3);
            let (gw, gh) = (g.col_g.iter().any(|&x| x), g.row_g.iter().any(|&x| x));
            let mut s = Size { w, h, gw, gh };
            if kind == Kind::Register { s.w = s.w.max(c.o.entries(o, MUIA_Register_Titles).iter().map(|t| c.text_w(t) + 16).sum()); }
            s
        }
        Shape::Wrapper => members(c, o).first().map_or(Size::default(), |&k| measure(c, k)),
        Shape::Pair => { let kids = members(c, o); let m: Vec<Size> = kids.iter().map(|&k| measure(c, k)).collect();
            Size { w: m.iter().map(|x| x.w).sum(), h: m.iter().map(|x| x.h).max().unwrap_or(0), gw: m.iter().any(|x| x.gw), gh: m.iter().any(|x| x.gh) } }
        Shape::Leaf => leaf_size(c, o, kind),
    };
    // Fixed sizes and explicit weights (Area): a fixed extent does not stretch; weight 0 does not either.
    if let Some(w) = c.num(o, MUIA_FixWidth).filter(|&w| w > 0) { s.w = w; s.gw = false; }
    if let Some(h) = c.num(o, MUIA_FixHeight).filter(|&h| h > 0) { s.h = h; s.gh = false; }
    if c.num(o, MUIA_Weight) == Some(0) || c.num(o, MUIA_HorizWeight) == Some(0) { s.gw = false; }
    if c.num(o, MUIA_Weight) == Some(0) || c.num(o, MUIA_VertWeight) == Some(0) { s.gh = false; }
    s
}

fn leaf_size(c: &Ctx, o: u32, kind: Kind) -> Size {
    let (fy, fx) = (c.fy(), c.font.x_size as i32);
    // A self-drawing object is as big as its own MUIM_AskMinMax said, and stretches where it allows more than its default.
    if let Some(d) = c.o.drawn.get(&o) { return Size { w: d.def.0, h: d.def.1, gw: d.max.0 > d.def.0, gh: d.max.1 > d.def.1 }; }
    match roles::leaf(c.o, o) {
        Some(ControlKind::Label { text, framed }) => {
            let (pw, ph) = if framed { (FRAME_W, FRAME_H) } else { (0, 0) };
            Size { w: c.text_w(&text) + pw, h: fy + ph, gw: !roles::is_label(c.o, o), gh: false }
        }
        Some(ControlKind::Button) | Some(ControlKind::ToggleButton { .. }) => {
            let text = roles::caption(c.o, o);
            let framed = c.o.num_or(o, MUIA_Frame, MUIV_Frame_None) != MUIV_Frame_None;
            let (pw, ph) = if framed { (FRAME_W, FRAME_H) } else { (0, 0) };
            Size { w: c.text_w(&text) + pw + c.m.pad_w, h: (fy + ph).max(c.m.control_h), gw: kind != Kind::Image, gh: false }
        }
        Some(ControlKind::Checkbox { .. }) => Size { w: (fy + 4).max(c.m.toggle.0), h: (fy + 4).max(c.m.toggle.1), gw: false, gh: false },
        Some(ControlKind::Field { .. }) => Size { w: fx * 12 + FRAME_W + c.m.pad_w, h: (fy + FRAME_H).max(c.m.control_h), gw: true, gh: false },
        Some(ControlKind::Choice { style: guestui_abi::ChoiceStyle::Radio, labels, .. }) => {
            let widest = labels.iter().map(|l| c.text_w(l)).max().unwrap_or(0);
            let n = labels.len() as i32;
            Size { w: fy + RADIO_HSPACE + widest, h: n * fy + (n - 1).max(0) * RADIO_VSPACE, gw: false, gh: false }
        }
        Some(ControlKind::Choice { labels, .. }) => {
            let widest = labels.iter().map(|l| c.text_w(l)).max().unwrap_or(0);
            Size { w: widest + ARROW + FRAME_W + c.m.pad_w, h: (fy + FRAME_H).max(c.m.control_h), gw: true, gh: false }
        }
        Some(ControlKind::Slider { .. }) => Size { w: fy * 4 + 2, h: (fy + 2).max(c.m.control_h), gw: true, gh: false },
        Some(ControlKind::Gauge { .. }) => Size { w: fx * 8 + FRAME_W, h: (fy + FRAME_H).max(c.m.control_h), gw: true, gh: false },
        Some(ControlKind::ScrollBar { vertical, .. }) => if vertical { Size { w: ARROW, h: ARROW * 3, gw: false, gh: true } } else { Size { w: ARROW * 3, h: ARROW, gw: true, gh: false } },
        Some(ControlKind::List { rows, columns, .. }) => {
            let widest = rows.iter().map(|r| r.iter().map(|c2| c.text_w(c2)).sum::<i32>() + 8 * r.len() as i32).max().unwrap_or(0).max(fx * 12);
            let head = if columns.iter().any(|h| !h.is_empty()) { fy + 2 } else { 0 };
            Size { w: widest + FRAME_W, h: head + fy * 3 + FRAME_H, gw: true, gh: true }
        }
        _ => match kind {
            Kind::Rectangle => {
                if c.o.flag(o, MUIA_Rectangle_HBar) { Size { w: 0, h: if roles::display_text(c.o, o, MUIA_Rectangle_BarTitle).is_empty() { 2 } else { fy }, gw: true, gh: false } }
                else if c.o.flag(o, MUIA_Rectangle_VBar) { Size { w: 2, h: 0, gw: false, gh: true } }
                else { Size { w: 0, h: 0, gw: true, gh: true } }
            }
            Kind::Balance => Size { w: 4, h: 4, gw: false, gh: false },
            Kind::Image => Size { w: 0, h: 0, gw: false, gh: false },
            _ => Size { w: 0, h: 0, gw: true, gh: true },
        },
    }
}

/// Lay out the tree under `root` (the window's root object) for a window whose content area is at least `avail`; margins included.
pub fn compute(o: &Objects, root: u32, font: &Font, avail: (i32, i32), metrics: Metrics) -> Layout {
    let c = Ctx { o, font, m: metrics };
    let s = measure(&c, root);
    let (px, py) = (WIN_PAD.0 + WIN_PAD.1, WIN_PAD.2 + WIN_PAD.3);
    let (w, h) = ((s.w + px).max(avail.0), (s.h + py).max(avail.1));
    let mut placed = Placed::default();
    let rect = GuestRect { left: WIN_PAD.0, top: WIN_PAD.2, width: w - px, height: h - py };
    place(&c, root, rect, &mut placed, &mut None);
    Layout { width: w, height: h, elems: placed.elems, drawn: placed.drawn }
}

fn push(c: &Ctx, o: u32, rect: GuestRect, label: Option<String>, out: &mut Placed) {
    if let Some(kind) = roles::leaf(c.o, o) {
        let label = match kind { ControlKind::Button | ControlKind::ToggleButton { .. } => roles::caption(c.o, o), _ => label.unwrap_or_default() };
        out.elems.push(Elem { obj: o, kind, label, rect, enabled: roles::enabled(c.o, o) });
    }
}

/// Give `o` the rectangle `r` (already the cell's, or the space the parent chose) and recurse.
fn place(c: &Ctx, o: u32, r: GuestRect, out: &mut Placed, pending: &mut Option<String>) {
    let Some(kind) = c.o.kind(o) else { return };
    match shape(kind) {
        Shape::Leaf => {
            if c.o.drawn.contains_key(&o) { out.drawn.push((o, r)); }
            let label = if roles::is_label(c.o, o) { None } else { pending.take() };
            push(c, o, r, label, out);
        }
        Shape::Wrapper => if let Some(&k) = members(c, o).first() { place(c, k, r, out, pending); },
        Shape::Pair => {
            let kids = members(c, o);
            let sizes: Vec<Size> = kids.iter().map(|&k| measure(c, k)).collect();
            let fixed: i32 = sizes.iter().filter(|s| !s.gw).map(|s| s.w).sum();
            let growers = sizes.iter().filter(|s| s.gw).count() as i32;
            let share = if growers > 0 { ((r.width - fixed) / growers).max(0) } else { 0 };
            let mut x = r.left;
            for (&k, s) in kids.iter().zip(&sizes) {
                let w = if s.gw { share } else { s.w };
                place(c, k, GuestRect { left: x, top: r.top, width: w, height: r.height }, out, &mut None);
                x += w;
            }
        }
        Shape::Grid => {
            let g = grid(c, o);
            if let Some(frame) = roles::frame_group(c.o, o) { out.elems.push(Elem { obj: o, kind: frame, label: String::new(), rect: r, enabled: roles::enabled(c.o, o) }); }
            let inner = GuestRect { left: r.left + g.pad.0, top: r.top + g.pad.2, width: r.width - g.pad.0 - g.pad.1, height: r.height - g.pad.2 - g.pad.3 };
            if kind == Kind::Register {
                push(c, o, GuestRect { left: r.left, top: r.top, width: r.width, height: tab_h(c) }, None, out);
            }
            let (mut cw, mut rh) = (g.col_w.clone(), g.row_h.clone());
            let (extra_w, extra_h) = (inner.width - total(&cw, g.hsp), inner.height - total(&rh, g.vsp));
            spread(&mut cw, &g.col_g, extra_w);
            spread(&mut rh, &g.row_g, extra_h);
            let mut label: Option<String> = None;
            let paged = kind == Kind::Register || c.o.flag(o, MUIA_Group_PageMode);
            for (i, &k) in g.kids.iter().enumerate() {
                let (ci, ri) = if paged { (0, 0) } else { (i % g.cols, i / g.cols) };
                let cx = inner.left + cw[..ci].iter().map(|w| w + g.hsp).sum::<i32>();
                let cy = inner.top + rh[..ri].iter().map(|h| h + g.vsp).sum::<i32>();
                // A MUIO_Label directly before a control is that control's label: it keeps its own cell (the label column).
                let next_is_control = g.kids.get(i + 1).is_some_and(|&n| !roles::is_label(c.o, n) && shape(c.o.kind(n).unwrap_or(Kind::Area)) == Shape::Leaf && roles::leaf(c.o, n).is_some());
                if roles::is_label(c.o, k) && next_is_control { label = Some(roles::display_text(c.o, k, MUIA_Text_Contents)); }
                let s = measure_cell(c, &g.kids, i);
                let (cw_i, rh_i) = (cw[ci], rh[ri]);
                let w = if s.gw { cw_i } else { s.w.min(cw_i) };
                let h = if s.gh { rh_i } else { s.h.min(rh_i) };
                let left = if toggle_label(c, &g.kids, i).is_some() { cx } else { cx + (cw_i - w) / 2 };
                let cell = GuestRect { left, top: cy + (rh_i - h) / 2, width: w, height: h };
                if roles::is_label(c.o, k) && next_is_control { continue; }
                place(c, k, cell, out, &mut label);
            }
        }
    }
}

/// Give `extra` pixels to the stretching columns (or rows), evenly.
fn spread(sizes: &mut [i32], grows: &[bool], extra: i32) {
    let n = grows.iter().filter(|&&g| g).count() as i32;
    if extra <= 0 || n == 0 { return; }
    let (each, mut rem) = (extra / n, extra % n);
    for (s, &g) in sizes.iter_mut().zip(grows) { if g { *s += each + if rem > 0 { rem -= 1; 1 } else { 0 }; } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spare_space_goes_to_the_stretching_columns_only() {
        let mut w = [10, 20, 30];
        spread(&mut w, &[true, false, true], 11);
        assert_eq!(w, [16, 20, 35], "5 and 6, the odd pixel to the first");
        spread(&mut w, &[false, false, false], 9);
        assert_eq!(w, [16, 20, 35], "nothing stretches: nothing moves");
        spread(&mut w, &[true, true, true], -4);
        assert_eq!(w, [16, 20, 35], "no spare space");
    }
}
