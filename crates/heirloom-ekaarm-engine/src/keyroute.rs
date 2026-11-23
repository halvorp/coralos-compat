//! The environment's delivery path for a key event (slice 4): `CCoeEnv::RunL` -> `iAppUi->HandleWsEventL(event, window)` (`wsevent.rs`) ->
//! for the three key event types (EEventKey, EEventKeyUp, EEventKeyDown) the AppUi offers the key to the controls on its control stack,
//! topmost first (`ctrlstack.rs`), each through the control's own virtual `OfferKeyEventL` (CCoeControl slot 3, confirmed by the
//! FileBrowser container's vtable holding guest code there); the first to answer `EKeyWasConsumed` ends it, and a key nobody consumed
//! goes to the AppUi's `HandleKeyEventL(const TKeyEvent&, TEventCode)` (coeaui.h:180, slot 7 after `HandleForegroundEventL` 8's
//! neighbours, see `appstart.rs`: HandleWsEventL 4 ... HandleKeyEventL 7). The Avkon layer (AKNAPPUI.CPP:1837-2075) puts key sounds, key code
//! aliasing and the help key before this; none of them changes an arrow or selection key's route, and none is modelled.
//! DELETE WHEN the AppUi's key alias conversion (`GetAliasKeyCodeL`) or the help key is reached by a corpus app.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::appstart::{anchor, SlotSpec};
use crate::callback::Continuation;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::run::Outcome;
use crate::wsevent::{EEVENT_FOCUS_GAINED, EEVENT_FOCUS_LOST, EEVENT_KEY, EEVENT_KEY_DOWN, EEVENT_KEY_UP};

/// `CCoeAppUi::HandleForegroundEventL(TBool)` (coeaui.h): slot 8.
const SLOT_HANDLE_FOREGROUND_EVENT_L: u32 = 8;
const INHERITED_HANDLE_FOREGROUND_EVENT_L: &[&str] = &[
    "_ZN9CCoeAppUi22HandleForegroundEventLEi",
    "_ZN9CEikAppUi22HandleForegroundEventLEi",
    "_ZN13CAknAppUiBase22HandleForegroundEventLEi",
    "_ZN9CAknAppUi22HandleForegroundEventLEi",
    "_ZN13CAknViewAppUi22HandleForegroundEventLEi",
];

/// `CCoeAppUi::HandleWsEventL` (coeaui.h:175): slot 4 (`appstart.rs` lists the numbering; the FileBrowser AppUi's vtable holds its own
/// code there, which calls the base class through an import).
pub const SLOT_HANDLE_WS_EVENT_L: u32 = 4;
/// The base-class implementations a guest AppUi may inherit in slot 4: the engine's natives below.
pub const INHERITED_HANDLE_WS_EVENT_L: &[&str] = HANDLE_WS_EVENT_NAMES;
/// `CCoeAppUi::HandleKeyEventL` (coeaui.h:180): slot 7.
const SLOT_HANDLE_KEY_EVENT_L: u32 = 7;
const INHERITED_HANDLE_KEY_EVENT_L: &[&str] = &[
    "_ZN9CCoeAppUi15HandleKeyEventLERK9TKeyEvent10TEventCode",
    "_ZN9CCoeAppUi14HandleKeyEventLERK9TKeyEvent10TEventCode",
];
/// `CCoeControl::OfferKeyEventL` (slot 3, control.rs) and the overrides of it this engine implements.
const SLOT_OFFER_KEY_EVENT_L: u32 = 3;
const INHERITED_OFFER_KEY_EVENT_L: &[&str] = &["_ZN11CCoeControl14OfferKeyEventLERK9TKeyEvent10TEventCode", crate::listkey::LIST_OFFER_KEY_EVENT_L];
/// `TWsEvent::iEventData` (wsevent.rs): the `TKeyEvent` of a key event.
const TWS_I_EVENT_DATA: u32 = 16;
/// `TKeyResponse` (w32std.h): EKeyWasNotConsumed 0, EKeyWasConsumed 1.
const EKEY_WAS_CONSUMED: u32 = 1;
/// S60 Avkon `EKeyEnter` -> `EKeyOK` (`EKeyDevice3`) alias constants (e32keys.h).
const EKEY_ENTER: u32 = 0x000d;
const EKEY_DEVICE3: u32 = 0xf845;
const ESTD_KEY_DEVICE3: u32 = 0xa7;

const HANDLE_WS_EVENT_NAMES: &[&str] = &[
    "_ZN9CCoeAppUi14HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN9CEikAppUi14HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN13CAknAppUiBase14HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN9CAknAppUi14HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN9CCoeAppUi13HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN9CEikAppUi13HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN13CAknAppUiBase13HandleWsEventLERK8TWsEventP11CCoeControl",
    "_ZN9CAknAppUi13HandleWsEventLERK8TWsEventP11CCoeControl",
    // FileBrowser's own HandleWsEventL calls this one (avkon/368).
    "_ZN13CAknViewAppUi14HandleWsEventLERK8TWsEventP11CCoeControl",
];

/// A key event being offered: the controls not yet asked (the next is the first), the key and its event type.
#[derive(Debug, Clone)]
pub struct Offer {
    appui: u32,
    key: u32,
    ty: u32,
    rest: Vec<u32>,
    /// If true and the AppUi does not consume `EKeyEnter`, retry once as S60 `EKeyDevice3` (`EKeyOK`).
    alias_enter_to_ok: bool,
}

/// Ask the next control, or, when none is left, the AppUi's `HandleKeyEventL`.
fn offer_next(p: &mut Process, mut o: Offer) -> Result<(), Outcome> {
    if o.rest.is_empty() {
        let spec = SlotSpec { class: "CCoeAppUi", method: "HandleKeyEventL", slot: SLOT_HANDLE_KEY_EVENT_L, inherited: INHERITED_HANDLE_KEY_EVENT_L };
        let target = anchor(p, o.appui, &spec).map_err(Outcome::Fault)?;
        let cont = if o.alias_enter_to_ok { Continuation::KeyAppUi(o.clone()) } else { Continuation::Resume };
        return p.call_guest(target, &[o.appui, o.key, o.ty], cont).map_err(Outcome::Fault);
    }
    let control = o.rest.remove(0);
    let spec = SlotSpec { class: "CCoeControl", method: "OfferKeyEventL", slot: SLOT_OFFER_KEY_EVENT_L, inherited: INHERITED_OFFER_KEY_EVENT_L };
    let target = anchor(p, control, &spec).map_err(Outcome::Fault)?;
    let (key, ty) = (o.key, o.ty);
    p.call_guest(target, &[control, key, ty], Continuation::KeyOffer(o)).map_err(Outcome::Fault)
}

/// A control's `OfferKeyEventL` returned `r0`.
pub(crate) fn offered(p: &mut Process, o: Offer, r0: u32) -> Result<(), Outcome> {
    if r0 == EKEY_WAS_CONSUMED {
        return Ok(());
    }
    offer_next(p, o)
}

/// `CCoeAppUi::HandleKeyEventL` returned `r0`; if `EKeyEnter` was not consumed, retry once as S60 `EKeyDevice3` (`EKeyOK`).
pub(crate) fn appui_handled(p: &mut Process, mut o: Offer, r0: u32) -> Result<(), Outcome> {
    if r0 == EKEY_WAS_CONSUMED || !o.alias_enter_to_ok {
        return Ok(());
    }
    // Rewrite `TKeyEvent.iCode` and `iScanCode` to S60 center-select (`EKeyOK` / `EStdKeyDevice3`) and re-offer.
    p.wr(o.key, EKEY_DEVICE3).map_err(Outcome::Fault)?;
    p.wr(o.key + 4, ESTD_KEY_DEVICE3).map_err(Outcome::Fault)?;
    o.rest = p.stack_key_targets(o.appui).map_err(Outcome::Fault)?;
    o.alias_enter_to_ok = false;
    offer_next(p, o)
}

/// `HandleWsEventL(const TWsEvent& aEvent, CCoeControl* aDestination)` of the AppUi's base classes: this = r0, the event = r1.
fn handle_ws_event(p: &mut Process) -> Result<(), String> {
    let (appui, event) = (p.cpu.regs[0], p.cpu.regs[1]);
    let ty = p.rd(event)?;
    if matches!(ty, EEVENT_FOCUS_GAINED | EEVENT_FOCUS_LOST) {
        let fg = u32::from(ty == EEVENT_FOCUS_GAINED);
        let spec = SlotSpec {
            class: "CCoeAppUi",
            method: "HandleForegroundEventL",
            slot: SLOT_HANDLE_FOREGROUND_EVENT_L,
            inherited: INHERITED_HANDLE_FOREGROUND_EVENT_L,
        };
        if let Ok(target) = anchor(p, appui, &spec) {
            return p.call_guest(target, &[appui, fg], Continuation::Resume);
        }
        return Ok(());
    }
    if !matches!(ty, EEVENT_KEY | EEVENT_KEY_UP | EEVENT_KEY_DOWN) {
        return Err(format!("window server event type {ty}: only key and focus events are routed"));
    }
    let key = event + TWS_I_EVENT_DATA;
    let alias_enter_to_ok = ty == EEVENT_KEY && p.rd(key).unwrap_or(0) == EKEY_ENTER;
    let rest = p.stack_key_targets(appui)?;
    p.log.push(format!("[key] event type {ty} offered to {} stacked control(s)", rest.len()));
    offer_next(p, Offer { appui, key, ty, rest, alias_enter_to_ok }).map_err(|o| match o { Outcome::Fault(m) | Outcome::Refused(m) => m, other => format!("{other:?}") })
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if matches!(
        name,
        "_ZN11CCoeControl14OfferKeyEventLERK9TKeyEvent10TEventCode"
            | "_ZN9CCoeAppUi15HandleKeyEventLERK9TKeyEvent10TEventCode"
            | "_ZN9CCoeAppUi14HandleKeyEventLERK9TKeyEvent10TEventCode"
    ) {
        // Base `CCoeControl::OfferKeyEventL` (coecntrl.cpp) and `CCoeAppUi::HandleKeyEventL`
        // (coeaui.cpp) both do nothing and return `EKeyWasNotConsumed` (0).
        p.cpu.regs[0] = 0;
        return Some(NativeOutcome::Return);
    }
    if INHERITED_HANDLE_FOREGROUND_EVENT_L.contains(&name) {
        return Some(NativeOutcome::Return);
    }
    if !HANDLE_WS_EVENT_NAMES.contains(&name) { return None; }
    Some(match handle_ws_event(p) {
        Ok(()) => NativeOutcome::Return,
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
    })
}
