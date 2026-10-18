//! The GadTools table. GadTools filters its gadgets' raw Intuition messages in GT_GetIMsg (a slider's Code becomes its level,
//! a checkbox's its state); the message built here already carries the filtered values, so for the kinds bridged GT_GetIMsg is
//! exactly GetMsg. Every GadTools gadget is GACT_RELVERIFY by construction, so its GADGETUP is unconditional.
//!
//! | gadget | intent | guest state | IDCMP message (IAddress = the Gadget) |
//! |---|---|---|---|
//! | BUTTON_KIND | `Activate` | none | GADGETUP, Code 0 |
//! | CHECKBOX_KIND | `SetValue(Checked(b))` | GFLG_SELECTED = b | GADGETUP, Code b |
//! | SLIDER_KIND | `SetValue(Level(l))` | the level | GADGETUP, Code l |
//! | STRING_KIND / INTEGER_KIND | `CommitText(t)` / `SetValue(Text(t))` | StringInfo buffer (+ LongInt) | GADGETUP, Code 0 |
use super::{post, row, Code, Effect, On, Role, Row};
use crate::idcmp::IDCMP_GADGETUP;

const UP: Option<super::Post> = post(IDCMP_GADGETUP, 0, Code::Zero);
const UP_VALUE: Option<super::Post> = post(IDCMP_GADGETUP, 0, Code::Value);

pub const TABLE: &[Row] = &[
    row(Role::Button, On::Activate, Effect::None, UP),
    row(Role::Check, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::Slider, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::Field, On::CommitText, Effect::Store, UP),
    row(Role::Field, On::SetValue, Effect::Store, UP),
];
