//! The host's key codes (`GuestUiEvent::Key::code`) for the keys that type NOTHING - the event's `text` is empty - named once for
//! every engine. The CoralOS host (Heirloom.app) sends the scalar its compositor delivered: the editing keys are coralos
//! coral-keymap-abi's C0 wire bytes (WIRE_HOME .. WIRE_PAGE_DOWN), Delete / Insert / F1-F12 are coral-wm-abi `key_event`'s named
//! private-use scalars. A key that types a character (Return, Tab, Backspace, Escape, a letter) is its character in `text`, and
//! `code` is that character too. The host has no Command key and sends no key repeat: a held key is a press, then a release.
//!
//! An engine maps these to its platform's own key codes (the Mac's ADB virtual keys, Symbian's TStdScanCode); a code that is in
//! none of them is refused by name.

/// Home (coral-keymap-abi WIRE_HOME, SOH).
pub const HOME: u32 = 0x01;
/// Left arrow (WIRE_LEFT, STX).
pub const LEFT: u32 = 0x02;
/// End (WIRE_END, EOT).
pub const END: u32 = 0x04;
/// Right arrow (WIRE_RIGHT, ACK).
pub const RIGHT: u32 = 0x06;
/// Down arrow (WIRE_DOWN, SO).
pub const DOWN: u32 = 0x0E;
/// Up arrow (WIRE_UP, DLE).
pub const UP: u32 = 0x10;
/// Page Up (WIRE_PAGE_UP, FS).
pub const PAGE_UP: u32 = 0x1C;
/// Page Down (WIRE_PAGE_DOWN, GS).
pub const PAGE_DOWN: u32 = 0x1D;
/// Forward Delete (coral-wm-abi key_event::DELETE).
pub const DELETE: u32 = 0xF700;
/// Insert (key_event::INSERT).
pub const INSERT: u32 = 0xF701;
/// F1; F2 is F1 + 1 ... F12 = F1 + 11 (key_event::F1..F12). The CoralOS compositor keeps F10-F12 as its own hotkeys.
pub const F1: u32 = 0xF710;

/// How long a host whose key wire carries no RELEASE (CoralOS's: presses only) holds a press before it sends the key's up (ms): a typing
/// tap, long enough that a guest reading held keys sees the key down across a frame. Heirloom.app's hold, and the harness's.
pub const HOLD_MS: u64 = 120;

/// The function-key number (1..=12) of a host code, or `None`.
pub const fn function_number(code: u32) -> Option<u8> {
    if code >= F1 && code < F1 + 12 { Some((code - F1 + 1) as u8) } else { None }
}
