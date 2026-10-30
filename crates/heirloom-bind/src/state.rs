//! The interpreter's state across an engine's run: the binding specs it was given, the groups parsed from files the program read,
//! which open window each group is anchored to, and every refusal, counted by reason and said once.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use heirloom_ui_abi::DrawOp;

use crate::anchor::apply_anchor;
use crate::parse::{parse, recognise, Group, Placement, Recognised};
use crate::spec::BindingSpec;

pub const REASON_LEGACY_SCHEMA: &str = "legacy schema";
pub const REASON_MALFORMED: &str = "malformed";
pub const REASON_AMBIGUOUS_WINDOW: &str = "ambiguous window";

#[derive(Clone, Debug)]
struct Loaded { spec: usize, group: Group, source: String }

/// The one binding interpreter's state (an engine keeps one and feeds it from its generic hooks).
#[derive(Default, Debug)]
pub struct Bindings {
    specs: Vec<BindingSpec>,
    groups: Vec<Loaded>,
    /// Placement records, per spec: the newest record for a path replaces an older one.
    placements: Vec<(usize, Placement)>,
    /// open window -> index into `groups`.
    windows: BTreeMap<u32, usize>,
    refused: BTreeMap<&'static str, u32>,
    said: BTreeSet<(usize, &'static str)>,
    log: Vec<String>,
}

impl Bindings {
    /// Add one binding spec (the engine converts each binding file it found into one).
    pub fn add_spec(&mut self, spec: BindingSpec) { self.specs.push(spec); }

    /// The program read the file `path` whose bytes are `bytes`. A file of no known form costs one header comparison per spec.
    pub fn on_file(&mut self, path: &str, bytes: &[u8]) {
        for spec in 0..self.specs.len() {
            match recognise(&self.specs[spec], bytes) {
                Recognised::NotThisForm => continue,
                Recognised::Legacy => { self.refuse(spec, REASON_LEGACY_SCHEMA); return; }
                Recognised::Ours => {
                    match parse(&self.specs[spec], bytes) {
                        Ok(parsed) => {
                            for group in parsed.groups {
                                // A re-read of the same file: the newer parse replaces the group with the same source, title and box,
                                // keeping its window anchor; anything else is a new group.
                                match self.groups.iter().position(|g| g.spec == spec && g.source == path && g.group.title == group.title && g.group.window_box == group.window_box) {
                                    Some(i) => self.groups[i].group = group,
                                    None => self.groups.push(Loaded { spec, group, source: String::from(path) }),
                                }
                            }
                            for pl in parsed.placements {
                                match self.placements.iter().position(|(s, p)| *s == spec && p.path.eq_ignore_ascii_case(&pl.path)) {
                                    Some(i) => self.placements[i].1 = pl,
                                    None => self.placements.push((spec, pl)),
                                }
                            }
                        }
                        Err(_) => { self.refuse(spec, REASON_MALFORMED); self.log.push(format!("[HEIRLOOM-BINDING {}] {path}: {REASON_MALFORMED}", self.specs[spec].id)); }
                    }
                    return;
                }
            }
        }
    }

    /// Window `win` opened with `title` (`None`: the program gave it none) at `box_` (left, top, width, height). It binds to the one
    /// group it anchors to by data; two groups for one window, or a group already anchored to another open window, binds neither.
    pub fn on_window(&mut self, win: u32, title: Option<&str>, box_: (i32, i32, i32, i32)) {
        let hits: Vec<usize> = (0..self.groups.len()).filter(|&g| {
            let lg = &self.groups[g];
            anchors(&lg.group, title, box_) || self.placements.iter().any(|(s, p)| *s == lg.spec && placed(p, &lg.source, box_))
        }).collect();
        match hits[..] {
            [] => {}
            [g] => {
                let spec = self.groups[g].spec;
                if let Some(other) = self.windows.iter().find(|(_, &j)| j == g).map(|(&w, _)| w) {
                    self.windows.remove(&other);
                    self.refuse(spec, REASON_AMBIGUOUS_WINDOW);
                } else {
                    self.windows.insert(win, g);
                    self.log.push(format!("[HEIRLOOM-BINDING {}] bound window {win:#x} ({} labels)", self.specs[spec].id, self.groups[g].group.labels.len()));
                }
            }
            _ => { let spec = self.groups[hits[0]].spec; self.refuse(spec, REASON_AMBIGUOUS_WINDOW); }
        }
    }

    /// Window `win` closed: its anchor goes with it.
    pub fn on_close(&mut self, win: u32) { self.windows.remove(&win); }

    /// Rewrite a bound window's display list (window coordinates) before it is published; an unbound window is untouched.
    pub fn bind_window(&mut self, win: u32, ops: &mut Vec<DrawOp>) {
        let Some(&g) = self.windows.get(&win) else { return };
        let spec = self.groups[g].spec;
        let mut why: Vec<&'static str> = Vec::new();
        apply_anchor(&self.specs[spec].anchor, &self.groups[g].group.labels, self.groups[g].group.inset, ops, |r| why.push(r));
        for r in why { self.refuse(spec, r); }
    }

    /// Every refusal so far, by reason.
    pub fn refusals(&self) -> &BTreeMap<&'static str, u32> { &self.refused }

    /// The windows currently bound (for an engine's census).
    pub fn bound_windows(&self) -> impl Iterator<Item = u32> + '_ { self.windows.keys().copied() }

    /// The log lines since the last call (one per distinct reason per spec, one per anchoring).
    pub fn take_log(&mut self) -> Vec<String> { core::mem::take(&mut self.log) }

    fn refuse(&mut self, spec: usize, why: &'static str) {
        *self.refused.entry(why).or_insert(0) += 1;
        if self.said.insert((spec, why)) { self.log.push(format!("[HEIRLOOM-BINDING {}] {why}", self.specs[spec].id)); }
    }
}

/// Does placement record `p` put the group read from `source` in a window at `box_`? Its path names that file (AmigaDOS-style
/// names compare without case) and its left and top are the window's: the program fits the size to its font, never the position.
fn placed(p: &Placement, source: &str, (l, t, _, _): (i32, i32, i32, i32)) -> bool {
    p.path.eq_ignore_ascii_case(source) && (i32::from(p.window_box.0), i32::from(p.window_box.1)) == (l, t)
}

/// Does `group` anchor to a window with this title and box? By its title (both present and equal) or its exact box (a box with no
/// positive size never matches: a program writes -1 or 0 for "as large as it can be").
fn anchors(group: &Group, title: Option<&str>, (l, t, w, h): (i32, i32, i32, i32)) -> bool {
    let by_title = matches!((group.title.as_deref(), title), (Some(a), Some(b)) if !a.is_empty() && a == b);
    let by_box = matches!(group.window_box, Some((gl, gt, gw, gh)) if gw > 0 && gh > 0 && (i32::from(gl), i32::from(gt), i32::from(gw), i32::from(gh)) == (l, t, w, h));
    by_title || by_box
}
