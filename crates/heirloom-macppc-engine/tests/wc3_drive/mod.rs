//! The host harness's input driver for the Warcraft III run (gl_triangle wc3_host_run_with_a_gl_worker): a script of pointer and key
//! steps, each fired once the game has swapped a given number of frames, delivered to the window its GL picture is drawn into the way
//! the CoralOS Heirloom.app host delivers them (coral-heirloom-app host_keys.rs, h86: a key's down at once and its release HOLD_MS =
//! 120 ms later = [`HOLD_SLICES`] slices of 20 ms; code = the scalar the compositor sent, text = what it types (empty for a key that
//! types nothing), Shift with a character typed shifted; a click = Down this slice, Up the next).
//!
//! `WC3_DRIVE="hover 544,114@100; click 544,114@110; keys Peter\r@140; keys {Up*50}@900"` - steps in order, `@n` = not before n swaps.
//! `keys` text escapes: `\r` Return, `\b` Backspace (0x08), `\e` Escape (0x1B), `\t` Tab, `\\` a backslash, `\{` a brace. A NAMED key
//! is `{Name}` - Up Down Left Right Home End PgUp PgDn Del Ins F1..F12 - with an optional `Shift+` / `Ctrl+` / `Alt+` prefix and an
//! optional `*n` hold in slices (`{Up*50}`: held for one second - what a held key will be once the key wire carries releases; Heirloom
//! holds a press for HOLD_SLICES). The older `WC3_CLICK=x,y@swap` (+ `WC3_HOVER_SWAPS=n`: a hover at `swap`, the click n swaps later) is
//! the script `hover x,y@swap; click x,y@swap+n`.
use heirloom_ui_abi::{host_key, GuestUiEvent, Modifiers, PointerButton, PointerPhase, UiObjectRef};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub enum Act { Hover(i32, i32), Click(i32, i32), Keys(Vec<Stroke>) }

/// One key press: the host code, what it types, the modifiers held, and how many slices it is held.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke { code: u32, text: String, mods: u8, hold: usize }

/// The host's hold ([`host_key::HOLD_MS`]) in the harness's 20 ms slices.
pub const HOLD_SLICES: usize = (host_key::HOLD_MS / 20) as usize;

pub struct Drive {
    steps: VecDeque<(usize, Act)>,
    /// the events of the step in flight, one batch per slice
    queued: VecDeque<Vec<Ev>>,
    pub fired: usize,
}

#[derive(Debug, Clone)]
enum Ev { Ptr(i32, i32, PointerButton, PointerPhase, u8), Key(u32, String, u8, bool) }

/// A character as Heirloom.app delivers it: itself, with Shift when the installed layout types it shifted (the compositor's KeyMods).
fn typed(c: char) -> Stroke {
    let shifted = u8::try_from(c as u32).ok().and_then(heirloom_mac68k_engine::keyboard::virtual_key)
        .is_some_and(|(_, m)| m & heirloom_mac68k_engine::keyboard::SHIFT_KEY != 0);
    Stroke { code: c as u32, text: c.to_string(), mods: if shifted { Modifiers::SHIFT } else { 0 }, hold: HOLD_SLICES }
}

/// `{[Shift+|Ctrl+|Alt+]Name[*n]}` without its braces.
fn named(t: &str) -> Option<Stroke> {
    let (mut mods, mut rest) = (0u8, t);
    loop {
        let Some((m, r)) = [("Shift+", Modifiers::SHIFT), ("Ctrl+", Modifiers::CTRL), ("Alt+", Modifiers::ALT)].iter()
            .find_map(|(p, m)| rest.strip_prefix(p).map(|r| (*m, r))) else { break };
        mods |= m;
        rest = r;
    }
    let (name, hold) = match rest.split_once('*') { Some((n, h)) => (n, h.parse().ok()?), None => (rest, HOLD_SLICES) };
    let code = match name {
        "Up" => host_key::UP, "Down" => host_key::DOWN, "Left" => host_key::LEFT, "Right" => host_key::RIGHT,
        "Home" => host_key::HOME, "End" => host_key::END, "PgUp" => host_key::PAGE_UP, "PgDn" => host_key::PAGE_DOWN,
        "Del" => host_key::DELETE, "Ins" => host_key::INSERT,
        f => host_key::F1 + u32::from(f.strip_prefix('F')?.parse::<u8>().ok().filter(|n| (1..=12).contains(n))?) - 1,
    };
    Some(Stroke { code, text: String::new(), mods, hold })
}

fn strokes(s: &str) -> Option<Vec<Stroke>> {
    let mut out = Vec::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(named(&it.by_ref().take_while(|c| *c != '}').collect::<String>())?),
            '\\' => out.push(typed(match it.next() {
                Some('r') => '\r',
                Some('b') => '\u{8}',
                Some('e') => '\u{1b}',
                Some('t') => '\t',
                Some(o) => o,
                None => '\\',
            })),
            c => out.push(typed(c)),
        }
    }
    Some(out)
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
                "keys" => strokes(arg).map(Act::Keys),
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
                        Act::Keys(t) => for k in t {
                            self.queued.push_back(vec![Ev::Key(k.code, k.text.clone(), k.mods, true)]);
                            self.queued.extend((1..k.hold).map(|_| Vec::new()));
                            self.queued.push_back(vec![Ev::Key(k.code, k.text, k.mods, false)]);
                        },
                    }
                }
            }
        }
        self.queued.pop_front().unwrap_or_default().into_iter().map(|e| match e {
            Ev::Ptr(x, y, button, phase, clicks) => GuestUiEvent::Pointer { win, x, y, button, phase, clicks },
            Ev::Key(code, text, m, down) => GuestUiEvent::Key { win, code, text, modifiers: Modifiers(m), down },
        }).collect()
    }
}

#[test]
fn the_drive_script_types_characters_and_names_keys_with_holds_and_modifiers() {
    let Act::Keys(k) = Drive::parse("keys Pe\\r{Up}{Shift+Left*50}{F2}@1").unwrap().steps[0].1.clone() else { panic!() };
    let s = |code: u32, text: &str, mods: u8, hold: usize| Stroke { code, text: text.into(), mods, hold };
    assert_eq!(k, [
        s('P' as u32, "P", Modifiers::SHIFT, HOLD_SLICES), s('e' as u32, "e", 0, HOLD_SLICES), s(0x0D, "\r", 0, HOLD_SLICES),
        s(host_key::UP, "", 0, HOLD_SLICES), s(host_key::LEFT, "", Modifiers::SHIFT, 50), s(host_key::F1 + 1, "", 0, HOLD_SLICES),
    ]);
    // a held key is its down, then nothing for the hold, then its release
    let mut d = Drive::parse("keys {Up*3}@0").unwrap();
    let win = UiObjectRef { id: 1, generation: 1 };
    let batches: Vec<usize> = (0..5).map(|_| d.due(0, win, &mut |_| {}).len()).collect();
    assert_eq!(batches, [1, 0, 0, 1, 0]);
    // NEGATIVE CONTROL: an unknown key name is a script error
    assert!(Drive::parse("keys {Hyper}@1").is_err() && Drive::parse("keys {F13}@1").is_err());
}
