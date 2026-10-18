//! The Amiga's half of the translation layer (ruling 17, piece 3): a role intent from the host becomes the exact guest-visible
//! sequence AmigaOS would have produced, by a TABLE per toolkit layer and one generic mapper.
//!
//! * [`boopsi`] — Intuition gadget types and BOOPSI system classes (what `native_gadgets` instantiated as controls).
//! * [`gadtools`] — GadTools gadget kinds.
//!
//! A table row says, for one ROLE and one INTENT, what guest state is written first ([`Effect`]) and which IntuiMessage then
//! follows ([`Post`]: class, the Gadget.Activation bit that must be set for Intuition to send it, and the Code). The mapper
//! ([`lookup`], [`message`]) knows nothing about a toolkit or an application. The procedural part of a row (writing a
//! PropInfo, a StringInfo, a GadTools kind's value) is the toolkit adapter's `apply`, next to its table: rows stay data, the
//! table never becomes a programming language. A role / intent pair with no row is a named refusal, never a guess.
pub mod boopsi;
pub mod gadtools;

use alloc::format;
use guestui_abi::{ControlKind, ControlValue, Intent};
use crate::Refusal;

/// The roles that take intents (the closed vocabulary, `guestui-abi::ControlKind` seen as what the user does to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Button, Toggle, Check, Scroll, Field, Slider }

/// An intent without its payload: the row key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum On { Activate, SetValue, ScrollTo, CommitText, Press, ReleaseInside, ReleaseOutside, Drag }

/// The guest state written first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing changes (a press that only announces itself).
    None,
    /// GFLG_SELECTED follows [`Sel`].
    Select(Sel),
    /// The scroll position (PropInfo pots/bodies, PGA_Top).
    Scroll,
    /// The text of an edit (StringInfo buffer, NumChars, cursor, LongInt).
    Text,
    /// The value of a GadTools gadget (checkbox state, slider level, string buffer).
    Store,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel { On, Off, Value }

/// What the message's Code is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code { Zero, Return, Value }

/// The IntuiMessage that follows the state: `class`, sent only when `Gadget.Activation & needs != 0` (0 = always).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Post { pub class: u32, pub needs: u32, pub code: Code }

#[derive(Debug, Clone, Copy)]
pub struct Row { pub role: Role, pub on: On, pub effect: Effect, pub post: Option<Post> }

/// Row constructor, short enough for a table to read as one line per row.
pub const fn row(role: Role, on: On, effect: Effect, post: Option<Post>) -> Row { Row { role, on, effect, post } }
pub const fn post(class: u32, needs: u32, code: Code) -> Option<Post> { Some(Post { class, needs, code }) }

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

fn on_of(i: &Intent) -> Option<On> {
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

/// The row of `table` for this control kind and intent, or the named refusal.
pub fn lookup(table: &'static [Row], kind: &ControlKind, intent: &Intent) -> Result<&'static Row, Refusal> {
    let none = || Refusal(format!("deliver: Intent {intent:?} on a {kind:?} control: this intent means nothing for the control"));
    let (role, on) = (role_of(kind).ok_or_else(none)?, on_of(intent).ok_or_else(none)?);
    table.iter().find(|r| r.role == role && r.on == on).ok_or_else(none)
}

/// The message a row sends given the gadget's Activation word, as `(class, code)`; `value` is the checked state / level the
/// intent carried (for [`Code::Value`]). `None` = Intuition would not send one.
pub fn message(r: &Row, activation: u32, value: Option<&ControlValue>) -> Option<(u32, u16)> {
    let p = r.post?;
    if p.needs != 0 && activation & p.needs == 0 { return None; }
    let code = match p.code {
        Code::Zero => 0,
        Code::Return => 0x0D,
        Code::Value => match value { Some(ControlValue::Checked(b)) => *b as u16, Some(ControlValue::Level(l)) => *l as u16, _ => 0 },
    };
    Some((p.class, code))
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: &[Row] = &[row(Role::Button, On::Press, Effect::None, post(0x20, 2, Code::Zero))];
    #[test]
    fn rows_key_on_role_and_intent_and_the_activation_bit_gates_the_message() {
        let r = lookup(T, &ControlKind::Button, &Intent::Press).unwrap();
        assert_eq!(message(r, 2, None), Some((0x20, 0)));
        assert_eq!(message(r, 1, None), None, "no GACT_IMMEDIATE: Intuition sends nothing");
        assert!(lookup(T, &ControlKind::Button, &Intent::Activate).is_err(), "no row: a named refusal");
        assert!(lookup(T, &ControlKind::Label { text: "x".into(), framed: false }, &Intent::Press).is_err());
        assert!(lookup(T, &ControlKind::Button, &Intent::SelectRow(1)).is_err());
    }
}
