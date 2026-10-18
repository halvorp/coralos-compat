//! A small deterministic layout for a MUI tree: Group (vertical, horizontal or N columns; every kind is one grid) and
//! Text cells. Sizes are in guest pixels with an 8x8 font. This only decides the window size and where each control
//! sits; the native UI draws them.
use alloc::string::String;
use alloc::vec::Vec;
use super::ids::*;
use crate::boopsi::{Ext, Kind, Objects};
use guestui_abi::GuestRect;

pub const MARGIN: i32 = 4;
const GAP: i32 = 4;
const CHAR_W: i32 = 8;
const CELL_H: i32 = 14;

/// One control to create: the Text object, where, and its Label's text when a Label sibling precedes it.
pub struct Placed { pub obj: u32, pub rect: GuestRect, pub label: String }

fn is_label(objs: &Objects, o: u32) -> bool { matches!(objs.map.get(&o).map(|x| &x.ext), Some(Ext::Text { label: true, .. })) }
fn is_text(objs: &Objects, o: u32) -> bool { objs.map.get(&o).is_some_and(|x| x.kind == Kind::Text) }
fn contents(objs: &Objects, o: u32) -> String { objs.map.get(&o).and_then(|x| x.strs.get(&MUIA_TEXT_CONTENTS).cloned()).unwrap_or_default() }

/// Column widths and row heights of group `g`.
fn grid(objs: &Objects, g: u32) -> (usize, Vec<i32>, Vec<i32>) {
    let Some(o) = objs.map.get(&g) else { return (1, Vec::new(), Vec::new()) };
    let n = o.children.len();
    let cols = match o.attrs.get(&MUIA_GROUP_COLUMNS).copied().unwrap_or(0) as usize {
        0 if o.attrs.get(&MUIA_GROUP_HORIZ).copied().unwrap_or(0) != 0 => n.max(1),
        0 => 1,
        c => c,
    };
    let rows = n.div_ceil(cols);
    let (mut cw, mut rh) = (alloc::vec![0; cols.min(n.max(1))], alloc::vec![0; rows]);
    for (i, &c) in o.children.iter().enumerate() {
        let (w, h) = measure(objs, c);
        let col = i % cols;
        if col < cw.len() { cw[col] = cw[col].max(w); }
        rh[i / cols] = rh[i / cols].max(h);
    }
    (cols, cw, rh)
}

fn total(v: &[i32]) -> i32 { v.iter().sum::<i32>() + GAP * (v.len() as i32 - 1).max(0) }

fn measure(objs: &Objects, o: u32) -> (i32, i32) {
    match objs.map.get(&o).map(|x| x.kind) {
        Some(Kind::Text) => (CHAR_W * (contents(objs, o).chars().count() as i32 + 1), CELL_H),
        Some(Kind::Group) => { let (_, cw, rh) = grid(objs, o); (total(&cw), total(&rh)) }
        _ => (0, 0),
    }
}

/// (inner width, inner height, controls) for the tree under `root`, margins included.
pub fn compute(objs: &Objects, root: u32) -> (i32, i32, Vec<Placed>) {
    let (w, h) = measure(objs, root);
    let mut out = Vec::new();
    place(objs, root, MARGIN, MARGIN, &mut out, &mut None);
    (w + 2 * MARGIN, h + 2 * MARGIN, out)
}

fn place(objs: &Objects, o: u32, x: i32, y: i32, out: &mut Vec<Placed>, label: &mut Option<String>) {
    match objs.map.get(&o).map(|k| k.kind) {
        Some(Kind::Text) => {
            let (w, h) = measure(objs, o);
            out.push(Placed { obj: o, rect: GuestRect { left: x, top: y, width: w, height: h }, label: label.take().unwrap_or_default() });
        }
        Some(Kind::Group) => {
            let (cols, cw, rh) = grid(objs, o);
            let kids = objs.map.get(&o).map(|g| g.children.clone()).unwrap_or_default();
            let mut pending: Option<String> = None;
            for (i, &c) in kids.iter().enumerate() {
                let cx = x + cw[..i % cols].iter().map(|w| w + GAP).sum::<i32>();
                let cy = y + rh[..i / cols].iter().map(|h| h + GAP).sum::<i32>();
                // A Label directly before a Text is that Text's label, not a control of its own.
                if is_label(objs, c) && kids.get(i + 1).is_some_and(|&n| is_text(objs, n) && !is_label(objs, n)) { pending = Some(contents(objs, c)); continue; }
                let (w, h) = (cw[i % cols], rh[i / cols]);
                if let Some(Kind::Text) = objs.map.get(&c).map(|k| k.kind) {
                    out.push(Placed { obj: c, rect: GuestRect { left: cx, top: cy, width: w, height: h }, label: pending.take().unwrap_or_default() });
                } else { place(objs, c, cx, cy, out, &mut pending); }
            }
        }
        _ => {}
    }
}
