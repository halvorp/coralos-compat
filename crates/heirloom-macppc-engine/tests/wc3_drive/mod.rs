//! The host harness's input driver for the Warcraft III run (gl_triangle wc3_host_run_with_a_gl_worker): a script of pointer and key
//! steps, each fired once the game has swapped a given number of frames, delivered to the window its GL picture is drawn into the way
//! the CoralOS Heirloom.app host delivers them (coral-heirloom-app main.rs AppEvent::Input: one character = Key down + Key up in the same
//! pump, code = the character, no modifiers; a click = Down this slice, Up the next).
//!
//! `WC3_DRIVE="hover 544,114@100; click 544,114@110; keys Peter\r@140"` - steps in order, `@n` = not before n swaps. `keys` text escapes:
//! `\r` Return, `\b` Backspace (0x08), `\e` Escape (0x1B), `\t` Tab, `\\` a backslash. The older `WC3_CLICK=x,y@swap` (+ `WC3_HOVER_SWAPS=n`:
//! a hover at `swap`, the click n swaps later) is the script `hover x,y@swap; click x,y@swap+n`.
use heirloom_ui_abi::{GuestUiEvent, Modifiers, PointerButton, PointerPhase, UiObjectRef};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub enum Act { Hover(i32, i32), Click(i32, i32), Keys(String) }

pub struct Drive {
    steps: VecDeque<(usize, Act)>,
    /// the events of the step in flight, one batch per slice
    queued: VecDeque<Vec<Ev>>,
    pub fired: usize,
}

#[derive(Debug, Clone)]
enum Ev { Ptr(i32, i32, PointerButton, PointerPhase, u8), Key(char, bool) }

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' { out.push(c); continue }
        match it.next() {
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('e') => out.push('\u{1b}'),
            Some('t') => out.push('\t'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn xy(s: &str) -> Option<(i32, i32)> { let (x, y) = s.trim().split_once(',')?; Some((x.trim().parse().ok()?, y.trim().parse().ok()?)) }

impl Drive {
    pub fn parse(script: &str) -> Result<Self, String> {
        let mut steps = VecDeque::new();
        for part in script.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (body, at) = part.rsplit_once('@').ok_or_else(|| format!("step `{part}` has no @swaps"))?;
            let at: usize = at.trim().parse().map_err(|_| format!("step `{part}`: bad swap count"))?;
            let (verb, arg) = body.split_once(' ').ok_or_else(|| format!("step `{part}` has no argument"))?;
            let act = match verb {
                "hover" => xy(arg).map(|(x, y)| Act::Hover(x, y)),
                "click" => xy(arg).map(|(x, y)| Act::Click(x, y)),
                "keys" => Some(Act::Keys(unescape(arg))),
                _ => None,
            }.ok_or_else(|| format!("step `{part}`: unknown verb or bad argument"))?;
            steps.push_back((at, act));
        }
        Ok(Drive { steps, queued: VecDeque::new(), fired: 0 })
    }

    /// The script from `WC3_DRIVE`, else the older `WC3_CLICK` / `WC3_HOVER_SWAPS`; None = no input.
    pub fn from_env() -> Option<Self> {
        if let Ok(s) = std::env::var("WC3_DRIVE") { return Some(Self::parse(&s).expect("WC3_DRIVE")) }
        let c = std::env::var("WC3_CLICK").ok()?;
        let (pt, at) = c.split_once('@')?;
        let at: usize = at.parse().ok()?;
        let hover: usize = std::env::var("WC3_HOVER_SWAPS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let s = if hover > 0 { format!("hover {pt}@{at}; click {pt}@{}", at + hover) } else { format!("click {pt}@{at}") };
        Some(Self::parse(&s).expect("WC3_CLICK"))
    }

    pub fn done(&self) -> bool { self.steps.is_empty() && self.queued.is_empty() }

    /// This slice's events for window `win` after `swaps` frames (empty: nothing due). Each step's events start only after the last step's
    /// are all out.
    pub fn due(&mut self, swaps: usize, win: UiObjectRef, log: &mut dyn FnMut(String)) -> Vec<GuestUiEvent> {
        if self.queued.is_empty() {
            if let Some((at, _)) = self.steps.front() {
                if swaps >= *at {
                    let (_, act) = self.steps.pop_front().unwrap();
                    log(format!("DRIVE step {} {act:?} at swap {swaps} in native window {}", self.fired, win.id));
                    self.fired += 1;
                    match act {
                        Act::Hover(x, y) => self.queued.push_back(vec![Ev::Ptr(x, y, PointerButton::None, PointerPhase::Move, 0)]),
                        Act::Click(x, y) => {
                            self.queued.push_back(vec![Ev::Ptr(x, y, PointerButton::Primary, PointerPhase::Down, 1)]);
                            self.queued.push_back(vec![Ev::Ptr(x, y, PointerButton::Primary, PointerPhase::Up, 0)]);
                        }
                        Act::Keys(t) => for c in t.chars() { self.queued.push_back(vec![Ev::Key(c, true), Ev::Key(c, false)]) },
                    }
                }
            }
        }
        self.queued.pop_front().unwrap_or_default().into_iter().map(|e| match e {
            Ev::Ptr(x, y, button, phase, clicks) => GuestUiEvent::Pointer { win, x, y, button, phase, clicks },
            Ev::Key(c, down) => GuestUiEvent::Key { win, code: c as u32, text: c.to_string(), modifiers: Modifiers::default(), down },
        }).collect()
    }
}
