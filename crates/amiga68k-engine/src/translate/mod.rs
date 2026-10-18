//! The Amiga's half of the translation layer (ruling 17, piece 3): a role intent from the host becomes the exact guest-visible
//! sequence AmigaOS would have produced, by a TABLE per toolkit layer and ONE generic mapper (`retro-translate`, shared with the Mac engines).
//!
//! * [`boopsi`] — Intuition gadget types and BOOPSI system classes (what `native_gadgets` instantiated as controls).
//! * [`gadtools`] — GadTools gadget kinds.
//! * [`mui`] — MUI controls (a row is the `set` calls MUI's own classes make).
//!
//! A table row says, for one ROLE and one INTENT, what guest state is written first ([`Effect`]) and which IntuiMessage then
//! follows ([`Post`]: class, the Gadget.Activation bit that must be set for Intuition to send it, and the Code). Roles, intent
//! keys, [`lookup`] and the stale-intent check are `retro-translate`'s; only the Amiga payloads (`Effect`, `Post`, [`message`])
//! live here. The procedural part of a row (writing a
//! PropInfo, a StringInfo, a GadTools kind's value) is the toolkit adapter's `apply`, next to its table: rows stay data, the
//! table never becomes a programming language. A role / intent pair with no row is a named refusal, never a guess.
pub mod boopsi;
pub mod gadtools;
pub mod mui;

use guestui_abi::ControlValue;
use crate::Refusal;
pub use retro_translate::{row, On, Role};

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

/// An Amiga table row: the generic row with Intuition's payloads.
pub type Row = retro_translate::Row<Effect, Post>;

pub const fn post(class: u32, needs: u32, code: Code) -> Option<Post> { Some(Post { class, needs, code }) }

/// The row of `table` for this control kind and intent, or the named refusal.
pub fn lookup(table: &'static [Row], kind: &guestui_abi::ControlKind, intent: &guestui_abi::Intent) -> Result<&'static Row, Refusal> {
    retro_translate::lookup(table, kind, intent).map_err(Refusal)
}

/// The message a row sends given the gadget's Activation word, as `(class, code)`; `value` is the checked state / level the
/// intent carried (for [`Code::Value`]). `None` = Intuition would not send one.
pub fn message(r: &Row, activation: u32, value: Option<&ControlValue>) -> Option<(u32, u16)> {
    let p = r.post?;
    if p.needs != 0 && activation & p.needs == 0 { return None; }
    let code = match p.code {
        Code::Zero => 0,
        Code::Return => 0x0D,
        Code::Value => match value {
            Some(ControlValue::Checked(b)) => *b as u16,
            Some(ControlValue::Level(l)) => *l as u16,
            // GadTools lists and choices: the chosen entry / row, a scroller's new Top.
            Some(ControlValue::Choice(i)) => *i as u16,
            Some(ControlValue::Rows { selected: Some(r), .. }) => *r as u16,
            Some(ControlValue::Scroll { top, .. }) => *top as u16,
            _ => 0,
        },
    };
    Some((p.class, code))
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: &[Row] = &[row(Role::Button, On::Press, Effect::None, post(0x20, 2, Code::Zero))];
    #[test]
    fn rows_key_on_role_and_intent_and_the_activation_bit_gates_the_message() {
        let r = lookup(T, &guestui_abi::ControlKind::Button, &guestui_abi::Intent::Press).unwrap();
        assert_eq!(message(r, 2, None), Some((0x20, 0)));
        assert_eq!(message(r, 1, None), None, "no GACT_IMMEDIATE: Intuition sends nothing");
        assert!(lookup(T, &guestui_abi::ControlKind::Button, &guestui_abi::Intent::Activate).is_err(), "no row: a named refusal");
        assert!(lookup(T, &guestui_abi::ControlKind::Label { text: "x".into(), framed: false }, &guestui_abi::Intent::Press).is_err());
    }
}
