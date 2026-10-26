//! The ROW-TABLE half of the translation layer (ruling 17, piece 3): a table row says, for one ROLE and one INTENT, which guest
//! effect `E` happens first and which guest message `P` follows. What `E` and `P` are belongs to the toolkit (Intuition's
//! `Post{class, Activation bit, Code}`, a Toolbox part code the original `TrackControl` reports, ...); the role vocabulary, the
//! intent keys and the lookup are the same for every guest, so there is exactly one copy of them. A role / intent pair with no
//! row is a NAMED refusal, never a guess.
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::{ControlKind, Intent};

/// The roles that take intents (`heirloom-ui-abi::ControlKind` seen as what the user does to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Button, Toggle, Check, Scroll, Field, Slider, Choice, List }

/// An intent without its payload: the row key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On { Activate, SetValue, ScrollTo, CommitText, Press, ReleaseInside, ReleaseOutside, Drag, SelectRow }

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
        ControlKind::Choice { .. } | ControlKind::Swatches { .. } => Role::Choice, // a palette is a choice among colours
        ControlKind::List { .. } => Role::List,
        ControlKind::Label { .. } | ControlKind::Group { .. } | ControlKind::Gauge { .. } => return None,
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
        Intent::SelectRow(_) => On::SelectRow,
    })
}

/// The row of `table` for this role and intent, or the named refusal text.
pub fn lookup_role<'t, E, P>(table: &'t [Row<E, P>], role: Role, intent: &Intent) -> Result<&'t Row<E, P>, String> {
    let none = || format!("deliver: Intent {intent:?} on a {role:?} control: this intent means nothing for the control");
    let on = on_of(intent).ok_or_else(none)?;
    table.iter().find(|r| r.role == role && r.on == on).ok_or_else(none)
}

/// The row of `table` for this control kind and intent, or the named refusal text.
pub fn lookup<'t, E, P>(table: &'t [Row<E, P>], kind: &ControlKind, intent: &Intent) -> Result<&'t Row<E, P>, String> {
    match role_of(kind) {
        Some(role) => lookup_role(table, role, intent),
        None => Err(format!("deliver: Intent {intent:?} on a {kind:?} control: this intent means nothing for the control")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: &[Row<(), u8>] = &[row(Role::Button, On::Press, (), Some(7))];
    #[test]
    fn rows_key_on_role_and_intent() {
        assert_eq!(lookup(T, &ControlKind::Button, &Intent::Press).unwrap().post, Some(7));
        assert!(lookup(T, &ControlKind::Button, &Intent::Activate).is_err(), "no row: a named refusal");
        assert!(lookup(T, &ControlKind::Label { text: "x".into(), framed: false }, &Intent::Press).is_err());
        assert!(lookup(T, &ControlKind::Button, &Intent::SelectRow(1)).is_err(), "no row for a row selection on a button");
        assert_eq!(role_of(&ControlKind::Gauge { value: 0, max: 1 }), None, "a gauge takes no intent");
    }
}
