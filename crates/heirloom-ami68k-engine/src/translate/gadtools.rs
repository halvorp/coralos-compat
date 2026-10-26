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
//! | CYCLE_KIND | `SetValue(Choice(i))` | the active entry | GADGETUP, Code i (cycleclass.c: GMR_VERIFY, Code = active) |
//! | MX_KIND | `SetValue(Choice(i))` | the active entry | GADGETDOWN, Code i (mxclass.c: GACT_IMMEDIATE, no RELVERIFY) |
//! | LISTVIEW_KIND | `SelectRow(r)` | the selected row | GADGETUP, Code r |
//! | | `ScrollTo(t)` | the top row | none (listview scrolling is private to the class: gt_filterimsg.c hides its scroller) |
//! | SCROLLER_KIND | `ScrollTo(t)` | the top | MOUSEMOVE if the top changed else GADGETUP, Code t (gt_filterimsg.c) |
//!
//! SLIDER_KIND and SCROLLER_KIND are filtered like AROS `GT_FilterIMsg` (gt_filterimsg.c, GADGETUP case): a release that CHANGED the level reaches
//! the program as IDCMP_MOUSEMOVE (Code = the level), one that did not stays GADGETUP ([`filter_class`]).
use super::{post, row, Code, Effect, On, Role, Row};
use crate::gt_kinds::{MX_KIND, SCROLLER_KIND, SLIDER_KIND};
use crate::idcmp::{IDCMP_GADGETDOWN, IDCMP_GADGETUP, IDCMP_MOUSEMOVE};

const UP: Option<super::Post> = post(IDCMP_GADGETUP, 0, Code::Zero);
const UP_VALUE: Option<super::Post> = post(IDCMP_GADGETUP, 0, Code::Value);

pub const TABLE: &[Row] = &[
    row(Role::Button, On::Activate, Effect::None, UP),
    row(Role::Check, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::Slider, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::Field, On::CommitText, Effect::Store, post(IDCMP_GADGETUP, 0, Code::Return)),
    row(Role::Field, On::SetValue, Effect::Store, UP),
    row(Role::Choice, On::SetValue, Effect::Store, UP_VALUE),
    row(Role::List, On::SelectRow, Effect::Store, UP_VALUE),
    row(Role::List, On::ScrollTo, Effect::Store, None),
    row(Role::Scroll, On::ScrollTo, Effect::Store, UP_VALUE),
];

/// MX_KIND shares the Choice role with CYCLE_KIND but is GACT_IMMEDIATE only, so its message is GADGETDOWN.
pub const MX_TABLE: &[Row] = &[row(Role::Choice, On::SetValue, Effect::Store, post(IDCMP_GADGETDOWN, 0, Code::Value))];

/// The table a gadget kind's rows live in.
pub fn table_for(kind: u32) -> &'static [Row] { if kind == MX_KIND { MX_TABLE } else { TABLE } }

/// GT_FilterIMsg's one class rewrite among the kinds bridged: a slider or scroller release whose value changed is a MOUSEMOVE.
pub fn filter_class(class: u32, kind: u32, level_changed: bool) -> u32 {
    if matches!(kind, SLIDER_KIND | SCROLLER_KIND) && class == IDCMP_GADGETUP && level_changed { IDCMP_MOUSEMOVE } else { class }
}
