//! The installed keyboard layout: ONE `'KCHR'` the engine owns, British (the CoralOS default region, smRegionCode = verBritain in
//! [`crate::script`]), for both CPUs. It answers two questions from the same bytes, so they can never disagree:
//! * a program's `KeyTranslate(kchr, keycode, &state)` (the PowerPC engine serves the address as smKCHRCache, 38) - Storm, the library
//!   Warcraft III is built on, turns every key's virtual key code into a Windows virtual key through it;
//! * which key types a character the host sends ([`virtual_key`]): the key-code byte of a `keyDown` message (`message >> 8`) and a Carbon
//!   raw key event's `kEventParamKeyCode`. The host delivers characters, not key positions, so this is the key that types the character
//!   on the installed layout (the plain table first, then Shift, then Option).
//!
//! Format (Executor's `C_KeyTranslate` reading, src/include/rsys/keyboard.h, MIT; the same layout `KeyTranslate` in the PowerPC engine
//! walks): version u16, a 256-byte modifier table (index = `EventRecord.modifiers >> 8`) naming a character table, the table count u16,
//! 128-byte character tables by virtual key, then the dead-key record count u16 (0 here: no dead keys).
//! Virtual key codes are the Apple Desktop Bus ones (Inside Macintosh: Macintosh Toolbox Essentials, figure 2-10; Events.h key codes).
//! The British layout differs from the US one by Shift-3 = pound (Mac Roman 0xA3) with Option-3 = `#`, and the ISO key (0x0A) is section /
//! plus-minus (0xA4 / 0xB1). Option-layer characters other than `#` are not tabled (0 = no character), and there are no dead keys.
//! CHECKED 2026-11-02 against the Mac OS 9.2.1 System file's own `'KCHR'` 2 ("British", the owner's install CD, read as an oracle and not
//! copied): for every key tabled here its plain, Shift, Caps Lock and Control characters are the same; it also has the full Option layer,
//! five dead keys and arrow characters on keypad keys 0x42/0x46/0x48/0x4D, which this table leaves out.
use alloc::vec::Vec;

/// The character tables, in modifier-table order.
const PLAIN: u8 = 0;
const SHIFT: u8 = 1;
const CAPS: u8 = 2;
const OPTION: u8 = 3;
const CONTROL: u8 = 4;
const TABLES: usize = 5;

/// `EventRecord.modifiers` bits a character's key needs: shiftKey, optionKey.
pub const SHIFT_KEY: u16 = 0x0200;
pub const OPTION_KEY: u16 = 0x0800;

/// (virtual key, plain, shifted) for every key whose character is the same in every table but Option and Control.
const KEYS: &[(u8, u8, u8)] = &[
    (0x00, b'a', b'A'), (0x01, b's', b'S'), (0x02, b'd', b'D'), (0x03, b'f', b'F'), (0x04, b'h', b'H'), (0x05, b'g', b'G'),
    (0x06, b'z', b'Z'), (0x07, b'x', b'X'), (0x08, b'c', b'C'), (0x09, b'v', b'V'), (0x0A, 0xA4, 0xB1), (0x0B, b'b', b'B'),
    (0x0C, b'q', b'Q'), (0x0D, b'w', b'W'), (0x0E, b'e', b'E'), (0x0F, b'r', b'R'), (0x10, b'y', b'Y'), (0x11, b't', b'T'),
    (0x12, b'1', b'!'), (0x13, b'2', b'@'), (0x14, b'3', 0xA3), (0x15, b'4', b'$'), (0x16, b'6', b'^'), (0x17, b'5', b'%'),
    (0x18, b'=', b'+'), (0x19, b'9', b'('), (0x1A, b'7', b'&'), (0x1B, b'-', b'_'), (0x1C, b'8', b'*'), (0x1D, b'0', b')'),
    (0x1E, b']', b'}'), (0x1F, b'o', b'O'), (0x20, b'u', b'U'), (0x21, b'[', b'{'), (0x22, b'i', b'I'), (0x23, b'p', b'P'),
    (0x25, b'l', b'L'), (0x26, b'j', b'J'), (0x27, b'\'', b'"'), (0x28, b'k', b'K'), (0x29, b';', b':'), (0x2A, b'\\', b'|'),
    (0x2B, b',', b'<'), (0x2C, b'/', b'?'), (0x2D, b'n', b'N'), (0x2E, b'm', b'M'), (0x2F, b'.', b'>'), (0x32, b'`', b'~'),
    (0x31, b' ', b' '),
];

/// Keys that type a control character (the same in every table): Return, Tab, Delete, Enter, Escape, the keypad, the navigation keys.
const CONTROL_KEYS: &[(u8, u8)] = &[
    (0x24, 0x0D), (0x30, 0x09), (0x33, 0x08), (0x34, 0x03), (0x35, 0x1B), (0x41, b'.'), (0x43, b'*'), (0x45, b'+'), (0x47, 0x1B),
    (0x4B, b'/'), (0x4C, 0x03), (0x4E, b'-'), (0x51, b'='), (0x52, b'0'), (0x53, b'1'), (0x54, b'2'), (0x55, b'3'), (0x56, b'4'),
    (0x57, b'5'), (0x58, b'6'), (0x59, b'7'), (0x5B, b'8'), (0x5C, b'9'),
    // Help, Home, Page Up, forward Delete, End, Page Down, and the arrows left / right / down / up
    (0x72, 0x05), (0x73, 0x01), (0x74, 0x0B), (0x75, 0x7F), (0x77, 0x04), (0x79, 0x0C), (0x7B, 0x1C), (0x7C, 0x1D), (0x7D, 0x1F), (0x7E, 0x1E),
];

/// F1..F15, in order: each types the function-key character 0x10.
const FUNCTION_KEYS: [u8; 15] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71];

/// The table a modifier byte (`modifiers >> 8`: bit 0 cmd, 1 shift, 2 caps lock, 3 option, 4 control, 5..7 the right-hand shift,
/// option, control) selects. Command selects no table of its own (a Command-key character is the plain or shifted one).
fn table_for(m: u8) -> u8 {
    if m & 0x90 != 0 { CONTROL } else if m & 0x48 != 0 { OPTION } else if m & 0x22 != 0 { SHIFT } else if m & 0x04 != 0 { CAPS } else { PLAIN }
}

/// The layout's character for virtual key `k` in table `t` (0 = none).
fn char_in(t: u8, k: u8) -> u8 {
    if let Some(&(_, c)) = CONTROL_KEYS.iter().find(|e| e.0 == k) { return c; }
    if FUNCTION_KEYS.contains(&k) { return 0x10; }
    let Some(&(_, plain, shifted)) = KEYS.iter().find(|e| e.0 == k) else { return 0 };
    match t {
        PLAIN => plain,
        SHIFT => shifted,
        CAPS => if k == 0x0A { shifted } else { plain.to_ascii_uppercase() },
        OPTION => if k == 0x14 { b'#' } else { 0 },
        _ => match plain { b'a'..=b'z' => plain & 0x1F, b'[' => 0x1B, b'\\' => 0x1C, b']' => 0x1D, b'-' => 0x1F, b'\'' => b'"', b';' => b':', 0xA4 => b'`', _ => plain },
    }
}

/// The installed `'KCHR'` resource's bytes.
pub fn kchr() -> Vec<u8> {
    let mut b = Vec::with_capacity(2 + 256 + 2 + 128 * TABLES + 2);
    b.extend(1u16.to_be_bytes()); // version
    b.extend((0..=255u8).map(table_for));
    b.extend((TABLES as u16).to_be_bytes());
    for t in 0..TABLES as u8 { b.extend((0..128u8).map(|k| char_in(t, k))); }
    b.extend(0u16.to_be_bytes()); // no dead keys
    b
}

/// The virtual key that types Mac Roman character `ch` on the installed layout and the modifiers it needs: the plain table first, then
/// Shift, then Option. None = no key types it (a character the host can send but this layout cannot).
pub fn virtual_key(ch: u8) -> Option<(u8, u16)> {
    [(PLAIN, 0), (SHIFT, SHIFT_KEY), (OPTION, OPTION_KEY)].iter()
        .find_map(|&(t, mods)| (0..128u8).find(|&k| char_in(t, k) == ch && ch != 0).map(|k| (k, mods)))
}

/// The Mac key for a host key that types nothing ([`heirloom_ui_abi::host_key`]): `(virtual key, the character it types)`. These keys type
/// the same character in every table ([`CONTROL_KEYS`]: the arrows 0x1C..0x1F, Home 0x01, End 0x04, Page Up / Down 0x0B / 0x0C, forward
/// Delete 0x7F, every function key 0x10). Insert is the Help key (0x72), which sits where a PC keyboard has Insert on Apple's extended
/// keyboards. `None` for a code that is no key a Mac keyboard has.
pub fn host_key(code: u32) -> Option<(u8, u8)> {
    use heirloom_ui_abi::host_key as h;
    let k = match code {
        h::UP => 0x7E,
        h::DOWN => 0x7D,
        h::LEFT => 0x7B,
        h::RIGHT => 0x7C,
        h::HOME => 0x73,
        h::END => 0x77,
        h::PAGE_UP => 0x74,
        h::PAGE_DOWN => 0x79,
        h::DELETE => 0x75,
        h::INSERT => 0x72,
        c => FUNCTION_KEYS[usize::from(h::function_number(c)?) - 1],
    };
    Some((k, char_in(PLAIN, k)))
}

/// The Mac Roman byte of a character the installed layout types: ASCII, and the three non-ASCII characters of the British table (the
/// pound sign on Shift-3, section and plus-minus on the ISO key). `None` for anything else.
pub fn mac_roman(c: char) -> Option<u8> {
    match c {
        c if (c as u32) < 0x80 => Some(c as u8),
        '\u{A3}' => Some(0xA3),
        '\u{A7}' => Some(0xA4),
        '\u{B1}' => Some(0xB1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout's own lookup, the way KeyTranslate reads the bytes (modifier byte -> table -> character).
    fn translate(b: &[u8], mods: u16, k: u8) -> u8 {
        let t = usize::from(b[2 + usize::from(mods >> 8)]);
        b[260 + 128 * t + usize::from(k)]
    }

    #[test]
    fn every_character_a_key_types_maps_back_to_that_key_through_the_layout_bytes() {
        let b = kchr();
        assert_eq!(b.len(), 2 + 256 + 2 + 128 * TABLES + 2);
        for ch in (0x01..=0x7Eu8).chain([0xA3, 0xA4, 0xB1]) {
            if let Some((k, mods)) = virtual_key(ch) { assert_eq!(translate(&b, mods, k), ch, "char {ch:#x} key {k:#x}"); }
        }
        assert_eq!(virtual_key(b'p'), Some((0x23, 0)));
        assert_eq!(virtual_key(b'P'), Some((0x23, SHIFT_KEY)));
        assert_eq!(virtual_key(0x0D), Some((0x24, 0)), "Return, not the keypad Enter");
        assert_eq!(virtual_key(0x08), Some((0x33, 0)), "Delete (backspace)");
        assert_eq!(virtual_key(0x1B), Some((0x35, 0)), "Escape, not the keypad Clear");
        assert_eq!(virtual_key(b'.'), Some((0x2F, 0)), "the main period, not the keypad's");
        assert_eq!(virtual_key(0xA3), Some((0x14, SHIFT_KEY)), "British: Shift-3 is the pound sign");
        assert_eq!(virtual_key(b'#'), Some((0x14, OPTION_KEY)), "British: Option-3 is #");
        // every printable ASCII character has a key
        for ch in 0x20..=0x7Eu8 { assert!(virtual_key(ch).is_some(), "{:?} has no key", ch as char); }
        // NEGATIVE CONTROL: a character no key types
        assert_eq!(virtual_key(0xFF), None);
        assert_eq!(virtual_key(0), None);
        // caps lock uppercases letters only; control turns a letter into its control character; command selects no table of its own
        assert_eq!(translate(&b, 0x0400, 0x00), b'A');
        assert_eq!(translate(&b, 0x0400, 0x12), b'1');
        assert_eq!(translate(&b, 0x1000, 0x08), 0x03);
        assert_eq!(translate(&b, 0x0100, 0x00), b'a');
        assert_eq!(translate(&b, 0x0300, 0x00), b'A');
    }
}
