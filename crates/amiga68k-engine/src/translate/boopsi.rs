//! The Intuition / BOOPSI system-class table: a native gadget control's intents and the guest messages they become.
//! Facts: AROS rom/intuition inputhandler gadget code and strgadgets.c (GADGETDOWN needs GACT_IMMEDIATE, GADGETUP needs
//! GACT_RELVERIFY, MOUSEMOVE needs GACT_FOLLOWMOUSE and the window's IDCMP_MOUSEMOVE; the Code of an ended string edit is the
//! mapped key, 0x0D for Return). The guest-visible state changes FIRST, whether or not the window listens.
//!
//! | control | intent | guest state | IDCMP message (IAddress = the Gadget) |
//! |---|---|---|---|
//! | button / arrow / hotspot | `Press` | GFLG_SELECTED set | GADGETDOWN if GACT_IMMEDIATE |
//! | | `Release{inside:true}` | GFLG_SELECTED cleared | GADGETUP if GACT_RELVERIFY |
//! | | `Release{inside:false}` | GFLG_SELECTED cleared | none |
//! | toggle | `Press` | none | GADGETDOWN if GACT_IMMEDIATE |
//! | | `SetValue(Checked(b))` | GFLG_SELECTED = b | GADGETUP if GACT_RELVERIFY |
//! | scrollbar | `Press` | none | GADGETDOWN if GACT_IMMEDIATE |
//! | | `Drag(Scroll)` | pot/body (PGA_Top) follow `top` | MOUSEMOVE if GACT_FOLLOWMOUSE |
//! | | `ScrollTo(top)` | same | GADGETUP if GACT_RELVERIFY |
//! | field | `Press` | none | GADGETDOWN if GACT_IMMEDIATE |
//! | | `Drag(Text)` | buffer, NumChars, cursor, LongInt | none |
//! | | `CommitText` | same | GADGETUP, Code 0x0D, if GACT_RELVERIFY |
use super::{post, row, Code, Effect, On, Role, Row, Sel};
use crate::boopsi_ids::{GACT_FOLLOWMOUSE, GACT_IMMEDIATE, GACT_RELVERIFY};
use crate::idcmp::{IDCMP_GADGETDOWN, IDCMP_GADGETUP, IDCMP_MOUSEMOVE};

const DOWN: Option<super::Post> = post(IDCMP_GADGETDOWN, GACT_IMMEDIATE, Code::Zero);
const UP: Option<super::Post> = post(IDCMP_GADGETUP, GACT_RELVERIFY, Code::Zero);

pub const TABLE: &[Row] = &[
    row(Role::Button, On::Press, Effect::Select(Sel::On), DOWN),
    row(Role::Button, On::ReleaseInside, Effect::Select(Sel::Off), UP),
    row(Role::Button, On::ReleaseOutside, Effect::Select(Sel::Off), None),
    row(Role::Toggle, On::Press, Effect::None, DOWN),
    row(Role::Toggle, On::SetValue, Effect::Select(Sel::Value), UP),
    row(Role::Toggle, On::ReleaseOutside, Effect::None, None),
    row(Role::Scroll, On::Press, Effect::None, DOWN),
    row(Role::Scroll, On::Drag, Effect::Scroll, post(IDCMP_MOUSEMOVE, GACT_FOLLOWMOUSE, Code::Zero)),
    row(Role::Scroll, On::ScrollTo, Effect::Scroll, UP),
    row(Role::Field, On::Press, Effect::None, DOWN),
    row(Role::Field, On::Drag, Effect::Text, None),
    row(Role::Field, On::CommitText, Effect::Text, post(IDCMP_GADGETUP, GACT_RELVERIFY, Code::Return)),
];
