//! The anchor rules: how a bound group's labels (the ROLE, from data) find their geometry in a window's display list (only where
//! the program itself drew that same label). Doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING: a run that matches no label is never a
//! button however it looks, and anything ambiguous is refused by name and left exactly as drawn.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use heirloom_ui_abi::{DrawOp, GuestPoint, GuestRect};

use crate::parse::Label;
use crate::spec::AnchorRule;

pub const REASON_LABEL_TWICE_IN_GROUP: &str = "label twice in the group";
pub const REASON_LABEL_DRAWN_TWICE: &str = "label drawn twice";
pub const REASON_AMBIGUOUS_PREFIX: &str = "ambiguous prefix";
pub const REASON_NO_FILL: &str = "no fill under the label";

/// Apply `rule` to one window's `ops` (window coordinates) for a group's `labels`; each refusal is reported through `on_refuse`.
pub fn apply_anchor(rule: &AnchorRule, labels: &[Label], inset: u32, ops: &mut Vec<DrawOp>, mut on_refuse: impl FnMut(&'static str)) {
    match rule {
        AnchorRule::LabelInFill { prefix_cut, strip_lines_inside, accent_from_swatch } =>
            label_in_fill(*prefix_cut, *strip_lines_inside, *accent_from_swatch, labels, inset, ops, &mut on_refuse),
    }
}

/// How a text run resolves against the candidate labels.
enum Resolve { Label(usize), Ambiguous, None }

fn resolve(run: &str, candidates: &[&str], prefix_cut: bool) -> Resolve {
    if let Some(i) = candidates.iter().position(|c| *c == run) { return Resolve::Label(i); }
    if !prefix_cut || run.is_empty() { return Resolve::None; }
    // The program cuts a label to its cell: a strict prefix of exactly one label is that label.
    let mut hits = candidates.iter().enumerate().filter(|(_, c)| c.len() > run.len() && c.starts_with(run)).map(|(i, _)| i);
    match (hits.next(), hits.next()) {
        (Some(i), None) => Resolve::Label(i),
        (Some(_), Some(_)) => Resolve::Ambiguous,
        _ => Resolve::None,
    }
}

/// `LabelInFill`: a text run naming one label, inside the smallest fill (a theme Fill or a Swatch) that contains it, makes that fill
/// a `Face` over the label's whole cell (the fill grown by the group's `inset` border ring) carrying the label; the run (and, if asked,
/// the program's own edge lines in that cell) go with it.
fn label_in_fill(prefix_cut: bool, strip_lines_inside: bool, accent_from_swatch: bool, labels: &[Label], inset: u32, ops: &mut Vec<DrawOp>, on_refuse: &mut impl FnMut(&'static str)) {
    // The candidates: every label that occurs once in the group. A label twice is refused for both buttons.
    let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
    for l in labels.iter().filter(|l| !l.text.is_empty()) { *seen.entry(l.text.as_str()).or_insert(0) += 1; }
    if seen.values().any(|&n| n > 1) { on_refuse(REASON_LABEL_TWICE_IN_GROUP); }
    let candidates: Vec<&str> = seen.iter().filter(|(_, &n)| n == 1).map(|(s, _)| *s).collect();

    // Each text run -> the label it names (with its box), counting the runs per label.
    let mut runs: Vec<(usize, usize, GuestRect)> = Vec::new(); // (op index, candidate, the run's box)
    let mut per_label: BTreeMap<usize, u32> = BTreeMap::new();
    for (i, op) in ops.iter().enumerate() {
        let DrawOp::Text { at, string, px, width, .. } = op else { continue };
        match resolve(string, &candidates, prefix_cut) {
            Resolve::Label(c) => {
                *per_label.entry(c).or_insert(0) += 1;
                runs.push((i, c, GuestRect { left: at.x, top: at.y, width: (*width).max(1), height: i32::from(*px) }));
            }
            Resolve::Ambiguous => on_refuse(REASON_AMBIGUOUS_PREFIX),
            Resolve::None => {}
        }
    }
    if per_label.values().any(|&n| n > 1) { on_refuse(REASON_LABEL_DRAWN_TWICE); }

    // Plan: for each label drawn exactly once, the smallest fill containing its run.
    let mut faces: BTreeMap<usize, DrawOp> = BTreeMap::new(); // fill op index -> its Face
    let mut drop = alloc::vec![false; ops.len()];
    for &(text_i, c, run) in runs.iter().filter(|(_, c, _)| per_label[c] == 1) {
        let fill = ops.iter().enumerate().filter_map(|(i, op)| match op {
            DrawOp::Fill { rect, .. } => Some((i, *rect, None)),
            DrawOp::Swatch { rect, rgb } => Some((i, *rect, Some(*rgb))),
            _ => None,
        }).filter(|(_, r, _)| r.width > 0 && r.height > 0 && contains(r, &run))
          .min_by_key(|(_, r, _)| i64::from(r.width) * i64::from(r.height));
        let Some((fill_i, fill_rect, rgb)) = fill else { on_refuse(REASON_NO_FILL); continue };
        // The face is the whole cell: the fill plus the format's border ring, where the program draws its own edge lines.
        let k = i32::try_from(inset).unwrap_or(0);
        let rect = GuestRect { left: fill_rect.left - k, top: fill_rect.top - k, width: fill_rect.width + 2 * k, height: fill_rect.height + 2 * k };
        if faces.contains_key(&fill_i) { continue; } // two labels in one fill: the first keeps it, the second stays drawn
        faces.insert(fill_i, DrawOp::Face { rect, label: String::from(candidates[c]), pressed: false, accent: if accent_from_swatch { rgb } else { None } });
        // pressed: always false. delete-when: a binding test carries a recorded frame of a pressed button from a real bound program (no evidence yet of how a pressed
        // button is drawn, so no rule reads it).
        drop[text_i] = true;
        if strip_lines_inside {
            for (i, op) in ops.iter().enumerate() {
                if let DrawOp::Line { a, b, .. } = op { if on_or_in(a, &rect) && on_or_in(b, &rect) { drop[i] = true; } }
            }
        }
    }
    if faces.is_empty() { return; }
    let old = core::mem::take(ops);
    for (i, op) in old.into_iter().enumerate() {
        if let Some(face) = faces.remove(&i) { ops.push(face); } else if !drop[i] { ops.push(op); }
    }
}

/// Does `outer` contain `inner` entirely (half-open pixel rectangles)?
fn contains(outer: &GuestRect, inner: &GuestRect) -> bool {
    let (or, ob) = (i64::from(outer.left) + i64::from(outer.width), i64::from(outer.top) + i64::from(outer.height));
    let (ir, ib) = (i64::from(inner.left) + i64::from(inner.width), i64::from(inner.top) + i64::from(inner.height));
    inner.left >= outer.left && inner.top >= outer.top && ir <= or && ib <= ob
}

/// Is the pixel `p` inside `r` or on its edge (a hairline drawn along the fill's border is the fill's)?
fn on_or_in(p: &GuestPoint, r: &GuestRect) -> bool {
    p.x >= r.left && p.y >= r.top && i64::from(p.x) < i64::from(r.left) + i64::from(r.width) && i64::from(p.y) < i64::from(r.top) + i64::from(r.height)
}
