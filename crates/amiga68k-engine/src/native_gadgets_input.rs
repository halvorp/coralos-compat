//! What the user does to a NATIVE gadget control, mapped back to the guest exactly as Intuition would report it
//! (`GuestUiEvent::Intent`, native_gadgets.rs). The mapping is DATA: `translate::boopsi::TABLE` (role x intent -> state written
//! first, then the IDCMP message gated by the Gadget's Activation bits). This file is that table's adapter: it writes the guest
//! structures a row's [`Effect`] names and posts the message the generic mapper returns. Every intent without a row is a named refusal.
use alloc::format;
use crate::boopsi_ids::*;
use crate::idcmp;
use crate::translate::{self, boopsi, Effect, Sel};
use crate::native_gadgets::{FREEVERT, PI_FLAGS, PI_HBODY, PI_HPOT, PI_TOP, PI_TOTAL, PI_VBODY, PI_VISIBLE, PI_VPOT};
use crate::{machine::Amiga, run::Delivered, Refusal};
use crate::gt_kinds;
use guestui_abi::{CallbackToken, ControlValue, GuestUi, Intent, UiObjectRef};

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
    let row = translate::lookup(boopsi::TABLE, &ng.model.kind, &intent)?;
    let bad = |why: &str| Refusal(format!("deliver: Intent {intent:?} on a {:?} control: {why}", ng.model.kind));
    match (row.effect, &intent) {
        (Effect::None, _) => {}
        (Effect::Select(sel), i) => {
            let on = match (sel, i) {
                (Sel::On, _) => true,
                (Sel::Off, _) => false,
                (Sel::Value, Intent::SetValue(ControlValue::Checked(b))) => *b,
                _ => return Err(bad("a toggle takes a Checked value")),
            };
            set_selected(a, g, on)?;
        }
        (Effect::Scroll, Intent::Drag(ControlValue::Scroll { top, .. }) | Intent::ScrollTo(top)) => scroll_to(a, g, *top)?,
        (Effect::Text, Intent::Drag(ControlValue::Text(t)) | Intent::CommitText(t)) => {
            let si = a.mem.read_u32(g + GAD_SPECIAL).map_err(Refusal::mem("Gadget.SpecialInfo"))?;
            if si == 0 { return Err(bad("the gadget has no StringInfo")); }
            let act = a.mem.read_u16(g + GAD_ACTIVATION).map_err(Refusal::mem("Gadget.Activation"))? as u32;
            crate::boopsi_strg::store_text(a, si, t, act & crate::boopsi_strg::GACT_LONGINT != 0)?;
        }
        _ => return Err(bad("the row's effect does not take this value")),
    }
    // The host's picture already shows what it just did; bring the engine's copy of it in line without a call back.
    crate::native_gadgets::absorb(a, g)?;
    let act = a.mem.read_u16(g + GAD_ACTIVATION).map_err(Refusal::mem("Gadget.Activation"))? as u32;
    match translate::message(row, act, None) {
        Some((class, code)) => idcmp::post(a, win, class, code, g),
        None => Ok(Delivered::Handled),
    }
}
