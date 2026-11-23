//! Declarative in-plane chrome bindings (`HEIRLOOM-APP-CHROME-NEEDS-A-BINDING`): DATA that names what regions of a
//! program's own drawn content plane (rulers, toolbars, redundant resize boxes) mean, so the engine rewrites them into
//! native seam ops (`DrawOp::Face`, `DrawOp::Icon`, theme `Fill`/`Line` roles) with zero per-app Rust logic.
//!
//! Keyed strictly by what a guest resource IS ([`PlaneChromeIdentity`]: resource type, ID, exact byte length, and FNV-1a 64
//! digest), never by a program's or file's name. Any other build or modified resource stays untouched.
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{DrawOp, GuestPoint, GuestRect, IconRole, PenRole};

/// What identifies the exact guest binary build a [`PlaneChromeBinding`] belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneChromeIdentity {
    /// A resource in the guest's resource chain: type (e.g. `CODE`), ID, exact byte length, and FNV-1a 64 digest.
    Resource { res_type: [u8; 4], res_id: i16, length: u32, fnv1a64: u64 },
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
}

fn op_bounds(op: &DrawOp) -> GuestRect {
    match op {
        DrawOp::Fill { rect, .. }
        | DrawOp::Bevel { rect, .. }
        | DrawOp::Icon { rect, .. }
        | DrawOp::Pixels { rect, .. }
        | DrawOp::Swatch { rect, .. }
        | DrawOp::Face { rect, .. } => *rect,
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

        let mut out = Vec::with_capacity(ops.len() + self.buttons.len() * 2 + self.bands.len() * 2);

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
}
