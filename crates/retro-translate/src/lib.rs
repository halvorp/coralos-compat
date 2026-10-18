//! The toolkit-NEUTRAL half of the translation layer (ruling 17, piece 3), shared by amiga68k, mac68k and macppc: a table row
//! says, for one ROLE and one INTENT, which guest effect `E` happens first and which guest message `P` follows. What `E` and
//! `P` are belongs to the toolkit (Intuition's `Post{class, Activation bit, Code}`, a Toolbox mouseDown + part code, ...); the
//! role vocabulary, the intent keys, the lookup and the single stale-intent check are the same for every guest, so there is
//! exactly one copy of them. A role / intent pair with no row is a NAMED refusal, never a guess.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::format;
use alloc::string::String;
use guestui_abi::{ControlKind, Intent, UiObjectRef, CallbackToken};

/// The roles that take intents (`guestui-abi::ControlKind` seen as what the user does to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Button, Toggle, Check, Scroll, Field, Slider }

/// An intent without its payload: the row key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On { Activate, SetValue, ScrollTo, CommitText, Press, ReleaseInside, ReleaseOutside, Drag }

/// One table row: guest effect `E` first, then the optional guest message `P`.
#[derive(Debug, Clone, Copy)]
pub struct Row<E, P> { pub role: Role, pub on: On, pub effect: E, pub post: Option<P> }

/// Row constructor, short enough for a table to read as one line per row.
pub const fn row<E, P>(role: Role, on: On, effect: E, post: Option<P>) -> Row<E, P> { Row { role, on, effect, post } }

/// The role of a control kind; `None` = a kind that takes no intent (labels, groups).
pub fn role_of(kind: &ControlKind) -> Option<Role> {
    Some(match kind {
        ControlKind::Button | ControlKind::ArrowButton { .. } | ControlKind::Hotspot => Role::Button,
        ControlKind::ToggleButton { .. } => Role::Toggle,
        ControlKind::Checkbox { .. } => Role::Check,
        ControlKind::ScrollBar { .. } => Role::Scroll,
        ControlKind::Field { .. } | ControlKind::String { .. } => Role::Field,
        ControlKind::Slider { .. } => Role::Slider,
        ControlKind::Label { .. } | ControlKind::Group { .. } => return None,
    })
}

pub fn on_of(i: &Intent) -> Option<On> {
    Some(match i {
        Intent::Activate => On::Activate,
        Intent::SetValue(_) => On::SetValue,
        Intent::ScrollTo(_) => On::ScrollTo,
        Intent::CommitText(_) => On::CommitText,
        Intent::Press => On::Press,
        Intent::Release { inside: true } => On::ReleaseInside,
        Intent::Release { inside: false } => On::ReleaseOutside,
        Intent::Drag(_) => On::Drag,
        Intent::SelectRow(_) => return None,
    })
}

/// The row of `table` for this control kind and intent, or the named refusal text.
pub fn lookup<'t, E, P>(table: &'t [Row<E, P>], kind: &ControlKind, intent: &Intent) -> Result<&'t Row<E, P>, String> {
    let none = || format!("deliver: Intent {intent:?} on a {kind:?} control: this intent means nothing for the control");
    let (role, on) = (role_of(kind).ok_or_else(none)?, on_of(intent).ok_or_else(none)?);
    table.iter().find(|r| r.role == role && r.on == on).ok_or_else(none)
}

/// The ONE stale-intent check. An intent names a control by ref + token + generation; it is delivered only when the ref's own
/// generation matches the one the intent carries AND `live` (the engine's record of what that token is: guest window,
/// host window, host control) is exactly the window and control the intent names. `None` = the token is not a live control.
pub fn check_live<W: PartialEq>(win: W, native: UiObjectRef, control: UiObjectRef, token: CallbackToken, generation: u32, live: Option<(W, UiObjectRef, UiObjectRef)>) -> Result<(), String> {
    if generation != control.generation {
        return Err(format!("deliver: Intent for token {:#x} carries generation {generation}, but its control ref has {} (stale)", token.0, control.generation));
    }
    match live {
        Some(l) if l == (win, native, control) => Ok(()),
        Some(_) => Err(format!("deliver: Intent for token {:#x}, which is not that native control of that window (stale)", token.0)),
        None => Err(format!("deliver: Intent token {:#x} is not a live control (retired, or never shown)", token.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: &[Row<(), u8>] = &[row(Role::Button, On::Press, (), Some(7))];
    fn r(id: u32, generation: u32) -> UiObjectRef { UiObjectRef { id, generation } }
    #[test]
    fn rows_key_on_role_and_intent() {
        assert_eq!(lookup(T, &ControlKind::Button, &Intent::Press).unwrap().post, Some(7));
        assert!(lookup(T, &ControlKind::Button, &Intent::Activate).is_err(), "no row: a named refusal");
        assert!(lookup(T, &ControlKind::Label { text: "x".into(), framed: false }, &Intent::Press).is_err());
        assert!(lookup(T, &ControlKind::Button, &Intent::SelectRow(1)).is_err());
    }
    #[test]
    fn a_stale_intent_is_refused_by_name() {
        let (w, c, t) = (r(1, 1), r(5, 1), CallbackToken(9));
        assert!(check_live(3u32, w, c, t, 1, Some((3, w, c))).is_ok());
        assert!(check_live(3u32, w, c, t, 2, Some((3, w, c))).unwrap_err().contains("stale"), "forged generation");
        assert!(check_live(3u32, w, r(6, 1), t, 1, Some((3, w, c))).unwrap_err().contains("not that native control"), "another control");
        assert!(check_live(4u32, w, c, t, 1, Some((3, w, c))).is_err(), "another window");
        assert!(check_live(3u32, w, c, t, 1, None).unwrap_err().contains("not a live control"));
    }
}
