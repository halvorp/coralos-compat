//! The GadTools table. GadTools filters its gadgets' raw Intuition messages in GT_GetIMsg (a slider's Code becomes its level,
//! a checkbox's its state); the message built here already carries the filtered values, so for the kinds bridged GT_GetIMsg is
//! exactly GetMsg. Every GadTools gadget is GACT_RELVERIFY by construction, so its GADGETUP is unconditional.
//!
//! | gadget | intent | guest state | IDCMP message (IAddress = the Gadget) |
//! |---|---|---|---|
//! | BUTTON_KIND | `Activate` | none | GADGETUP, Code 0 |
//! | CHECKBOX_KIND | `SetValue(Checked(b))` | GFLG_SELECTED = b | GADGETUP, Code b |
//! | SLIDER_KIND | `SetValue(Level(l))` | the level | GADGETUP, Code l |
//! | STRING_KIND / INTEGER_KIND | `CommitText(t)` | StringInfo buffer (+ LongInt) | GADGETUP, Code 0x0D (strgclass: the ending key) |
//! | | `SetValue(Text(t))` | same | GADGETUP, Code 0 |
//!
//! SLIDER_KIND is filtered like AROS `GT_FilterIMsg` (gt_filterimsg.c, GADGETUP case): a release that CHANGED the level reaches
//! the program as IDCMP_MOUSEMOVE (Code = the level), one that did not stays GADGETUP ([`filter_class`]).
use super::{post, row, Code, Effect, On, Role, Row};
use crate::idcmp::{IDCMP_GADGETUP, IDCMP_MOUSEMOVE};

const UP: Option<super::Post> = post(IDCMP_GADGETUP, 0, Code::Zero);
const UP_VALUE: Option<super::Post> = post(IDCMP_GADGETUP, 0, Code::Value);

pub const TABLE: &[Row] = &[
    row(Role::Button, On::Activate, Effect::None, UP),
    row(Role::Check, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::Slider, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::Field, On::CommitText, Effect::Store, post(IDCMP_GADGETUP, 0, Code::Return)),
    row(Role::Field, On::SetValue, Effect::Store, UP),
];

/// GT_FilterIMsg's one class rewrite among the kinds bridged: a slider release whose level changed is a MOUSEMOVE.
pub fn filter_class(class: u32, kind: u32, level_changed: bool) -> u32 {
    if kind == crate::gt_kinds::SLIDER_KIND && class == IDCMP_GADGETUP && level_changed { IDCMP_MOUSEMOVE } else { class }
}
