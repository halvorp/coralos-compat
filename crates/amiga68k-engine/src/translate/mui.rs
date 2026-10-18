//! The MUI table (ruling 17, piece 3): a native control's intent and the attribute writes MUI's own classes make for it. MUI has
//! no message beyond the notifications that `set` itself runs, so a row's guest effect IS the list of `set` calls (each a
//! step: one SetAttrs, or a `nnset` that notifies nobody) and no row posts anything else.
//! Facts: Zune area.c (`handle_press`: RelVerify sets `MUIA_Selected` and `MUIA_Pressed` TRUE together, then bumps `MUIA_Timer`;
//! `handle_release`: `MUIA_Pressed` FALSE (with notification), then `MUIA_Selected` FALSE; a release outside only nnsets Pressed;
//! Toggle flips Selected and Pressed together), string.c (Return sets `MUIA_String_Acknowledge` to the contents), cycle.c /
//! radio.c (`MUIA_*_Active`), numeric.c (`MUIA_Numeric_Value`), prop.c (`MUIA_Prop_First`), list.c (`MUIA_List_Active`).
//!
//! | control | intent | steps (writes, in order) |
//! |---|---|---|
//! | button | `Press` | `Selected=1, Pressed=1`; then `Timer+1` |
//! | | `ReleaseInside` | `Pressed=0`; then `Selected=0` (only while Selected) |
//! | | `ReleaseOutside` | nnset `Pressed=0`; then `Selected=0` (only while Selected) |
//! | | `Activate` | the press steps, then the release-inside steps |
//! | toggle | `Activate` | `Selected, Pressed` flip together |
//! | | `SetValue(Checked(b))` | `Selected=b, Pressed=b` |
//! | checkbox | `SetValue(Checked(b))` | `Selected=b, Pressed=b` |
//! | slider | `SetValue(Level(v))`, `Drag(Level(v))` | `Numeric_Value=v` |
//! | choice | `SetValue(Choice(i))` | the class's `Active=i` (`Cycle_Active`, `Radio_Active`, `Group_ActivePage` for tabs) |
//! | field | `CommitText(t)` | `String_Contents=t`; then `String_Acknowledge=t` |
//! | | `Drag(Text(t))` | `String_Contents=t` |
//! | scrollbar | `ScrollTo(n)`, `Drag(Scroll)` | `Prop_First=n` |
//! | list | `SelectRow(i)` | `List_Active=i` |
use super::{row, On, Role};

/// The MUI attributes a row writes, by what they mean; `input.rs` turns each into the class's real attribute number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attr { Selected, Pressed, Timer, Active, Value, Contents, Acknowledge, First, ListActive }

/// What is written: a constant, the intent's value, the intent's text, the opposite of the attribute's current value, or `+1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Val { Zero, One, Intent, Text, Flip, Tick }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Write { pub attr: Attr, pub val: Val }

/// One `set` call: the writes it carries, and whether the notifications run (`false` = MUI's `nnset`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step { pub writes: &'static [Write], pub notify: bool }

/// A row's guest effect: the `set` calls, in order.
pub type Effect = &'static [Step];

/// MUI posts nothing besides the notifications a `set` runs: the type has no value, no row has a post.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Post {}

pub type Row = retro_translate::Row<Effect, Post>;

const fn w(attr: Attr, val: Val) -> Write { Write { attr, val } }
const fn step(writes: &'static [Write], notify: bool) -> Step { Step { writes, notify } }

const PRESS: Step = step(&[w(Attr::Selected, Val::One), w(Attr::Pressed, Val::One)], true);
const TICK: Step = step(&[w(Attr::Timer, Val::Tick)], true);
const RELEASE_PRESSED: Step = step(&[w(Attr::Pressed, Val::Zero)], true);
const RELEASE_CANCEL: Step = step(&[w(Attr::Pressed, Val::Zero)], false);
const DESELECT: Step = step(&[w(Attr::Selected, Val::Zero)], true);
const FLIP: Step = step(&[w(Attr::Selected, Val::Flip), w(Attr::Pressed, Val::Flip)], true);
const SET_BOTH: Step = step(&[w(Attr::Selected, Val::Intent), w(Attr::Pressed, Val::Intent)], true);

pub const TABLE: &[Row] = &[
    row(Role::Button, On::Press, &[PRESS, TICK], None),
    row(Role::Button, On::ReleaseInside, &[RELEASE_PRESSED, DESELECT], None),
    row(Role::Button, On::ReleaseOutside, &[RELEASE_CANCEL, DESELECT], None),
    row(Role::Button, On::Activate, &[PRESS, TICK, RELEASE_PRESSED, DESELECT], None),
    row(Role::Toggle, On::Activate, &[FLIP], None),
    row(Role::Toggle, On::SetValue, &[SET_BOTH], None),
    row(Role::Check, On::SetValue, &[SET_BOTH], None),
    row(Role::Slider, On::SetValue, &[step(&[w(Attr::Value, Val::Intent)], true)], None),
    row(Role::Slider, On::Drag, &[step(&[w(Attr::Value, Val::Intent)], true)], None),
    row(Role::Choice, On::SetValue, &[step(&[w(Attr::Active, Val::Intent)], true)], None),
    row(Role::Field, On::CommitText, &[step(&[w(Attr::Contents, Val::Text)], true), step(&[w(Attr::Acknowledge, Val::Text)], true)], None),
    row(Role::Field, On::Drag, &[step(&[w(Attr::Contents, Val::Text)], true)], None),
    row(Role::Scroll, On::ScrollTo, &[step(&[w(Attr::First, Val::Intent)], true)], None),
    row(Role::Scroll, On::Drag, &[step(&[w(Attr::First, Val::Intent)], true)], None),
    row(Role::List, On::SelectRow, &[step(&[w(Attr::ListActive, Val::Intent)], true)], None),
];

/// The row of the MUI table for this control kind and intent, or the named refusal.
pub fn lookup(kind: &guestui_abi::ControlKind, intent: &guestui_abi::Intent) -> Result<&'static Row, crate::Refusal> {
    retro_translate::lookup(TABLE, kind, intent).map_err(crate::Refusal)
}
