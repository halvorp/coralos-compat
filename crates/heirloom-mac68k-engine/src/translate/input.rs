//! Plain input for content (ruling 17, piece 8): the pointer and the keyboard, as the events a classic Mac program already
//! handles, so a content plane is fully usable before it has any binding. Coordinates: the host sends window coordinates
//! (0,0 = the window's content top-left); the Event Manager reports GLOBAL ones, so the window's content origin is added.
//!
//! * `Pointer` primary `Down`/`Up` -> `mouseDown`/`mouseUp`; the secondary button is a control-click (`controlKey` in the
//!   modifiers, the Mac convention for the contextual click); the middle button has no classic meaning: refused by name.
//!   Motion produces no event (a Mac program polls `GetMouse`/`Button`): it only updates the low-memory `Mouse` point.
//! * `Key` down -> `keyDown`, the same key again before its release -> `autoKey`; release produces nothing (`keyUp` is
//!   outside the default system event mask) unless the program listens for key releases ([`Mac::hear_key_ups`]: a Carbon program
//!   with a `kEventRawKeyUp` handler), then `keyUp`. The message is the character (Mac Roman) with, in the key-code byte, the Mac
//!   virtual key: for a character, the key that types it on the installed layout ([`crate::keyboard::virtual_key`]; with Control held
//!   a letter's control character is that letter's key); for a key that types nothing (empty text: the arrows, Home, End, Page Up /
//!   Down, forward Delete, the F-keys - [`heirloom_ui_abi::host_key`]) the Mac key itself and the character it types on a Mac
//!   ([`crate::keyboard::host_key`]). Storm (Warcraft III) turns the key code into a Windows virtual key through `KeyTranslate`, so a 0
//!   there made every key 'A', and before h86 an arrow arrived as the host's C0 byte taken for a character (Up = the F-key character).
//!   The modifiers are the event's `modifiers` (Shift, Option = Alt, Control, Command). A code that is no Mac key, or a character the
//!   layout does not type, is refused by name. Return/Enter in a dialog is the default item's hit.
//! * every key down and up also keeps the KEY MAP `GetKeys` reads ([`Mac::key_map`]): the keys held, and the modifier keys held with them.
use crate::events::{Delivered, AUTO_KEY, KEY_DOWN, KEY_UP, MOUSE_DOWN, MOUSE_UP};
use crate::lowmem::{MB_STATE, MOUSE, RAW_MOUSE};
use crate::machine::Mac;
use crate::pascal::Rect;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use heirloom_ui_abi::{GuestUi, Modifiers, PointerButton, PointerPhase, UiObjectRef};

#[derive(Default)]
pub struct State {
    /// The Mac keys held (virtual key -> the `EventRecord.modifiers` they went down with).
    keys: BTreeMap<u8, u16>,
    key_ups: bool,
}

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
        let modifiers = mac_modifiers(mods);
        let (v, h) = (self.mem.read_i16(MOUSE).unwrap_or(0), self.mem.read_i16(MOUSE + 2).unwrap_or(0));
        if !down {
            // a release of a key the Mac has no key for was refused at its press: nothing to let go
            let Ok((vk, ch)) = mac_key(code, text, mods) else { return Ok(Delivered::Ignored) };
            self.tr.input.keys.remove(&vk);
            // a release is an event only to a program that listens for one (and never inside a dialog: the Dialog Manager's keys)
            if !self.tr.input.key_ups || self.tr.dlg.dialogs.contains_key(&ptr) { return Ok(Delivered::Ignored); }
            self.post_with(KEY_UP, u32::from(vk) << 8 | u32::from(ch), v, h, modifiers);
            return Ok(Delivered::Queued);
        }
        let (vk, ch) = mac_key(code, text, mods)?;
        let repeat = self.tr.input.keys.insert(vk, modifiers).is_some();
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
        self.post_with(if repeat { AUTO_KEY } else { KEY_DOWN }, u32::from(vk) << 8 | u32::from(ch), v, h, modifiers);
        Ok(Delivered::Queued)
    }

    /// The 128 key-down bits `GetKeys` returns (key k = bit k % 8 of byte k / 8): every key held, and the modifier keys (Command 0x37,
    /// Shift 0x38, Option 0x3A, Control 0x3B) held with any of them. The host reports modifiers only with a key, so a modifier pressed
    /// alone is not seen.
    pub fn key_map(&self) -> [u8; 16] {
        let mut map = [0u8; 16];
        let mut set = |k: u8| map[usize::from(k >> 3)] |= 1 << (k & 7);
        for (&vk, &m) in &self.tr.input.keys {
            set(vk);
            for (bit, k) in [(CMD, 0x37), (SHIFT, 0x38), (OPTION, 0x3A), (CONTROL, 0x3B)] {
                if m & bit != 0 { set(k); }
            }
        }
        map
    }

    /// From now on a key release is a `keyUp` event (the program listens for releases: a Carbon `kEventRawKeyUp` handler). Without it a
    /// game that tracks held keys from the raw key events (Storm's key state, Warcraft III's camera keys) never sees a key let go.
    pub fn hear_key_ups(&mut self) { self.tr.input.key_ups = true; }
}

/// The Mac key a host key event is: `(virtual key, Mac Roman character)`. A key with no text is [`crate::keyboard::host_key`]; a character
/// is the key that types it on the installed layout - with Control held, a letter's control character is that letter's key (Ctrl-A types
/// 0x01 on the A key, not Home), except Backspace, Tab and Return, which are their own keys. 0 = no key on the layout types it.
fn mac_key(code: u32, text: &str, mods: Modifiers) -> Result<(u8, u8), Refusal> {
    let Some(c) = text.chars().next() else {
        return crate::keyboard::host_key(code).ok_or_else(|| Refusal(format!("Key: host code {code:#x} types nothing and is no key the Mac keyboard has")));
    };
    let ch = match c {
        '\n' | '\r' => 0x0D,
        c => crate::keyboard::mac_roman(c).ok_or_else(|| Refusal(format!("Key: U+{:04X} is outside ASCII and the British layout's keys; Mac Roman input is not mapped", c as u32)))?,
    };
    let letter = mods.has(Modifiers::CTRL) && (0x01..=0x1A).contains(&ch) && !matches!(ch, 0x08 | 0x09 | 0x0D);
    let vk = crate::keyboard::virtual_key(if letter { ch | 0x60 } else { ch }).map_or(0, |(k, _)| k);
    Ok((vk, ch))
}
