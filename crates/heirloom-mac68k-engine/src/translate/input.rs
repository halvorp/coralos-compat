//! Plain input for content (ruling 17, piece 8): the pointer and the keyboard, as the events a classic Mac program already
//! handles, so a content plane is fully usable before it has any binding. Coordinates: the host sends window coordinates
//! (0,0 = the window's content top-left); the Event Manager reports GLOBAL ones, so the window's content origin is added.
//!
//! * `Pointer` primary `Down`/`Up` -> `mouseDown`/`mouseUp`; the secondary button is a control-click (`controlKey` in the
//!   modifiers, the Mac convention for the contextual click); the middle button has no classic meaning: refused by name.
//!   Motion produces no event (a Mac program polls `GetMouse`/`Button`): it only updates the low-memory `Mouse` point.
//! * `Key` down -> `keyDown`, the same code again before its release -> `autoKey`; release produces nothing (`keyUp` is
//!   outside the default system event mask) unless the program listens for key releases ([`Mac::hear_key_ups`]: a Carbon program
//!   with a `kEventRawKeyUp` handler), then `keyUp`. The message is the character code (ASCII) with, in the key-code byte, the
//!   virtual key that types it on the installed layout ([`crate::keyboard::virtual_key`]; the host's key code is not a Mac one):
//!   Storm (Warcraft III) turns that key code into a Windows virtual key through `KeyTranslate`, so a 0 there made every key 'A'.
//!   A key without a character, or one outside ASCII, is refused by name. Return/Enter in a dialog is the default item's hit.
use crate::events::{Delivered, AUTO_KEY, KEY_DOWN, KEY_UP, MOUSE_DOWN, MOUSE_UP};
use crate::lowmem::{MB_STATE, MOUSE, RAW_MOUSE};
use crate::machine::Mac;
use crate::pascal::Rect;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::{GuestUi, Modifiers, PointerButton, PointerPhase, UiObjectRef};

#[derive(Default)]
pub struct State { held: Option<u32>, key_ups: bool }

/// Mac `EventRecord.modifiers` bits: cmdKey, shiftKey, optionKey, controlKey.
const CMD: u16 = 0x0100;
const SHIFT: u16 = 0x0200;
const OPTION: u16 = 0x0800;
const CONTROL: u16 = 0x1000;

fn mac_modifiers(m: Modifiers) -> u16 {
    [(Modifiers::META, CMD), (Modifiers::SHIFT, SHIFT), (Modifiers::ALT, OPTION), (Modifiers::CTRL, CONTROL)]
        .iter().filter(|(b, _)| m.has(*b)).fold(0, |a, (_, v)| a | v)
}

fn clamp16(v: i32) -> i16 { v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 }

impl<U: GuestUi> Mac<U> {
    /// The guest window (and its content rectangle) a native window ref belongs to.
    pub fn guest_window(&self, native: UiObjectRef) -> Option<(u32, Rect)> {
        self.windows.iter().find(|(_, w)| w.native == Some(native)).map(|(&p, w)| (p, w.rect))
    }

    /// The mouse is at global `(v, h)`; `what` 0 = just moved, else the button event to queue. Updates the low-memory
    /// `Mouse`, `RawMouse` and `MBState` (bit 7 set = button up) a polling program reads.
    pub(crate) fn mouse_event(&mut self, what: u16, v: i16, h: i16, modifiers: u16) -> Result<(), Refusal> {
        let e = Refusal::mem("low-memory mouse");
        for at in [RAW_MOUSE, MOUSE] { self.mem.write_u16(at, v as u16).and_then(|_| self.mem.write_u16(at + 2, h as u16)).map_err(&e)?; }
        match what {
            MOUSE_DOWN => self.mem.write_u8(MB_STATE, 0).map_err(&e)?,
            MOUSE_UP => self.mem.write_u8(MB_STATE, 0x80).map_err(&e)?,
            _ => {}
        }
        if what != 0 { self.post_with(what, 0, v, h, modifiers); }
        Ok(())
    }

    pub(crate) fn deliver_pointer(&mut self, win: UiObjectRef, x: i32, y: i32, button: PointerButton, phase: PointerPhase, _clicks: u8) -> Result<Delivered, Refusal> {
        let Some((_, r)) = self.guest_window(win) else { return Ok(Delivered::NotForAGuestWindow) };
        let (v, h) = (clamp16(y + i32::from(r.top)), clamp16(x + i32::from(r.left)));
        let (what, modifiers) = match (button, phase) {
            (_, PointerPhase::Move) => (0, 0),
            (PointerButton::Primary, PointerPhase::Down) => (MOUSE_DOWN, 0),
            (PointerButton::Primary, PointerPhase::Up) => (MOUSE_UP, 0),
            (PointerButton::Secondary, PointerPhase::Down) => (MOUSE_DOWN, CONTROL),
            (PointerButton::Secondary, PointerPhase::Up) => (MOUSE_UP, CONTROL),
            (b, p) => return Err(Refusal(format!("Pointer: {b:?} {p:?} has no classic Mac event (a one-button mouse; the secondary button is a control-click)"))),
        };
        if what != 0 { self.tr.supersede(); } // a button event ends any native gesture still waiting for its TrackControl
        self.mouse_event(what, v, h, modifiers)?;
        Ok(if what == 0 { Delivered::Ignored } else { Delivered::Queued })
    }

    pub(crate) fn deliver_key(&mut self, win: UiObjectRef, code: u32, text: &str, mods: Modifiers, down: bool) -> Result<Delivered, Refusal> {
        let Some((ptr, _)) = self.guest_window(win) else { return Ok(Delivered::NotForAGuestWindow) };
        if !down {
            let was_held = self.tr.input.held == Some(code);
            if was_held { self.tr.input.held = None; }
            // a release is an event only to a program that listens for one (and never inside a dialog: the Dialog Manager's keys)
            if !self.tr.input.key_ups || self.tr.dlg.dialogs.contains_key(&ptr) { return Ok(Delivered::Ignored); }
            let Some(ch) = text.chars().next().filter(|c| (*c as u32) < 0x80) else { return Ok(Delivered::Ignored) };
            let ch = if matches!(ch, '\n' | '\r') { 0x0D } else { ch as u8 };
            let (v, h) = (self.mem.read_i16(MOUSE).unwrap_or(0), self.mem.read_i16(MOUSE + 2).unwrap_or(0));
            self.post_with(KEY_UP, key_message(ch), v, h, mac_modifiers(mods));
            return Ok(Delivered::Queued);
        }
        let ch = match text.chars().next() {
            None => return Err(Refusal(format!("Key: host code {code} has no character; keys without one are not mapped to classic Mac key codes"))),
            Some('\n' | '\r') => 0x0D,
            Some(c) if (c as u32) < 0x80 => c as u8,
            Some(c) => return Err(Refusal(format!("Key: U+{:04X} is outside ASCII; Mac Roman input is not mapped", c as u32))),
        };
        self.tr.supersede();
        if self.tr.dlg.dialogs.contains_key(&ptr) {
            if matches!(ch, 0x0D | 0x03) {
                let item = self.default_item(ptr)?;
                self.dialog_hit(ptr, item);
                return Ok(Delivered::Queued);
            }
            // a dialog's keys are the Dialog Manager's: they never reach the program's own event loop
            return Ok(if self.dialog_key(ptr, ch, mods.has(Modifiers::META))? { Delivered::Queued } else { Delivered::Ignored });
        }
        let what = if self.tr.input.held == Some(code) { AUTO_KEY } else { KEY_DOWN };
        self.tr.input.held = Some(code);
        let (v, h) = (self.mem.read_i16(MOUSE).unwrap_or(0), self.mem.read_i16(MOUSE + 2).unwrap_or(0));
        self.post_with(what, key_message(ch), v, h, mac_modifiers(mods));
        Ok(Delivered::Queued)
    }

    /// From now on a key release is a `keyUp` event (the program listens for releases: a Carbon `kEventRawKeyUp` handler). Without it a
    /// game that tracks held keys from the raw key events (Storm's key state, Warcraft III's camera keys) never sees a key let go.
    pub fn hear_key_ups(&mut self) { self.tr.input.key_ups = true; }
}

/// A key event's message: the character code, and the virtual key that types it on the installed layout in the key-code byte (0 when
/// no key on the layout types it).
fn key_message(ch: u8) -> u32 {
    let key = crate::keyboard::virtual_key(ch).map_or(0, |(k, _)| k);
    u32::from(key) << 8 | u32::from(ch)
}
