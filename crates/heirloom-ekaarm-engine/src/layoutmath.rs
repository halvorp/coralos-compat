//! The layout arithmetic around the decoded lines: `TAknWindowComponentLayout::Compose` and `LayoutLine` (layouts/aknlayout2/src/
//! AknLayout2Scalable.cpp:361-420 and 511-640, the release build) and `AknLayoutUtils::RectFromCoords` (avkon uifw/AvKon/src/
//! aknutils.cpp:2905), implemented natively over the 16-word line the decoder produces (cells at words 6..12: C l t r b W H).
use crate::layoutdecode::{Words, LAYOUT_EMPTY};

const P: i32 = 0x4000;
const P_RANGE: i32 = 0x2000;
const C: usize = 6;
const L: usize = 7;
const T: usize = 8;
const R: usize = 9;
const B: usize = 10;
const W: usize = 11;
const H: usize = 12;

/// `IsParentRelative`: p-based values lie in (P - range, P].
fn parent_relative(v: i32) -> bool {
    v > P - P_RANGE && v <= P
}

fn empty() -> i32 {
    LAYOUT_EMPTY as i16 as i32
}

fn get(w: &Words, i: usize) -> i32 {
    w[i] as i16 as i32
}

fn set(w: &mut Words, i: usize, v: i32) {
    w[i] = v as i16 as u16;
}

/// `TAknWindowComponentLayout()`: C, W, H empty, the rest zero.
fn blank() -> Words {
    let mut w: Words = [0; 16];
    set(&mut w, C, empty());
    set(&mut w, W, empty());
    set(&mut w, H, empty());
    w
}

/// A window line as `TAknWindowLineLayout` (`LayoutLine()`): one empty and one parent-relative value of each of l/W/r and t/H/b
/// trade places, so the line says what it means: `l + W + r = parent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub c: i32,
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
    pub w: i32,
    pub h: i32,
}

pub fn layout_line(w: &Words) -> Line {
    let mut v = [get(w, L), get(w, W), get(w, R)];
    let mut u = [get(w, T), get(w, H), get(w, B)];
    for axis in [&mut v, &mut u] {
        // The last of each kind wins (the source overwrites its pointers in order l, W, r).
        let empty_at = axis.iter().rposition(|&x| x == empty());
        let rel_at = axis.iter().rposition(|&x| parent_relative(x));
        let number_at = (0..3).rev().find(|&i| Some(i) != empty_at && Some(i) != rel_at);
        if let (Some(e), Some(p), Some(n)) = (empty_at, rel_at, number_at) {
            axis[e] = (P - axis[p] - axis[n]) as i16 as i32;
            axis[p] = empty();
        }
    }
    Line { c: get(w, C), l: v[0], t: u[0], r: v[2], b: u[2], w: v[1], h: u[1] }
}

/// `TAknWindowComponentLayout::Compose(aLine1, aLine2)`: line2 expressed inside line1's parent.
pub fn compose(mut a: Words, mut b: Words) -> Words {
    let e = empty();
    let fix = |x: &mut Words| {
        if get(x, R) == e { let v = P - get(x, W) - get(x, L); set(x, R, v) }
        if get(x, B) == e { let v = P - get(x, H) - get(x, T); set(x, B, v) }
        if get(x, L) == e { let v = P - get(x, W) - get(x, R); set(x, L, v) }
        if get(x, T) == e { let v = P - get(x, H) - get(x, B); set(x, T, v) }
    };
    fix(&mut a);
    fix(&mut b);
    // Parent-relative values of line2 are relative to line2's parent: convert to line1's parent.
    let (xdiff, ydiff) = (get(&a, L) + get(&a, R), get(&a, T) + get(&a, B));
    if parent_relative(get(&b, R)) { let v = get(&b, R) - xdiff; set(&mut b, R, v) }
    if parent_relative(get(&b, B)) { let v = get(&b, B) - ydiff; set(&mut b, B, v) }
    if parent_relative(get(&b, L)) { let v = get(&b, L) - xdiff; set(&mut b, L, v) }
    if parent_relative(get(&b, T)) { let v = get(&b, T) - ydiff; set(&mut b, T, v) }
    if parent_relative(get(&a, R)) && parent_relative(get(&b, R)) { let v = get(&a, R) - P; set(&mut a, R, v) }
    if parent_relative(get(&a, B)) && parent_relative(get(&b, B)) { let v = get(&a, B) - P; set(&mut a, B, v) }
    let mut out = blank();
    set(&mut out, L, get(&a, L) + get(&b, L));
    set(&mut out, T, get(&a, T) + get(&b, T));
    set(&mut out, R, get(&a, R) + get(&b, R));
    set(&mut out, B, get(&a, B) + get(&b, B));
    set(&mut out, W, e);
    set(&mut out, H, e);
    set(&mut out, C, get(&b, C));
    out
}

/// A rectangle as `TRect`: top-left and bottom-right (exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x0: x, y0: y, x1: x + w, y1: y + h }
    }
    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> i32 {
        self.y1 - self.y0
    }
}

/// `TAknLayoutRect::LayoutRect(aParent, TAknWindowLineLayout)` = `AknLayoutUtils::RectFromCoords` over the line's l/t/r/b/W/H.
pub fn layout_rect(parent: Rect, w: &Words) -> Rect {
    let Line { l, t, r, b, w: mut ww, h: mut hh, .. } = layout_line(w);
    let (mut l, mut t, mut r, mut b) = (l, t, r, b);
    if l == 0 && t == 0 && r == 0 && b == 0 && ww == 0 && hh == 0 {
        ww = empty();
        hh = empty();
    }
    let (pw, ph) = (parent.width(), parent.height());
    if parent_relative(ww) { ww += pw - P }
    if parent_relative(hh) { hh += ph - P }
    if parent_relative(r) { r += pw - P }
    if parent_relative(b) { b += ph - P }
    if parent_relative(l) { l += pw - P }
    if parent_relative(t) { t += ph - P }
    if l == empty() { l = pw - ww - r } else if ww == empty() { ww = pw - r - l }
    if t == empty() { t = ph - hh - b } else if hh == empty() { hh = ph - b - t }
    Rect::new(parent.x0 + l, parent.y0 + t, ww, hh)
}
