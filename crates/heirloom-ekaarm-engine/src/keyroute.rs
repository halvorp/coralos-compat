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
use crate::wsevent::{EEVENT_KEY, EEVENT_KEY_DOWN, EEVENT_KEY_UP};

/// `CCoeAppUi::HandleWsEventL` (coeaui.h:175): slot 4 (`appstart.rs` lists the numbering; the FileBrowser AppUi's vtable holds its own
/// code there, which calls the base class through an import).
pub const SLOT_HANDLE_WS_EVENT_L: u32 = 4;
/// The base-class implementations a guest AppUi may inherit in slot 4: the engine's natives below.
pub const INHERITED_HANDLE_WS_EVENT_L: &[&str] = HANDLE_WS_EVENT_NAMES;
/// `CCoeAppUi::HandleKeyEventL` (coeaui.h:180): slot 7.
const SLOT_HANDLE_KEY_EVENT_L: u32 = 7;
const INHERITED_HANDLE_KEY_EVENT_L: &[&str] = &["_ZN9CCoeAppUi14HandleKeyEventLERK9TKeyEvent10TEventCode"];
/// `CCoeControl::OfferKeyEventL` (slot 3, control.rs) and the overrides of it this engine implements.
const SLOT_OFFER_KEY_EVENT_L: u32 = 3;
const INHERITED_OFFER_KEY_EVENT_L: &[&str] = &["_ZN11CCoeControl14OfferKeyEventLERK9TKeyEvent10TEventCode", crate::listkey::LIST_OFFER_KEY_EVENT_L];
/// `TWsEvent::iEventData` (wsevent.rs): the `TKeyEvent` of a key event.
const TWS_I_EVENT_DATA: u32 = 16;
/// `TKeyResponse` (w32std.h): EKeyWasNotConsumed 0, EKeyWasConsumed 1.
const EKEY_WAS_CONSUMED: u32 = 1;

const HANDLE_WS_EVENT_NAMES: &[&str] = &[
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
}

/// Ask the next control, or, when none is left, the AppUi's `HandleKeyEventL`.
fn offer_next(p: &mut Process, mut o: Offer) -> Result<(), Outcome> {
    if o.rest.is_empty() {
        let spec = SlotSpec { class: "CCoeAppUi", method: "HandleKeyEventL", slot: SLOT_HANDLE_KEY_EVENT_L, inherited: INHERITED_HANDLE_KEY_EVENT_L };
        let target = anchor(p, o.appui, &spec).map_err(Outcome::Fault)?;
        return p.call_guest(target, &[o.appui, o.key, o.ty], Continuation::Resume).map_err(Outcome::Fault);
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

/// `HandleWsEventL(const TWsEvent& aEvent, CCoeControl* aDestination)` of the AppUi's base classes: this = r0, the event = r1.
fn handle_ws_event(p: &mut Process) -> Result<(), String> {
    let (appui, event) = (p.cpu.regs[0], p.cpu.regs[1]);
    let ty = p.rd(event)?;
    if !matches!(ty, EEVENT_KEY | EEVENT_KEY_UP | EEVENT_KEY_DOWN) {
        return Err(format!("window server event type {ty}: only key events are routed"));
    }
    let rest = p.stack_key_targets(appui)?;
    p.log.push(format!("[key] event type {ty} offered to {} stacked control(s)", rest.len()));
    offer_next(p, Offer { appui, key: event + TWS_I_EVENT_DATA, ty, rest }).map_err(|o| match o { Outcome::Fault(m) | Outcome::Refused(m) => m, other => format!("{other:?}") })
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if !HANDLE_WS_EVENT_NAMES.contains(&name) { return None; }
    Some(match handle_ws_event(p) {
        Ok(()) => NativeOutcome::Return,
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
    })
}
