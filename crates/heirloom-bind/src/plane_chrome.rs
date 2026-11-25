//! Declarative in-plane chrome bindings (`HEIRLOOM-APP-CHROME-NEEDS-A-BINDING`): DATA that names what regions of a
//! program's own drawn content plane (rulers, toolbars, redundant resize boxes) mean, so the engine rewrites them into
//! native seam ops (`DrawOp::Face`, `DrawOp::Icon`, theme `Fill`/`Line` roles) with zero per-app Rust logic.
//!
//! Keyed strictly by what a guest resource IS ([`PlaneChromeIdentity`]: resource type, ID, exact byte length, and FNV-1a 64
//! digest), never by a program's or file's name. Any other build or modified resource stays untouched.
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{DrawOp, GuestPoint, GuestRect, IconRole, PalettePart, PenRole};

/// What identifies the exact guest binary build a [`PlaneChromeBinding`] belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneChromeIdentity {
    /// A resource in the guest's resource chain: type (e.g. `CODE`), ID, exact byte length, and FNV-1a 64 digest.
    Resource { res_type: [u8; 4], res_id: i16, length: u32, fnv1a64: u64 },
}

/// Inclusive pixel-size window filter (`max == 0` means unbounded above).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRange {
    pub min_w: i32,
    pub max_w: i32,
    pub min_h: i32,
    pub max_h: i32,
}

impl WindowRange {
    pub fn matches(&self, win_w: i32, win_h: i32) -> bool {
        win_w >= self.min_w
            && (self.max_w <= 0 || win_w <= self.max_w)
            && win_h >= self.min_h
            && (self.max_h <= 0 || win_h <= self.max_h)
    }
}

/// Content-identifying guard over a guest `DrawOp::Pixels` box: FNV-1a 64 of the 1-bit dark-pixel mask (`luma < dark_luma`).
/// Invariant to active/inactive background shade so a palette stays identified across tab switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelMaskGuard {
    pub rect: GuestRect,
    pub dark_luma: u8,
    pub fnv1a64: u64,
}

/// One tab entry in [`PaletteRuleKind::Tabs`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabProbeRule {
    pub label: String,
    /// Sample box on the tab's background; the brightest tab (highest mean luma) is active.
    pub probe_rect: GuestRect,
}

/// One tool cell in [`PaletteRuleKind::ToolGrid`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolCellRule {
    pub icon: IconRole,
    pub has_flyout: bool,
    pub break_after: bool,
    /// Sample box checked for sunken/active shading (darkest cell noticeably darker than max luma).
    pub probe_rect: GuestRect,
}

/// One action key in [`PaletteRuleKind::Footer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FooterKeyRule {
    pub icon: IconRole,
    /// Optional sample box; when present, checks whether dark ink pixels exist (enabled vs ghosted).
    pub probe_rect: Option<GuestRect>,
}

/// One toggle key in [`PaletteRuleKind::MiniToggles`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MiniToggleRule {
    pub icon: IconRole,
    /// Sample box checked for latched state (`mean_luma < threshold_luma`).
    pub probe_rect: GuestRect,
    pub threshold_luma: u8,
}

/// What a [`PaletteRule`] emits into the display list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteRuleKind {
    /// Emit `DrawOp::Fill { rect, pen_role }` (typically `PenRole::Surface`).
    SurfaceFill { pen_role: PenRole },
    /// Crop and preserve the guest pixels inside `rect` from incoming `DrawOp::Pixels` ops.
    KeepPixels,
    /// Emit `DrawOp::Palette(PalettePart::Tabs)` with live active-tab probing.
    Tabs { tabs: Vec<TabProbeRule>, flyout: bool },
    /// Emit `DrawOp::Palette(PalettePart::ToolGrid)` with live active-tool probing.
    ToolGrid { cols: u8, tools: Vec<ToolCellRule> },
    /// Emit `DrawOp::Palette(PalettePart::ColorWell)` by sampling live `fore` and `back` RGB pixels.
    ColorWell { fore_pt: GuestPoint, back_pt: GuestPoint },
    /// Emit `DrawOp::Palette(PalettePart::Footer)`.
    Footer { keys: Vec<FooterKeyRule> },
    /// Emit `DrawOp::Palette(PalettePart::MiniToggles)` with live latched-bit probing.
    MiniToggles { keys: Vec<MiniToggleRule> },
    /// Emit `DrawOp::Palette(PalettePart::GradientSlider)` with optional live thumb scan along `track_rect`.
    GradientSlider {
        label: String,
        stops: Vec<u32>,
        track_rect: Option<GuestRect>,
        default_permille: u16,
    },
    /// Emit compact 11 px `DrawOp::Text` label (e.g. `"Feather:"`, `"Opacity:"`, `"Lock:"`, `"Fill:"`, `"100%"`).
    Label { text: String, muted: bool },
    /// Emit `DrawOp::Palette(PalettePart::Popup)` (`PART_POPUP_COMPACT`).
    Popup { label: String, icon: Option<IconRole>, enabled: bool },
    /// Emit `DrawOp::Palette(PalettePart::Check)` (`PART_CHECK_COMPACT`), optionally probing dark checkmark ink.
    Check { label: String, probe_rect: Option<GuestRect>, enabled: bool },
    /// Emit `DrawOp::Palette(PalettePart::FieldXs)` (`PART_FIELD_XS` / `PART_NUMERIC_FIELD`).
    FieldXs { value: String, unit: String, numeric_cap: bool, enabled: bool },
    /// Emit `DrawOp::Palette(PalettePart::MiniSlider)` (`PART_MINI_SLIDER`).
    MiniSlider { track_rect: Option<GuestRect>, default_permille: u16, enabled: bool },
}

/// One declarative compact-palette interior rule.
/// Coordinates in `rect` and `clear_rect` support edge anchoring:
/// `left < 0` anchors to `win_w + left`, `top < 0` to `win_h + top`,
/// `width <= 0` stretches to `win_w - resolved_left + width`,
/// `height <= 0` stretches to `win_h - resolved_top + height`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRule {
    pub rect: GuestRect,
    pub clear_rect: Option<GuestRect>,
    pub kind: PaletteRuleKind,
}

/// A toolbar / ruler band whose raw white background and heavy black border lines become a Sandstone theme strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BandRule {
    pub rect: GuestRect,
    /// Convert pure 1-bit black-on-white `DrawOp::Pixels` blits inside the band into transparent `PenRole::Text` spans.
    pub remap_1bit_pixels: bool,
}

/// One in-plane button / toggle control declared over a region of the window's content plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonRule {
    /// The target box where the native kit `DrawOp::Face` (and optional `DrawOp::Icon`) is emitted.
    pub rect: GuestRect,
    /// Guest drawing ops whose bounding box lies inside `clear_rect` are stripped (covers shadow lines around `rect`).
    pub clear_rect: GuestRect,
    /// Optional interior box checked for inversion / dark pixels to determine `pressed` (active/latched) state.
    pub active_rect: Option<GuestRect>,
    /// Label text for `DrawOp::Face` (use `" "` for icon-only faces; empty falls back to `" "`).
    pub label: String,
    /// Optional semantic icon role overlaid centered on the button face.
    pub icon: Option<IconRole>,
}

/// One declarative in-plane chrome binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaneChromeBinding {
    pub id: String,
    pub identity: PlaneChromeIdentity,
    /// When set, `bands` and `buttons` apply only while a `DrawOp::Text` containing this substring is live in the plane
    /// (so toggling a ruler/toolbar off in the guest automatically disables the band/button rules).
    pub present_if_text: Option<String>,
    pub bands: Vec<BandRule>,
    pub buttons: Vec<ButtonRule>,
    /// Suppress redundant guest-drawn grow/resize chrome inside the bottom-right `(w, h)` corner of the window.
    pub suppress_bottom_right: Option<(i32, i32)>,
    /// Optional window pixel-size filter (used by compact-palette bindings so each rule set targets one palette geometry).
    pub window_range: Option<WindowRange>,
    /// Optional 1-bit dark-pixel mask guards (all must match) to distinguish same-sized palettes without app names.
    pub pixel_guards: Vec<PixelMaskGuard>,
    /// Declarative compact-palette rules (`DrawOp::Fill`, `KeepPixels`, `DrawOp::Palette`).
    pub palette_rules: Vec<PaletteRule>,
}

fn op_bounds(op: &DrawOp) -> GuestRect {
    match op {
        DrawOp::Fill { rect, .. }
        | DrawOp::Bevel { rect, .. }
        | DrawOp::Icon { rect, .. }
        | DrawOp::Pixels { rect, .. }
        | DrawOp::Swatch { rect, .. }
        | DrawOp::Face { rect, .. }
        | DrawOp::Palette { rect, .. } => *rect,
        DrawOp::Line { a, b, .. } => GuestRect {
            left: a.x.min(b.x),
            top: a.y.min(b.y),
            width: (a.x - b.x).abs() + 1,
            height: (a.y - b.y).abs() + 1,
        },
        DrawOp::Text { at, px, width, .. } => GuestRect {
            left: at.x,
            top: at.y,
            width: (*width).max(1),
            height: (*px as i32).max(1),
        },
    }
}

/// True when a cropped `KeepPixels` box is pure empty Platinum chrome grey (neutral light grey with no document pixels,
/// borders, text, or colour ramps). Skipping it lets the underlying `PenRole::Surface` fill show through cleanly in dark mode.
fn is_uniform_platinum_ground(rgb: &[u32]) -> bool {
    if rgb.is_empty() {
        return true;
    }
    let mut min_l = 255u32;
    let mut max_l = 0u32;
    for &px in rgb {
        let r = ((px >> 16) & 0xFF) as i32;
        let g = ((px >> 8) & 0xFF) as i32;
        let b = (px & 0xFF) as i32;
        if (r - g).abs() > 6 || (g - b).abs() > 6 || (r - b).abs() > 6 {
            return false;
        }
        let l = luma(px);
        if l < 185 || l > 248 {
            return false;
        }
        if l < min_l { min_l = l; }
        if l > max_l { max_l = l; }
    }
    max_l.saturating_sub(min_l) <= 36
}

fn luma(rgb: u32) -> u32 {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    (r * 299 + g * 587 + b * 114) / 1000
}

fn resolve_rect(r: GuestRect, win_w: i32, win_h: i32) -> GuestRect {
    let left = if r.left < 0 { (win_w + r.left).max(0) } else { r.left };
    let top = if r.top < 0 { (win_h + r.top).max(0) } else { r.top };
    let width = if r.width <= 0 { (win_w - left + r.width).max(0) } else { r.width };
    let height = if r.height <= 0 { (win_h - top + r.height).max(0) } else { r.height };
    GuestRect { left, top, width, height }
}

fn resolve_point(p: GuestPoint, win_w: i32, win_h: i32) -> GuestPoint {
    GuestPoint {
        x: if p.x < 0 { (win_w + p.x).max(0) } else { p.x },
        y: if p.y < 0 { (win_h + p.y).max(0) } else { p.y },
    }
}

fn sample_rgb(ops: &[DrawOp], x: i32, y: i32) -> Option<u32> {
    for op in ops.iter().rev() {
        if let DrawOp::Pixels { rect, rgb } = op {
            if x >= rect.left
                && x < rect.left + rect.width
                && y >= rect.top
                && y < rect.top + rect.height
            {
                let idx = ((y - rect.top) * rect.width + (x - rect.left)) as usize;
                if let Some(&px) = rgb.get(idx) {
                    return Some(px & 0x00FF_FFFF);
                }
            }
        }
    }
    None
}

fn mean_luma(ops: &[DrawOp], area: GuestRect) -> Option<u32> {
    if area.width <= 0 || area.height <= 0 {
        return None;
    }
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in area.top..(area.top + area.height) {
        for x in area.left..(area.left + area.width) {
            if let Some(px) = sample_rgb(ops, x, y) {
                sum += luma(px) as u64;
                count += 1;
            }
        }
    }
    if count == 0 { None } else { Some((sum / count) as u32) }
}

/// FNV-1a 64 hash of the 1-bit dark-pixel mask (`luma < dark_luma`) over `area`, row-major.
pub fn dark_mask_hash(ops: &[DrawOp], area: GuestRect, dark_luma: u8) -> Option<u64> {
    if area.width <= 0 || area.height <= 0 {
        return None;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut any = false;
    let thr = u32::from(dark_luma);
    for y in area.top..(area.top + area.height) {
        for x in area.left..(area.left + area.width) {
            let bit = match sample_rgb(ops, x, y) {
                Some(px) => {
                    any = true;
                    u8::from(luma(px) < thr)
                }
                None => 0,
            };
            h ^= u64::from(bit);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    if any { Some(h) } else { None }
}

fn crop_pixels(ops: &[DrawOp], area: GuestRect) -> Option<Vec<u32>> {
    if area.width <= 0 || area.height <= 0 {
        return None;
    }
    let mut out = Vec::with_capacity((area.width * area.height) as usize);
    let mut any = false;
    for y in area.top..(area.top + area.height) {
        for x in area.left..(area.left + area.width) {
            if let Some(px) = sample_rgb(ops, x, y) {
                any = true;
                out.push(px);
            } else {
                out.push(0x00DD_DDDD);
            }
        }
    }
    if any { Some(out) } else { None }
}

fn probe_slider_permille(ops: &[DrawOp], tr: GuestRect, default_permille: u16) -> u16 {
    if tr.width <= 1 || tr.height <= 0 {
        return default_permille.min(1000);
    }
    let mut col_sum = 0u64;
    let mut col_cnt = 0u64;
    let mut best_x = 0i32;
    let mut min_col = u32::MAX;
    for dx in 0..tr.width {
        let mut s = 0u64;
        let mut c = 0u64;
        for dy in 0..tr.height {
            if let Some(px) = sample_rgb(ops, tr.left + dx, tr.top + dy) {
                s += luma(px) as u64;
                c += 1;
            }
        }
        if c > 0 {
            let avg = (s / c) as u32;
            col_sum += avg as u64;
            col_cnt += 1;
            if avg < min_col {
                min_col = avg;
                best_x = dx;
            }
        }
    }
    if col_cnt == 0 {
        return default_permille.min(1000);
    }
    let overall = (col_sum / col_cnt) as u32;
    if min_col + 25 <= overall {
        ((best_x as u64 * 1000) / (tr.width as u64 - 1)).min(1000) as u16
    } else {
        default_permille.min(1000)
    }
}

fn within(inner: GuestRect, outer: GuestRect) -> bool {
    inner.left >= outer.left
        && inner.top >= outer.top
        && inner.left + inner.width <= outer.left + outer.width
        && inner.top + inner.height <= outer.top + outer.height
}

fn overlaps(a: GuestRect, b: GuestRect) -> bool {
    a.left < b.left + b.width
        && a.left + a.width > b.left
        && a.top < b.top + b.height
        && a.top + a.height > b.top
}

fn is_active_in(ops: &[DrawOp], area: GuestRect) -> bool {
    let mut active = false;
    for op in ops {
        match op {
            DrawOp::Fill { rect, pen_role: PenRole::Selection } if overlaps(*rect, area) => {
                active = true;
            }
            DrawOp::Pixels { rect, rgb } if overlaps(*rect, area) && !rgb.is_empty() => {
                // An InvertRect over drawn icon lines snapshots inverted pixels (mostly dark ink, >55% dark).
                let mut total = 0usize;
                let mut dark = 0usize;
                for y in 0..rect.height {
                    let gy = rect.top + y;
                    if gy < area.top || gy >= area.top + area.height {
                        continue;
                    }
                    for x in 0..rect.width {
                        let gx = rect.left + x;
                        if gx < area.left || gx >= area.left + area.width {
                            continue;
                        }
                        let px = rgb[(y * rect.width + x) as usize] & 0x00FF_FFFF;
                        total += 1;
                        if px == 0 {
                            dark += 1;
                        }
                    }
                }
                if total > 0 && dark * 100 >= total * 55 {
                    active = true;
                }
            }
            _ => {}
        }
    }
    active
}

/// Convert a pure 1-bit black-on-white `Pixels` blit into horizontal `Fill(PenRole::Text)` runs so the guest's
/// transparent 1-bit icons/markers sit seamlessly over the Sandstone band without opaque white rectangles.
fn try_1bit_to_text_fills(rect: GuestRect, rgb: &[u32], out: &mut Vec<DrawOp>) -> bool {
    if rect.width <= 0 || rect.height <= 0 || rgb.len() != (rect.width * rect.height) as usize {
        return false;
    }
    if !rgb.iter().all(|&p| {
        let c = p & 0x00FF_FFFF;
        c == 0 || c == 0x00FF_FFFF
    }) {
        return false;
    }
    for y in 0..rect.height {
        let row = (y * rect.width) as usize;
        let mut x = 0i32;
        while x < rect.width {
            while x < rect.width && (rgb[row + x as usize] & 0x00FF_FFFF) != 0 {
                x += 1;
            }
            let start = x;
            while x < rect.width && (rgb[row + x as usize] & 0x00FF_FFFF) == 0 {
                x += 1;
            }
            if x > start {
                out.push(DrawOp::Fill {
                    rect: GuestRect { left: rect.left + start, top: rect.top + y, width: x - start, height: 1 },
                    pen_role: PenRole::Text,
                });
            }
        }
    }
    true
}

impl PlaneChromeBinding {
    /// Rewrite `ops` for a window of pixel size `(win_w, win_h)` according to this binding's declarative rules.
    pub fn apply(&self, ops: Vec<DrawOp>, win_size: (i32, i32)) -> Vec<DrawOp> {
        let (win_w, win_h) = win_size;
        if let Some(wr) = self.window_range {
            if !wr.matches(win_w, win_h) {
                return ops;
            }
        }
        if !self.pixel_guards.is_empty() {
            let all_match = self.pixel_guards.iter().all(|g| {
                let area = resolve_rect(g.rect, win_w, win_h);
                dark_mask_hash(&ops, area, g.dark_luma) == Some(g.fnv1a64)
            });
            if !all_match {
                return ops;
            }
        }
        let corner_box = self.suppress_bottom_right.map(|(cw, ch)| GuestRect {
            left: (win_w - cw).max(0),
            top: (win_h - ch).max(0),
            width: cw,
            height: ch,
        });
        let chrome_live = match &self.present_if_text {
            Some(needle) => ops.iter().any(|op| matches!(op, DrawOp::Text { string, .. } if string.contains(needle.as_str()))),
            None => self.buttons.is_empty()
                || self.buttons.iter().any(|btn| ops.iter().any(|op| within(op_bounds(op), btn.clear_rect))),
        };

        let active_states: Vec<bool> = if chrome_live {
            self.buttons
                .iter()
                .map(|b| b.active_rect.is_some_and(|ar| is_active_in(&ops, ar)))
                .collect()
        } else {
            Vec::new()
        };

        // Pre-resolve palette clear boxes and evaluate live pixel probes BEFORE dropping guest ops.
        let palette_clears: Vec<GuestRect> = if chrome_live {
            self.palette_rules
                .iter()
                .filter_map(|r| r.clear_rect.map(|cr| resolve_rect(cr, win_w, win_h)))
                .filter(|r| r.width > 0 && r.height > 0)
                .collect()
        } else {
            Vec::new()
        };

        let mut palette_emitted: Vec<DrawOp> = Vec::with_capacity(self.palette_rules.len());
        if chrome_live && !self.palette_rules.is_empty() {
            for rule in &self.palette_rules {
                let rect = resolve_rect(rule.rect, win_w, win_h);
                if rect.width <= 0 || rect.height <= 0 {
                    continue;
                }
                match &rule.kind {
                    PaletteRuleKind::SurfaceFill { pen_role } => {
                        palette_emitted.push(DrawOp::Fill { rect, pen_role: *pen_role });
                    }
                    PaletteRuleKind::KeepPixels => {
                        if let Some(rgb) = crop_pixels(&ops, rect) {
                            if !is_uniform_platinum_ground(&rgb) {
                                palette_emitted.push(DrawOp::Pixels { rect, rgb });
                            }
                        }
                    }
                    PaletteRuleKind::Tabs { tabs, flyout } => {
                        if tabs.is_empty() {
                            continue;
                        }
                        let mut active: u16 = 0;
                        let mut best_luma: u32 = 0;
                        for (idx, t) in tabs.iter().enumerate() {
                            let area = resolve_rect(t.probe_rect, win_w, win_h);
                            if let Some(m) = mean_luma(&ops, area) {
                                if m > best_luma {
                                    best_luma = m;
                                    active = idx as u16;
                                }
                            }
                        }
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::Tabs {
                                labels: tabs.iter().map(|t| t.label.clone()).collect(),
                                active,
                                flyout: *flyout,
                            },
                        });
                    }
                    PaletteRuleKind::ToolGrid { cols, tools } => {
                        if *cols == 0 || tools.is_empty() {
                            continue;
                        }
                        let lumas: Vec<Option<u32>> = tools
                            .iter()
                            .map(|t| mean_luma(&ops, resolve_rect(t.probe_rect, win_w, win_h)))
                            .collect();
                        let max_l = lumas.iter().flatten().copied().max().unwrap_or(0);
                        let mut active: i32 = -1;
                        let mut min_l = u32::MAX;
                        for (idx, l) in lumas.iter().enumerate() {
                            if let Some(v) = *l {
                                if v < min_l {
                                    min_l = v;
                                    active = idx as i32;
                                }
                            }
                        }
                        if min_l == u32::MAX || min_l + 10 > max_l {
                            active = -1;
                        }
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::ToolGrid {
                                cols: *cols,
                                tools: tools.iter().map(|t| (t.icon, t.has_flyout, t.break_after)).collect(),
                                active,
                            },
                        });
                    }
                    PaletteRuleKind::ColorWell { fore_pt, back_pt } => {
                        let fp = resolve_point(*fore_pt, win_w, win_h);
                        let bp = resolve_point(*back_pt, win_w, win_h);
                        let fore = sample_rgb(&ops, fp.x, fp.y).unwrap_or(0x0000_0000);
                        let back = sample_rgb(&ops, bp.x, bp.y).unwrap_or(0x00FF_FFFF);
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::ColorWell { fore, back, active: 0 },
                        });
                    }
                    PaletteRuleKind::Footer { keys } => {
                        let out_keys = keys
                            .iter()
                            .map(|k| {
                                let enabled = match k.probe_rect {
                                    None => true,
                                    Some(pr) => {
                                        let area = resolve_rect(pr, win_w, win_h);
                                        // Enabled icon glyphs have dark ink (< 100 luma); ghosted icons are washed gray.
                                        let mut has_dark = false;
                                        for y in area.top..(area.top + area.height) {
                                            for x in area.left..(area.left + area.width) {
                                                if sample_rgb(&ops, x, y).is_some_and(|px| luma(px) < 100) {
                                                    has_dark = true;
                                                    break;
                                                }
                                            }
                                            if has_dark {
                                                break;
                                            }
                                        }
                                        has_dark
                                    }
                                };
                                (k.icon, false, enabled)
                            })
                            .collect();
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::Footer { keys: out_keys },
                        });
                    }
                    PaletteRuleKind::MiniToggles { keys } => {
                        let mut latched: u32 = 0;
                        for (idx, k) in keys.iter().enumerate() {
                            if idx < 32 {
                                let area = resolve_rect(k.probe_rect, win_w, win_h);
                                if mean_luma(&ops, area).is_some_and(|m| m < u32::from(k.threshold_luma)) {
                                    latched |= 1u32 << idx;
                                }
                            }
                        }
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::MiniToggles {
                                keys: keys.iter().map(|k| k.icon).collect(),
                                latched,
                            },
                        });
                    }
                    PaletteRuleKind::GradientSlider { label, stops, track_rect, default_permille } => {
                        if stops.len() < 2 {
                            continue;
                        }
                        let permille = match track_rect {
                            Some(tr) => probe_slider_permille(&ops, resolve_rect(*tr, win_w, win_h), *default_permille),
                            None => (*default_permille).min(1000),
                        };
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::GradientSlider {
                                label: label.clone(),
                                stops: stops.clone(),
                                permille,
                            },
                        });
                    }
                    PaletteRuleKind::Label { text, muted } => {
                        palette_emitted.push(DrawOp::Text {
                            at: GuestPoint { x: rect.left, y: rect.top },
                            string: text.clone(),
                            font_role: heirloom_ui_abi::FontRole::Ui,
                            px: 11,
                            width: rect.width,
                            pen_role: if *muted { PenRole::TextMuted } else { PenRole::Text },
                            bg_role: None,
                        });
                    }
                    PaletteRuleKind::Popup { label, icon, enabled } => {
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::Popup {
                                label: label.clone(),
                                icon: *icon,
                                enabled: *enabled,
                            },
                        });
                    }
                    PaletteRuleKind::Check { label, probe_rect, enabled } => {
                        let checked = match probe_rect {
                            None => false,
                            Some(pr) => {
                                let area = resolve_rect(*pr, win_w, win_h);
                                mean_luma(&ops, area).is_some_and(|m| m < 210)
                            }
                        };
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::Check {
                                label: label.clone(),
                                checked,
                                enabled: *enabled,
                            },
                        });
                    }
                    PaletteRuleKind::FieldXs { value, unit, numeric_cap, enabled } => {
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::FieldXs {
                                value: value.clone(),
                                unit: unit.clone(),
                                numeric_cap: *numeric_cap,
                                enabled: *enabled,
                            },
                        });
                    }
                    PaletteRuleKind::MiniSlider { track_rect, default_permille, enabled } => {
                        let permille = match track_rect {
                            Some(tr) => probe_slider_permille(&ops, resolve_rect(*tr, win_w, win_h), *default_permille),
                            None => (*default_permille).min(1000),
                        };
                        palette_emitted.push(DrawOp::Palette {
                            rect,
                            part: PalettePart::MiniSlider { permille, enabled: *enabled },
                        });
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(ops.len() + self.buttons.len() * 2 + self.bands.len() * 2 + palette_emitted.len());

        // Emit Sandstone toolbar band ground first so remaining ruler ticks/text sit cleanly on top.
        if chrome_live {
            for band in &self.bands {
                let w = band.rect.width.min(win_w.saturating_sub(band.rect.left)).max(0);
                let h = band.rect.height.min(win_h.saturating_sub(band.rect.top)).max(0);
                if w > 0 && h > 0 {
                    let r = GuestRect { left: band.rect.left, top: band.rect.top, width: w, height: h };
                    out.push(DrawOp::Fill { rect: r, pen_role: PenRole::Highlight });
                    let foot_y = r.top + r.height - 1;
                    out.push(DrawOp::Line {
                        a: GuestPoint { x: r.left, y: foot_y },
                        b: GuestPoint { x: r.left + r.width - 1, y: foot_y },
                        pen_role: PenRole::Shadow,
                    });
                }
            }
        }

        for op in ops {
            let b = op_bounds(&op);
            if let Some(cb) = corner_box {
                if within(b, cb) {
                    continue;
                }
            }
            if chrome_live {
                if palette_clears.iter().any(|cr| within(b, *cr)) {
                    continue;
                }
                if self.buttons.iter().any(|btn| within(b, btn.clear_rect)) {
                    continue;
                }
                let mut handled = false;
                for band in &self.bands {
                    if !overlaps(b, band.rect) {
                        continue;
                    }
                    match &op {
                        // Drop the guest's white band erase fill and thick top/bottom black border strokes.
                        DrawOp::Fill { rect, pen_role: PenRole::Surface } if within(*rect, band.rect) || (rect.top == band.rect.top && rect.height <= band.rect.height + 4) => {
                            handled = true;
                            break;
                        }
                        DrawOp::Fill { rect, pen_role: PenRole::Text | PenRole::TextMuted }
                            if rect.height <= 4 && (rect.top <= band.rect.top + 3 || rect.top >= band.rect.top + band.rect.height - 4) =>
                        {
                            handled = true;
                            break;
                        }
                        DrawOp::Line { a, b: lb, .. }
                            if a.y == lb.y && (a.y <= band.rect.top + 3 || a.y >= band.rect.top + band.rect.height - 4) =>
                        {
                            handled = true;
                            break;
                        }
                        // Soften the ruler's horizontal/vertical tick lines to muted Sandstone ink.
                        DrawOp::Line { a, b: lb, pen_role: PenRole::Text } if within(b, band.rect) => {
                            out.push(DrawOp::Line { a: *a, b: *lb, pen_role: PenRole::TextMuted });
                            handled = true;
                            break;
                        }
                        // Strip opaque white backgrounds from text inside the toolbar band so it sits on SURFACE2.
                        DrawOp::Text { at, string, font_role, px, width, pen_role, bg_role: Some(PenRole::Surface) } if within(b, band.rect) => {
                            out.push(DrawOp::Text {
                                at: *at,
                                string: string.clone(),
                                font_role: *font_role,
                                px: *px,
                                width: *width,
                                pen_role: *pen_role,
                                bg_role: None,
                            });
                            handled = true;
                            break;
                        }
                        DrawOp::Pixels { rect, .. }
                            if rect.height <= 3 && (rect.top <= band.rect.top + 3 || rect.top >= band.rect.top + band.rect.height - 4) =>
                        {
                            handled = true;
                            break;
                        }
                        DrawOp::Pixels { rect, rgb } if band.remap_1bit_pixels && within(*rect, band.rect) => {
                            if try_1bit_to_text_fills(*rect, rgb, &mut out) {
                                handled = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if handled {
                    continue;
                }
            }
            out.push(op);
        }

        if chrome_live {
            out.extend(palette_emitted);
            for (btn, &pressed) in self.buttons.iter().zip(active_states.iter()) {
                let label = if btn.label.is_empty() { String::from(" ") } else { btn.label.clone() };
                out.push(DrawOp::Face { rect: btn.rect, label, pressed, accent: None });
                if let Some(role) = btn.icon {
                    out.push(DrawOp::Icon { rect: btn.rect, role });
                }
            }
        }

        out
    }
}

/// Find the binding matching `seen`, if any.
pub fn find<'a>(bindings: &'a [PlaneChromeBinding], seen: &PlaneChromeIdentity) -> Option<&'a PlaneChromeBinding> {
    bindings.iter().find(|b| b.identity == *seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use heirloom_ui_abi::FontRole;

    fn gr(left: i32, top: i32, width: i32, height: i32) -> GuestRect {
        GuestRect { left, top, width, height }
    }

    #[test]
    fn rewrites_band_buttons_active_state_and_suppresses_bottom_right_corner() {
        let binding = PlaneChromeBinding {
            id: String::from("test-ruler"),
            identity: PlaneChromeIdentity::Resource { res_type: *b"CODE", res_id: 1, length: 100, fnv1a64: 0x1234 },
            present_if_text: Some(String::from("6 lines/inch")),
            bands: vec![BandRule { rect: gr(0, 0, 487, 46), remap_1bit_pixels: true }],
            buttons: vec![
                ButtonRule {
                    rect: gr(190, 26, 34, 18),
                    clear_rect: gr(189, 25, 36, 20),
                    active_rect: Some(gr(193, 29, 28, 12)),
                    label: String::from("1\u{00d7}"),
                    icon: None,
                },
                ButtonRule {
                    rect: gr(326, 26, 34, 18),
                    clear_rect: gr(325, 25, 36, 20),
                    active_rect: Some(gr(329, 29, 28, 12)),
                    label: String::from(" "),
                    icon: Some(IconRole::AlignLeft),
                },
            ],
            suppress_bottom_right: Some((22, 22)),
            window_range: None,
            pixel_guards: vec![],
            palette_rules: vec![],
        };

        let ops = vec![
            DrawOp::Fill { rect: gr(0, 0, 487, 46), pen_role: PenRole::Surface },
            DrawOp::Text { at: GuestPoint { x: 92, y: 32 }, string: String::from("6 lines/inch"), font_role: FontRole::Ui, px: 12, width: 80, pen_role: PenRole::Text, bg_role: Some(PenRole::Surface) },
            // Active button 1 inverted via dark pixels
            DrawOp::Pixels { rect: gr(193, 29, 28, 12), rgb: vec![0x0000_0000; 28 * 12] },
            // Inactive button 2 frame line
            DrawOp::Line { a: GuestPoint { x: 327, y: 27 }, b: GuestPoint { x: 359, y: 27 }, pen_role: PenRole::Text },
            // 1-bit marker inside band (2x1 black-white)
            DrawOp::Pixels { rect: gr(10, 18, 2, 1), rgb: vec![0x0000_0000, 0x00FF_FFFF] },
            // Bottom-right grow box
            DrawOp::Fill { rect: gr(488, 279, 18, 18), pen_role: PenRole::Text },
        ];

        let out = binding.apply(ops, (506, 297));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Face { label, pressed: true, .. } if label == "1\u{00d7}")));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Face { pressed: false, .. })));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Icon { role: IconRole::AlignLeft, .. })));
        assert!(!out.iter().any(|o| matches!(o, DrawOp::Pixels { .. })), "1-bit marker converted to theme fills and inverted button stripped");
        assert!(!out.iter().any(|o| op_bounds(o).left >= 484 && op_bounds(o).top >= 275), "bottom-right corner suppressed");

        // NEGATIVE CONTROL: when signature text is absent, bands and buttons do not fire.
        let hidden = vec![DrawOp::Line { a: GuestPoint { x: 327, y: 27 }, b: GuestPoint { x: 359, y: 27 }, pen_role: PenRole::Text }];
        let out_hidden = binding.apply(hidden, (506, 297));
        assert!(!out_hidden.iter().any(|o| matches!(o, DrawOp::Face { .. })));
    }

    #[test]
    fn palette_rules_probe_live_tabs_tools_colors_sliders_and_respect_guards() {
        let mut buf = vec![0x00DD_DDDDu32; 200 * 100];
        let set_box = |b: &mut [u32], r: GuestRect, c: u32| {
            for y in r.top..(r.top + r.height) {
                for x in r.left..(r.left + r.width) {
                    b[(y * 200 + x) as usize] = c;
                }
            }
        };
        // Tab 0 dark/inactive, Tab 1 bright/active
        set_box(&mut buf, gr(4, 2, 40, 14), 0x00BB_BBBB);
        set_box(&mut buf, gr(50, 2, 40, 14), 0x00EE_EEEE);
        // Guard ink pixel inside (6, 4, 8, 8)
        set_box(&mut buf, gr(8, 6, 3, 3), 0x0010_1010);
        // Tool 0 inactive, Tool 1 active (darker)
        set_box(&mut buf, gr(4, 24, 20, 20), 0x00DD_DDDD);
        set_box(&mut buf, gr(26, 24, 20, 20), 0x0099_9999);
        // Live FG/BG color samples
        set_box(&mut buf, gr(60, 30, 8, 8), 0x00C8_2010);
        set_box(&mut buf, gr(74, 38, 8, 8), 0x0010_40A0);
        // Slider track at y=60, x=20..120 with black thumb at x=70 (500 permille)
        set_box(&mut buf, gr(20, 60, 101, 6), 0x00CC_CCDC);
        set_box(&mut buf, gr(70, 60, 1, 6), 0x0000_0000);
        // Latched toggle 0 (dark), unlatched toggle 1 (bright)
        set_box(&mut buf, gr(140, 24, 14, 14), 0x0080_8080);
        set_box(&mut buf, gr(158, 24, 14, 14), 0x00EE_EEEE);

        let ops = vec![DrawOp::Pixels { rect: gr(0, 0, 200, 100), rgb: buf }];
        let hash = dark_mask_hash(&ops, gr(6, 4, 8, 8), 80).unwrap();

        let binding = PlaneChromeBinding {
            id: String::from("test-palette"),
            identity: PlaneChromeIdentity::Resource { res_type: *b"WDEF", res_id: 128, length: 100, fnv1a64: 0x42 },
            present_if_text: None,
            bands: vec![],
            buttons: vec![],
            suppress_bottom_right: None,
            window_range: Some(WindowRange { min_w: 190, max_w: 210, min_h: 90, max_h: 110 }),
            pixel_guards: vec![PixelMaskGuard { rect: gr(6, 4, 8, 8), dark_luma: 80, fnv1a64: hash }],
            palette_rules: vec![
                PaletteRule {
                    rect: gr(0, 0, 0, 0),
                    clear_rect: Some(gr(0, 0, 0, 0)),
                    kind: PaletteRuleKind::SurfaceFill { pen_role: PenRole::Surface },
                },
                PaletteRule {
                    rect: gr(0, 0, 0, 20),
                    clear_rect: None,
                    kind: PaletteRuleKind::Tabs {
                        tabs: vec![
                            TabProbeRule { label: String::from("Color"), probe_rect: gr(4, 2, 40, 14) },
                            TabProbeRule { label: String::from("Swatches"), probe_rect: gr(50, 2, 40, 14) },
                        ],
                        flyout: true,
                    },
                },
                PaletteRule {
                    rect: gr(0, 22, 52, 24),
                    clear_rect: None,
                    kind: PaletteRuleKind::ToolGrid {
                        cols: 2,
                        tools: vec![
                            ToolCellRule { icon: IconRole::BoxSelect, has_flyout: true, break_after: false, probe_rect: gr(4, 24, 20, 20) },
                            ToolCellRule { icon: IconRole::Move, has_flyout: false, break_after: false, probe_rect: gr(26, 24, 20, 20) },
                        ],
                    },
                },
                PaletteRule {
                    rect: gr(56, 24, 36, 32),
                    clear_rect: None,
                    kind: PaletteRuleKind::ColorWell {
                        fore_pt: GuestPoint { x: 64, y: 34 },
                        back_pt: GuestPoint { x: 78, y: 42 },
                    },
                },
                PaletteRule {
                    rect: gr(10, 56, 120, 14),
                    clear_rect: None,
                    kind: PaletteRuleKind::GradientSlider {
                        label: String::from("R"),
                        stops: vec![0x0000_0000, 0x00FF_0000],
                        track_rect: Some(gr(20, 60, 101, 6)),
                        default_permille: 0,
                    },
                },
                PaletteRule {
                    rect: gr(136, 22, 40, 18),
                    clear_rect: None,
                    kind: PaletteRuleKind::MiniToggles {
                        keys: vec![
                            MiniToggleRule { icon: IconRole::Lock, probe_rect: gr(140, 24, 14, 14), threshold_luma: 180 },
                            MiniToggleRule { icon: IconRole::Paintbrush, probe_rect: gr(158, 24, 14, 14), threshold_luma: 180 },
                        ],
                    },
                },
                PaletteRule {
                    rect: gr(0, -20, 0, 20),
                    clear_rect: None,
                    kind: PaletteRuleKind::Footer {
                        keys: vec![FooterKeyRule { icon: IconRole::Trash, probe_rect: None }],
                    },
                },
            ],
        };

        let out = binding.apply(ops.clone(), (200, 100));
        assert!(!out.iter().any(|o| matches!(o, DrawOp::Pixels { .. })), "Platinum pixels cleared");
        assert!(out.iter().any(|o| matches!(o, DrawOp::Fill { rect, pen_role: PenRole::Surface } if *rect == gr(0, 0, 200, 100))));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Palette { part: PalettePart::Tabs { active: 1, flyout: true, .. }, .. })));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Palette { part: PalettePart::ToolGrid { active: 1, .. }, .. })));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Palette { part: PalettePart::ColorWell { fore: 0x00C8_2010, back: 0x0010_40A0, .. }, .. })));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Palette { part: PalettePart::GradientSlider { permille: 500, .. }, .. })));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Palette { part: PalettePart::MiniToggles { latched: 0b01, .. }, .. })));
        assert!(out.iter().any(|o| matches!(o, DrawOp::Palette { rect, part: PalettePart::Footer { .. } } if *rect == gr(0, 80, 200, 20))), "bottom-anchored footer");

        // NEGATIVE CONTROLS: wrong window size and wrong pixel guard leave guest ops untouched.
        let wrong_size = binding.apply(ops.clone(), (300, 100));
        assert!(wrong_size.iter().any(|o| matches!(o, DrawOp::Pixels { .. })));
        assert!(!wrong_size.iter().any(|o| matches!(o, DrawOp::Palette { .. })));

        let mut bad_guard = binding.clone();
        bad_guard.pixel_guards[0].fnv1a64 ^= 1;
        let wrong_guard = bad_guard.apply(ops, (200, 100));
        assert!(!wrong_guard.iter().any(|o| matches!(o, DrawOp::Palette { .. })));
    }
}
