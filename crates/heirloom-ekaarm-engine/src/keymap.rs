//! Host key -> Symbian scan and key codes. A host key event carries a CoralOS Unicode scalar or C0 key-wire byte and the text it types;
//! the window server's `EEventKey` carries a `TStdScanCode` and `TKeyCode` (e32keys.h:140+, :340+). A key with no key code gets only
//! its key-down and key-up events; text of more than one UTF-16 unit is refused, not cut.
use alloc::format;
use alloc::string::String;

// (host_code, TStdScanCode, TKeyCode) — C0 key-wire bytes from coral-keymap-abi lib.rs:126-188,
// named keys from coral-wm-abi key_event.rs. Symbian values from e32keys.h.
const HOST_KEYS: &[(u32, u32, u32)] = &[
    (0x01, 0x08, 0xf802), // WIRE_HOME   -> EStdKeyHome 0x08 (e32keys.h:152),  EKeyHome 0xf802 (e32keys.h:356)
    (0x02, 0x0e, 0xf807), // WIRE_LEFT   -> EStdKeyLeftArrow 0x0e (e32keys.h:158), EKeyLeftArrow 0xf807 (e32keys.h:361)
    (0x04, 0x09, 0xf803), // WIRE_END    -> EStdKeyEnd 0x09 (e32keys.h:153),  EKeyEnd 0xf803 (e32keys.h:357)
    (0x06, 0x0f, 0xf808), // WIRE_RIGHT  -> EStdKeyRightArrow 0x0f (e32keys.h:159), EKeyRightArrow 0xf808 (e32keys.h:362)
    (0x08, 0x01, 0x0008), // Backspace   -> EStdKeyBackspace 0x01 (e32keys.h:145), EKeyBackspace 0x0008 (e32keys.h:345)
    (0x09, 0x02, 0x0009), // Tab         -> EStdKeyTab 0x02 (e32keys.h:146),  EKeyTab 0x0009 (e32keys.h:346)
    (0x0D, 0x03, 0x000d), // Enter       -> EStdKeyEnter 0x03 (e32keys.h:147), EKeyEnter 0x000d (e32keys.h:347)
    (0x0E, 0x11, 0xf80a), // WIRE_DOWN   -> EStdKeyDownArrow 0x11 (e32keys.h:161), EKeyDownArrow 0xf80a (e32keys.h:364)
    (0x10, 0x10, 0xf809), // WIRE_UP     -> EStdKeyUpArrow 0x10 (e32keys.h:160), EKeyUpArrow 0xf809 (e32keys.h:363)
    (0x1B, 0x04, 0x001b), // WIRE_ESCAPE -> EStdKeyEscape 0x04 (e32keys.h:148), EKeyEscape 0x001b (e32keys.h:348)
    (0x1C, 0x0a, 0xf804), // WIRE_PAGE_UP   -> EStdKeyPageUp 0x0a (e32keys.h:154), EKeyPageUp 0xf804 (e32keys.h:358)
    (0x1D, 0x0b, 0xf805), // WIRE_PAGE_DOWN -> EStdKeyPageDown 0x0b (e32keys.h:155), EKeyPageDown 0xf805 (e32keys.h:359)
    (0xF700, 0x0d, 0x007f), // DELETE  -> EStdKeyDelete 0x0d (e32keys.h:157), EKeyDelete 0x007f (e32keys.h:353)
    (0xF701, 0x0c, 0xf806), // INSERT  -> EStdKeyInsert 0x0c (e32keys.h:156), EKeyInsert 0xf806 (e32keys.h:360)
    // DELETE WHEN: a corpus app ignores Enter and wants EKeyDevice3.
    // EStdKeyDevice3 0xa7 (e32keys.h:167), EKeyDevice3 0xf845 (e32keys.h:373) — no host key maps to it yet.
];

// EStdKeyF1 (e32keys.h) = 0x60; F1..F12 = U+F710..U+F71B (coral-wm-abi key_event.rs).
const F_KEY_BASE: u32 = 0xF710;
const F_SCAN_BASE: u32 = 0x60; // EStdKeyF1 (e32keys.h)

/// Translate a host key code (CoralOS Unicode scalar or C0 key-wire byte) into
/// Symbian (TStdScanCode, Option<TKeyCode>). For non-printing keys, `text` is
/// ignored (the host may wrongly carry the C0 char). For printable keys, `text`
/// must be exactly one UTF-16 unit; the unit becomes the TKeyCode and the
/// TStdScanCode is the upper-case ASCII letter/digit for [A-Za-z0-9], else
/// EStdKeyNull 0x00. Empty text with no table hit yields (0x00, None) —
/// modifier or unknown key, key-down/up only.
pub fn host_key(code: u32, text: &str) -> Result<(u32, Option<u32>), String> {
    // 1. Non-printing host key — text ignored.
    if let Some(&(_, scan, key)) = HOST_KEYS.iter().find(|&&(h, _, _)| h == code) {
        return Ok((scan, Some(key)));
    }
    // 2. F1..F12 — no TKeyCode.
    if (F_KEY_BASE..=F_KEY_BASE + 11).contains(&code) {
        return Ok((F_SCAN_BASE + (code - F_KEY_BASE), None));
    }
    // 3. Printable key — text must be exactly one UTF-16 unit.
    let units = text.encode_utf16().count();
    match units {
        0 => Ok((0x00, None)), // modifier / unknown
        1 => {
            let unit = text.encode_utf16().next().unwrap() as u32;
            let std_scan = match unit {
                0x41..=0x5A => unit,                    // A-Z: scan = uppercase ASCII
                0x61..=0x7A => unit - 0x20,             // a-z: scan = uppercase ASCII
                0x30..=0x39 => unit,                    // 0-9: scan = digit ASCII
                _ => 0x00,                              // EStdKeyNull
            };
            Ok((std_scan, Some(unit)))
        }
        _ => Err(format!("text has more than one UTF-16 unit: {:?}", text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_empty_text()   { assert_eq!(host_key(0x0E, "").unwrap(), (0x11, Some(0xf80a))); }
    #[test]
    fn down_c0_text()      { assert_eq!(host_key(0x0E, "\u{0E}").unwrap(), (0x11, Some(0xf80a))); }
    #[test]
    fn up_arrow()          { assert_eq!(host_key(0x10, "").unwrap(), (0x10, Some(0xf809))); }
    #[test]
    fn lowercase_a()       { assert_eq!(host_key(0x61, "a").unwrap(), (0x41, Some(0x61))); }
    #[test]
    fn uppercase_a()       { assert_eq!(host_key(0x41, "A").unwrap(), (0x41, Some(0x41))); }
    #[test]
    fn enter_key()         { assert_eq!(host_key(0x0D, "\r").unwrap(), (0x03, Some(0x0d))); }
    #[test]
    fn f5_key()            { assert_eq!(host_key(0xF714, "").unwrap(), (0x64, None)); }
    #[test]
    fn multi_unit_refused(){ assert!(host_key(0x61, "ab").is_err()); }
    #[test]
    fn unmapped_wire()     { assert_eq!(host_key(0x11, "").unwrap(), (0x00, None)); }
}
