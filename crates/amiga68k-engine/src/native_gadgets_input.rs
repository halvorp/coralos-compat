//! What the user does to a NATIVE gadget control, mapped back to the guest exactly as Intuition would report it
//! (`GuestUiEvent::Intent`, native_gadgets.rs). The guest-visible state changes FIRST, whether or not the window listens:
//!
//! | control | intent | guest state | IDCMP message (IAddress = the Gadget) |
//! |---|---|---|---|
//! | button / arrow / hotspot | `Press` | GFLG_SELECTED set | GADGETDOWN if GACT_IMMEDIATE |
//! | | `Release{inside:true}` | GFLG_SELECTED cleared | GADGETUP if GACT_RELVERIFY |
//! | | `Release{inside:false}` | GFLG_SELECTED cleared | none (released outside) |
//! | toggle | `Press` | none | GADGETDOWN if GACT_IMMEDIATE |
//! | | `SetValue(Checked(b))` | GFLG_SELECTED = b | GADGETUP if GACT_RELVERIFY |
//! | scrollbar | `Press` | none | GADGETDOWN if GACT_IMMEDIATE |
//! | | `Drag(Scroll)` | pot/body (and PGA_Top) follow `top` | MOUSEMOVE if GACT_FOLLOWMOUSE |
//! | | `ScrollTo(top)` | same | GADGETUP if GACT_RELVERIFY |
//! | field | `Press` | none | GADGETDOWN if GACT_IMMEDIATE |
//! | | `Drag(Text)` | buffer, NumChars, cursor, LongInt follow the text | none |
//! | | `CommitText` = Return | same | GADGETUP (Code 0x0D, the key that ended the edit) if GACT_RELVERIFY |
//!
//! Facts: AROS rom/intuition strgadgets.c (IntuiMessage.Code of an ended edit = the mapped key, 0x0D for Return), inputhandler
//! gadget code (GADGETDOWN needs GACT_IMMEDIATE, GADGETUP needs GACT_RELVERIFY, MOUSEMOVE needs GACT_FOLLOWMOUSE and the window's
//! IDCMP_MOUSEMOVE). Every other intent is a named refusal.
use alloc::format;
use crate::boopsi_ids::*;
use crate::idcmp::{self, IDCMP_GADGETDOWN, IDCMP_GADGETUP, IDCMP_MOUSEMOVE};
use crate::native_gadgets::{FREEVERT, PI_FLAGS, PI_HBODY, PI_HPOT, PI_TOP, PI_TOTAL, PI_VBODY, PI_VISIBLE, PI_VPOT};
use crate::{machine::Amiga, run::Delivered, Refusal};
use crate::gt_kinds;
use guestui_abi::{CallbackToken, ControlKind, ControlValue, GuestUi, Intent, UiObjectRef};

const RETURN_KEY: u16 = 0x0D;

fn set_selected<U: GuestUi>(a: &mut Amiga<U>, g: u32, on: bool) -> Result<(), Refusal> {
    let f = a.mem.read_u16(g + GAD_FLAGS).map_err(Refusal::mem("Gadget.Flags"))? as u32;
    let f = if on { f | GFLG_SELECTED } else { f & !GFLG_SELECTED };
    a.mem.write_u16(g + GAD_FLAGS, f as u16).map_err(Refusal::mem("Gadget.Flags"))
}

/// The scrollbar moved to `top`: PropInfo follows (a propgclass gadget also keeps PGA_Top).
fn scroll_to<U: GuestUi>(a: &mut Amiga<U>, g: u32, top: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("PropInfo");
    let pi = a.mem.read_u32(g + GAD_SPECIAL).map_err(&m)?;
    if pi == 0 { return Err(Refusal(format!("deliver: scrollbar for gadget {g:#x}, which has no PropInfo"))); }
    let vert = a.mem.read_u16(pi + PI_FLAGS).map_err(&m)? as u32 & FREEVERT != 0;
    let (pot_at, body_at) = if vert { (PI_VPOT, PI_VBODY) } else { (PI_HPOT, PI_HBODY) };
    let body = a.mem.read_u16(pi + body_at).map_err(&m)? as u32;
    let propg = crate::native_gadgets::is_propgclass(a, g) && a.mem.read_u32(pi + PI_TOTAL).map_err(&m)? != 0;
    let (pot, body) = if propg {
        let (total, visible) = (a.mem.read_u32(pi + PI_TOTAL).map_err(&m)?, a.mem.read_u32(pi + PI_VISIBLE).map_err(&m)?);
        let top = top.min(total.saturating_sub(visible));
        a.mem.write_u32(pi + PI_TOP, top).map_err(&m)?;
        crate::boopsi_prop::pot_body(total, visible, top)
    } else {
        // pots and bodies on the 0xFFFF scale native_gadgets::scroll_of reads: top = pot * (0xFFFF - body) / 0xFFFF
        let room = 0xFFFF - body.min(0xFFFF);
        let pot = if room == 0 { 0 } else { (top.min(room) as u64 * 0xFFFF / room as u64) as u32 };
        (pot, body)
    };
    a.mem.write_u16(pi + pot_at, pot as u16).map_err(&m)?;
    a.mem.write_u16(pi + body_at, body as u16).map_err(&m)
}

/// A user intent on a control (idcmp::deliver's arm for `GuestUiEvent::Intent`): the token says whether it is a native gadget
/// (here) or a GadTools gadget (`gt_kinds`). A `control` / `generation` that is not the live control of the token is refused by name.
pub fn intent<U: GuestUi>(a: &mut Amiga<U>, win: u32, native: UiObjectRef, control: UiObjectRef, token: CallbackToken, generation: u32, intent: Intent) -> Result<Delivered, Refusal> {
    if generation != control.generation {
        return Err(Refusal(format!("deliver: Intent for token {:#x} carries generation {generation}, but its control ref has {} (stale)", token.0, control.generation)));
    }
    let native_gadget = u32::try_from(token.0).is_ok_and(|g| a.native_gads.map.contains_key(&g));
    if native_gadget { driven(a, win, native, control, token, intent) } else { gt_kinds::control_changed(a, win, native, control, token, intent) }
}

fn driven<U: GuestUi>(a: &mut Amiga<U>, win: u32, native: UiObjectRef, control: UiObjectRef, token: CallbackToken, intent: Intent) -> Result<Delivered, Refusal> {
    let g = u32::try_from(token.0).ok().filter(|g| a.native_gads.map.contains_key(g))
        .ok_or_else(|| Refusal(format!("deliver: Intent token {:#x} is not a native gadget", token.0)))?;
    let ng = a.native_gads.map[&g].clone();
    if (ng.win, ng.native, ng.ctl) != (win, native, control) {
        return Err(Refusal(format!("deliver: Intent for gadget {g:#x}, which is not that native control of that window (stale?)")));
    }
    let act = a.mem.read_u16(g + GAD_ACTIVATION).map_err(Refusal::mem("Gadget.Activation"))? as u32;
    let bad = |why: &str| Refusal(format!("deliver: Intent {intent:?} on a {:?} control: {why}", ng.model.kind));
    let mut post = None; // (IDCMP class, Code)
    match (&ng.model.kind, &intent) {
        (ControlKind::Button | ControlKind::ArrowButton { .. } | ControlKind::Hotspot, Intent::Press) => {
            set_selected(a, g, true)?;
            if act & GACT_IMMEDIATE != 0 { post = Some((IDCMP_GADGETDOWN, 0)); }
        }
        (ControlKind::Button | ControlKind::ArrowButton { .. } | ControlKind::Hotspot, Intent::Release { inside: true }) => {
            set_selected(a, g, false)?;
            if act & GACT_RELVERIFY != 0 { post = Some((IDCMP_GADGETUP, 0)); }
        }
        (ControlKind::Button | ControlKind::ArrowButton { .. } | ControlKind::Hotspot | ControlKind::ToggleButton { .. }, Intent::Release { inside: false }) => {
            if !matches!(ng.model.kind, ControlKind::ToggleButton { .. }) { set_selected(a, g, false)?; }
        }
        (ControlKind::ToggleButton { .. } | ControlKind::ScrollBar { .. } | ControlKind::Field { .. }, Intent::Press) => {
            if act & GACT_IMMEDIATE != 0 { post = Some((IDCMP_GADGETDOWN, 0)); }
        }
        (ControlKind::ToggleButton { .. }, Intent::SetValue(ControlValue::Checked(on))) => {
            set_selected(a, g, *on)?;
            if act & GACT_RELVERIFY != 0 { post = Some((IDCMP_GADGETUP, 0)); }
        }
        (ControlKind::ScrollBar { .. }, Intent::Drag(ControlValue::Scroll { top, .. })) => {
            scroll_to(a, g, *top)?;
            if act & GACT_FOLLOWMOUSE != 0 { post = Some((IDCMP_MOUSEMOVE, 0)); }
        }
        (ControlKind::ScrollBar { .. }, Intent::ScrollTo(top)) => {
            scroll_to(a, g, *top)?;
            if act & GACT_RELVERIFY != 0 { post = Some((IDCMP_GADGETUP, 0)); }
        }
        (ControlKind::Field { .. }, Intent::Drag(ControlValue::Text(t)) | Intent::CommitText(t)) => {
            let si = a.mem.read_u32(g + GAD_SPECIAL).map_err(Refusal::mem("Gadget.SpecialInfo"))?;
            if si == 0 { return Err(bad("the gadget has no StringInfo")); }
            crate::boopsi_strg::store_text(a, si, t, act & crate::boopsi_strg::GACT_LONGINT != 0)?;
            if matches!(intent, Intent::CommitText(_)) && act & GACT_RELVERIFY != 0 { post = Some((IDCMP_GADGETUP, RETURN_KEY)); }
        }
        _ => return Err(bad("this intent means nothing for the control")),
    }
    // The host's picture already shows what it just did; bring the engine's copy of it in line without a call back.
    crate::native_gadgets::absorb(a, g)?;
    match post {
        Some((class, code)) => idcmp::post(a, win, class, code, g),
        None => Ok(Delivered::Handled),
    }
}
